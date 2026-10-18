//! October's AI, for Ask Lantern and Point & Ask (Swift `OctoberAI.swift`).
//!
//! Questions go to Lantern's endpoint on October (`POST /api/lantern/ask`) with the October
//! account's access token: October picks the model by plan, bills paid plans' credit, and the
//! answer streams back as `delta` events and a final `done` (which may carry a task to start and
//! a note about the account). Where that endpoint isn't live (404/405), the answer comes from
//! October's inference gateway (`/v1`, OpenAI-compatible) instead.
//!
//! What's sent: the question, earlier turns, the context lines (the app and window for Point &
//! Ask, a summary of the agents for Ask Lantern) and Point & Ask's screenshot. Nothing else.
use crate::model::Platform;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ASK: &str = "https://www.october.dev/api/lantern/ask";
const GATEWAY: &str = "https://www.october.dev/v1";

#[derive(Clone, Debug, Default)]
pub struct Request {
    /// Earlier questions and answers in this conversation.
    pub turns: Vec<(String, String)>,
    pub question: String,
    /// The screenshot, as JPEG.
    pub image: Option<Vec<u8>>,
    /// The app, window, page and selection, or the agent summary, as lines of text.
    pub context: String,
    /// "chat" for Ask Lantern (its own prompt on the server); `None` for Point & Ask.
    pub mode: Option<String>,
    pub platform: Platform,
}

/// A piece of the answer, or something to show with it.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Text(String),
    /// About the account ("3 free questions left today.").
    Note(String),
    /// Work the assistant decided to start (chat mode); agent and project only when named.
    Task {
        prompt: String,
        agent: Option<String>,
        project: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    SignedOut,
    Plan(String),
    Limited(String),
    Unavailable(String),
    NotDeployed,
    /// 400: the server couldn't read the request.
    Invalid(String),
    /// The person (or a newer question) cancelled it.
    Cancelled,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Self::SignedOut => "Sign in to October to ask.",
            Self::NotDeployed => "October's AI isn't available right now.",
            Self::Cancelled => "Cancelled.",
            Self::Plan(m) | Self::Limited(m) | Self::Unavailable(m) | Self::Invalid(m) => m,
        })
    }
}

/// Point & Ask's instructions for the gateway (Lantern's endpoint has its own), naming this computer.
pub fn system_prompt(platform: Platform) -> String {
    let computer = match platform {
        Platform::Windows => "Windows PC",
        Platform::Linux => "Linux computer",
    };
    format!(
        "You are October Lantern's Point & Ask on the user's {computer}. The user selected part of their screen (the attached \
         screenshot) and asked about it. Answer directly and briefly: a short paragraph or a few bullets, plain Markdown, no \
         headings. If the question is about code or an error, say what is wrong and how to fix it. If you can't see enough to \
         answer, say what you'd need."
    )
}

/// Modes this server turned down (it answered the plain way): asked plainly from then on, until
/// Lantern restarts, so later questions don't pay for a refused request first.
static UNSUPPORTED_MODES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Streams the answer into `sink`, which returns false to stop (a cancelled question).
pub fn stream(request: &Request, token: &str, sink: &mut dyn FnMut(Piece) -> bool) -> Result<(), Failure> {
    let known = |mode: &String| !UNSUPPORTED_MODES.lock().unwrap_or_else(|e| e.into_inner()).contains(mode);
    let request = &Request { mode: request.mode.clone().filter(known), ..request.clone() };
    let first = match ask(request, token, sink) {
        // A server without the mode refuses the request as invalid: ask the plain way instead (and
        // from now on, once that works).
        Err(Failure::Invalid(_)) if request.mode.is_some() => {
            let plain = ask(&Request { mode: None, ..request.clone() }, token, sink);
            if plain.is_ok()
                && let Some(mode) = &request.mode
            {
                UNSUPPORTED_MODES.lock().unwrap_or_else(|e| e.into_inner()).push(mode.clone());
            }
            plain
        }
        other => other,
    };
    match first {
        Err(Failure::NotDeployed) => {}
        other => return other,
    }
    // Lantern's own endpoint isn't live: use the inference gateway.
    let model = pick_model(token, request.image.is_some())?;
    let with_image = request.image.is_some() && model.vision;
    match run(&gateway_body(request, &model.id, with_image), token, sink) {
        // The gateway only takes text for this model or plan: ask without the picture.
        Err(Failure::Invalid(message)) if with_image && message.to_lowercase().contains("text") => {
            run(&gateway_body(request, &model.id, false), token, sink)
        }
        other => other,
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(90))).http_status_as_error(false).build().into()
}

/// Lantern's endpoint refuses a request with any key it doesn't know (`parseAskRequest` on
/// October): `client` is exactly `{app, version}`, and `history` is at most 10 messages, each a
/// non-empty text of at most 8000 characters. The Mac sends 20 messages, which the server
/// refuses after five questions; here the oldest whole turns are left out instead. Its lengths
/// are JavaScript's (UTF-16 code units), so an emoji counts twice.
const HISTORY_MESSAGES: usize = 10;
const HISTORY_UNITS: usize = 8000;
const QUESTION_UNITS: usize = 4000;
const CONTEXT_UNITS: usize = 6000;

/// The longest start of `text` that is at most `units` UTF-16 code units, never splitting a
/// character.
fn utf16_prefix(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (at, c) in text.char_indices() {
        used += c.len_utf16();
        if used > units {
            return &text[..at];
        }
    }
    text
}

/// The JSON for Lantern's endpoint.
pub fn ask_body(request: &Request) -> Value {
    let turns: Vec<_> =
        request.turns.iter().filter(|(question, answer)| !question.trim().is_empty() && !answer.trim().is_empty()).collect();
    let content = |text: &str| utf16_prefix(text, HISTORY_UNITS).to_string();
    let history: Vec<Value> = turns[turns.len().saturating_sub(HISTORY_MESSAGES / 2)..]
        .iter()
        .flat_map(|(question, answer)| {
            [json!({"role": "user", "content": content(question)}), json!({"role": "assistant", "content": content(answer)})]
        })
        .collect();
    let mut body = json!({
        "question": utf16_prefix(&request.question, QUESTION_UNITS),
        "history": history,
        "client": {"app": "lantern", "version": env!("CARGO_PKG_VERSION")},
    });
    if !request.context.is_empty() {
        body["context"] = utf16_prefix(&request.context, CONTEXT_UNITS).into();
    }
    if let Some(mode) = &request.mode {
        body["mode"] = mode.clone().into();
    }
    if let Some(image) = &request.image {
        use base64::Engine;
        body["image"] = base64::engine::general_purpose::STANDARD.encode(image).into();
    }
    body
}

fn ask(request: &Request, token: &str, sink: &mut dyn FnMut(Piece) -> bool) -> Result<(), Failure> {
    let response = agent()
        .post(ASK)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "text/event-stream")
        .send_json(ask_body(request))
        .map_err(|error| Failure::Unavailable(format!("Couldn't reach October: {error}")))?;
    let status = response.status().as_u16();
    if status == 404 || status == 405 {
        return Err(Failure::NotDeployed);
    }
    let mut body = response.into_body().into_reader();
    if status != 200 {
        return Err(failure(status, &read_some(&mut body)));
    }
    read_ask_events(body, sink)
}

/// Lantern's endpoint: `delta` text, then `done` (with an optional task and account note).
pub fn read_ask_events(reader: impl Read, sink: &mut dyn FnMut(Piece) -> bool) -> Result<(), Failure> {
    for line in BufReader::new(reader.take(4 * 1024 * 1024)).lines() {
        let line = line.map_err(|error| Failure::Unavailable(format!("October's answer was cut off: {error}")))?;
        let Some(event) = line.strip_prefix("data:").and_then(|payload| serde_json::from_str::<Value>(payload.trim()).ok()) else {
            continue;
        };
        match event["type"].as_str() {
            Some("delta") => {
                if let Some(text) = event["text"].as_str()
                    && !sink(Piece::Text(text.into()))
                {
                    return Err(Failure::Cancelled);
                }
            }
            Some("done") => {
                if let Some(task) = event["task"].as_object()
                    && let Some(prompt) = task.get("prompt").and_then(Value::as_str).filter(|prompt| !prompt.trim().is_empty())
                {
                    let named = |key: &str| task.get(key).and_then(Value::as_str).map(str::to_string);
                    sink(Piece::Task { prompt: prompt.chars().take(4000).collect(), agent: named("agent"), project: named("project") });
                }
                if let Some(note) = note(&event) {
                    sink(Piece::Note(note));
                }
                return Ok(());
            }
            // Not sent by today's server, but never shown as an answer if it is.
            Some("error") => {
                return Err(Failure::Unavailable(event["message"].as_str().unwrap_or("October's AI stopped answering.").into()));
            }
            _ => {}
        }
    }
    Ok(())
}

/// "3 free questions left today." from a free account's `done` event (only at 10 or fewer).
pub fn note(done: &Value) -> Option<String> {
    let remaining = &done["remaining"];
    if remaining["kind"] != "free" {
        return None;
    }
    let left = remaining["questions"].as_i64().filter(|left| *left <= 10)?;
    Some(match left {
        0 => "That was today's last free question.".into(),
        1 => "1 free question left today.".into(),
        n => format!("{n} free questions left today."),
    })
}

/// October's error responses, as people should read them.
pub fn failure(status: u16, body: &[u8]) -> Failure {
    let error = serde_json::from_slice::<Value>(body).ok().map(|value| value["error"].clone()).unwrap_or_default();
    let code = error["code"].as_str().unwrap_or("");
    let message = error["message"].as_str().map(str::to_string);
    match (status, code) {
        (401, _) => Failure::SignedOut,
        (_, "plan_required") => Failure::Plan(message.unwrap_or_else(|| "This needs an October Pro or Max plan.".into())),
        (402, _) | (_, "credit_exhausted") => {
            Failure::Plan(message.unwrap_or_else(|| "You've used this period's October AI credit.".into()))
        }
        (_, "free_limit") => {
            Failure::Limited(message.unwrap_or_else(|| "That's today's free questions. Upgrade to October Pro for more.".into()))
        }
        (429, _) => Failure::Limited(message.unwrap_or_else(|| "Too many questions for now. Try again in a minute.".into())),
        (400, _) => Failure::Invalid(message.unwrap_or_else(|| "October's AI couldn't read the request.".into())),
        _ => Failure::Unavailable(message.unwrap_or_else(|| format!("October's AI isn't available right now ({status}). Try again soon."))),
    }
}

fn read_some(reader: &mut impl Read) -> Vec<u8> {
    let mut body = Vec::new();
    let _ = reader.take(64_000).read_to_end(&mut body);
    body
}

// MARK: The gateway (until Lantern's endpoint is live everywhere)

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub id: String,
    pub vision: bool,
}

static CACHED: Mutex<Option<(Model, Instant)>> = Mutex::new(None);

fn pick_model(token: &str, needs_vision: bool) -> Result<Model, Failure> {
    if let Some((model, at)) = CACHED.lock().unwrap().clone()
        && at.elapsed() < Duration::from_secs(3600)
        && (model.vision || !needs_vision)
    {
        return Ok(model);
    }
    let response = agent()
        .get(&format!("{GATEWAY}/models"))
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|error| Failure::Unavailable(format!("Couldn't reach October: {error}")))?;
    let status = response.status().as_u16();
    let body = read_some(&mut response.into_body().into_reader());
    if status != 200 {
        return Err(failure(status, &body));
    }
    let list: Value = serde_json::from_slice(&body).unwrap_or_default();
    let models: Vec<(String, Option<bool>)> = list["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| Some((m["id"].as_str()?.to_string(), m["capabilities"]["vision"].as_bool())))
        .collect();
    let chosen = choose(&models).ok_or_else(|| Failure::Unavailable("October's AI has no models available right now.".into()))?;
    *CACHED.lock().unwrap() = Some((chosen.clone(), Instant::now()));
    Ok(chosen)
}

/// Prefers models that can see, then well-known capable families, then anything.
pub fn choose(models: &[(String, Option<bool>)]) -> Option<Model> {
    const PREFERRED: [&str; 11] =
        ["gemini", "claude", "gpt-4", "gpt-5", "qwen2.5-vl", "qwen3-vl", "vl", "llama-4", "gemma-3", "mistral-small", "pixtral"];
    let rank = |id: &str| {
        let lower = id.to_lowercase();
        PREFERRED.iter().position(|family| lower.contains(family)).unwrap_or(PREFERRED.len())
    };
    let best = |list: Vec<&(String, Option<bool>)>| list.into_iter().min_by_key(|(id, _)| rank(id)).map(|(id, _)| id.clone());
    if let Some(id) = best(models.iter().filter(|(_, vision)| *vision == Some(true)).collect()) {
        return Some(Model { id, vision: true });
    }
    best(models.iter().collect()).map(|id| Model { id, vision: false })
}

/// The OpenAI-style request: instructions, then the conversation, the context with the first question.
pub fn gateway_body(request: &Request, model: &str, with_image: bool) -> Value {
    let mut messages = vec![json!({"role": "system", "content": system_prompt(request.platform)})];
    let mut first = request.turns.first().map_or(request.question.clone(), |(question, _)| question.clone());
    if !request.context.is_empty() {
        first.push_str(&format!("\n\nWhere this is on my computer:\n{}", request.context));
    }
    if !with_image && request.image.is_some() {
        first.push_str("\n\n(A screenshot was taken but this model can't see images.)");
    }
    let content = match (&request.image, with_image) {
        (Some(image), true) => {
            use base64::Engine;
            let url = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(image));
            json!([{"type": "text", "text": first}, {"type": "image_url", "image_url": {"url": url}}])
        }
        _ => json!(first),
    };
    messages.push(json!({"role": "user", "content": content}));
    for (index, (_, answer)) in request.turns.iter().enumerate() {
        messages.push(json!({"role": "assistant", "content": answer}));
        let next = request.turns.get(index + 1).map_or(&request.question, |(question, _)| question);
        messages.push(json!({"role": "user", "content": next}));
    }
    json!({"model": model, "messages": messages, "stream": true, "max_tokens": 1200})
}

fn run(body: &Value, token: &str, sink: &mut dyn FnMut(Piece) -> bool) -> Result<(), Failure> {
    let response = agent()
        .post(&format!("{GATEWAY}/chat/completions"))
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "text/event-stream")
        .send_json(body)
        .map_err(|error| Failure::Unavailable(format!("Couldn't reach October: {error}")))?;
    let status = response.status().as_u16();
    let mut reader = response.into_body().into_reader();
    if status != 200 {
        return Err(failure(status, &read_some(&mut reader)));
    }
    read_gateway_events(reader, sink)
}

/// OpenAI-style server-sent events: `choices[0].delta.content`, until `[DONE]`.
pub fn read_gateway_events(reader: impl Read, sink: &mut dyn FnMut(Piece) -> bool) -> Result<(), Failure> {
    for line in BufReader::new(reader.take(4 * 1024 * 1024)).lines() {
        let line = line.map_err(|error| Failure::Unavailable(format!("October's answer was cut off: {error}")))?;
        let Some(payload) = line.strip_prefix("data:").map(str::trim) else { continue };
        if payload == "[DONE]" {
            break;
        }
        let Ok(event) = serde_json::from_str::<Value>(payload) else { continue };
        if event["error"].is_object() {
            return Err(Failure::Unavailable(event["error"]["message"].as_str().unwrap_or("October's AI stopped answering.").into()));
        }
        if let Some(text) = event["choices"][0]["delta"]["content"].as_str().filter(|text| !text.is_empty())
            && !sink(Piece::Text(text.into()))
        {
            return Err(Failure::Cancelled);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(read: impl Fn(&mut dyn FnMut(Piece) -> bool) -> Result<(), Failure>) -> (Vec<Piece>, Result<(), Failure>) {
        let mut pieces = Vec::new();
        let result = read(&mut |piece| {
            pieces.push(piece);
            true
        });
        (pieces, result)
    }

    #[test]
    fn ask_events_stream_text_then_task_and_note() {
        let sse = "event: start\ndata: {\"type\":\"start\"}\n\ndata: {\"type\":\"delta\",\"text\":\"On \"}\n\ndata: {\"type\":\"delta\",\"text\":\"it.\"}\n\n\
                   data: {\"type\":\"done\",\"task\":{\"prompt\":\"fix the wifi\",\"agent\":\"claude\"},\"remaining\":{\"kind\":\"free\",\"questions\":3}}\n\n\
                   data: {\"type\":\"delta\",\"text\":\"ignored after done\"}\n";
        let (pieces, result) = collect(|sink| read_ask_events(sse.as_bytes(), sink));
        assert_eq!(result, Ok(()));
        assert_eq!(
            pieces,
            vec![
                Piece::Text("On ".into()),
                Piece::Text("it.".into()),
                Piece::Task { prompt: "fix the wifi".into(), agent: Some("claude".into()), project: None },
                Piece::Note("3 free questions left today.".into()),
            ]
        );
    }

    #[test]
    fn a_blank_task_starts_nothing_and_an_error_event_is_not_an_answer() {
        let (pieces, _) = collect(|sink| read_ask_events("data: {\"type\":\"done\",\"task\":{\"prompt\":\"  \"}}\n".as_bytes(), sink));
        assert!(pieces.is_empty());
        let (_, result) = collect(|sink| read_ask_events("data: {\"type\":\"error\",\"message\":\"overloaded\"}\n".as_bytes(), sink));
        assert_eq!(result, Err(Failure::Unavailable("overloaded".into())));
    }

    #[test]
    fn a_cancelled_sink_stops_the_stream() {
        let sse = "data: {\"type\":\"delta\",\"text\":\"a\"}\ndata: {\"type\":\"delta\",\"text\":\"b\"}\n";
        assert_eq!(read_ask_events(sse.as_bytes(), &mut |_| false), Err(Failure::Cancelled));
    }

    #[test]
    fn notes_only_for_free_accounts_with_ten_or_fewer_left() {
        assert_eq!(note(&json!({"remaining": {"kind": "free", "questions": 11}})), None);
        assert_eq!(note(&json!({"remaining": {"kind": "paid", "questions": 1}})), None);
        assert_eq!(note(&json!({"remaining": {"kind": "free", "questions": 1}})).as_deref(), Some("1 free question left today."));
        assert_eq!(note(&json!({"remaining": {"kind": "free", "questions": 0}})).as_deref(), Some("That was today's last free question."));
    }

    #[test]
    fn failures_map_like_the_mac() {
        let body = |code: &str, message: &str| json!({"error": {"code": code, "message": message}}).to_string().into_bytes();
        assert_eq!(failure(401, b""), Failure::SignedOut);
        assert_eq!(failure(403, &body("plan_required", "")), Failure::Plan("".into()));
        assert_eq!(failure(402, b"{}"), Failure::Plan("You've used this period's October AI credit.".into()));
        assert_eq!(failure(429, &body("free_limit", "Out of free questions")), Failure::Limited("Out of free questions".into()));
        assert_eq!(failure(400, b"not json"), Failure::Invalid("October's AI couldn't read the request.".into()));
        assert_eq!(failure(503, b""), Failure::Unavailable("October's AI isn't available right now (503). Try again soon.".into()));
    }

    #[test]
    fn ask_body_fits_what_october_accepts() {
        // October's parseAskRequest: only these keys, client exactly {app, version}, at most 10
        // history messages of 1..=8000 characters. Anything else is "a question this server can't read".
        let mut turns: Vec<_> = (0..15).map(|n| (format!("q{n}"), format!("a{n}"))).collect();
        turns[13].1 = String::new(); // cancelled before any answer
        turns[14].1 = "y".repeat(9000);
        let request = Request {
            turns,
            question: "x".repeat(5000),
            context: "c".repeat(7000),
            mode: Some("chat".into()),
            platform: Platform::Windows,
            image: Some(vec![0xFF, 0xD8, 0xFF]),
        };
        let body = ask_body(&request);
        let keys = |value: &Value| {
            let mut keys = value.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
            keys.sort();
            keys
        };
        assert_eq!(keys(&body), ["client", "context", "history", "image", "mode", "question"]);
        assert_eq!(keys(&body["client"]), ["app", "version"]);
        let history = body["history"].as_array().unwrap();
        let contents: Vec<_> = history.iter().map(|m| m["content"].as_str().unwrap()).collect();
        assert_eq!(history.len(), 10);
        assert_eq!(contents[..8], ["q9", "a9", "q10", "a10", "q11", "a11", "q12", "a12"]);
        assert_eq!((contents[8], contents[9].chars().count()), ("q14", 8000));
        assert!(history.iter().all(|m| keys(m) == ["content", "role"]));
        assert_eq!(body["question"].as_str().unwrap().len(), 4000);
        assert_eq!(body["context"].as_str().unwrap().len(), 6000);
        assert_eq!(body["mode"], "chat");
        let point = ask_body(&Request { question: "what's this?".into(), ..Default::default() });
        assert_eq!(keys(&point), ["client", "history", "question"]);
    }

    #[test]
    fn text_limits_count_like_javascript() {
        let units = |text: &str| text.encode_utf16().count();
        // "😀" is two UTF-16 units: one that would straddle the limit is left out whole.
        assert_eq!(utf16_prefix("ab😀", 3), "ab");
        assert_eq!(utf16_prefix("ab😀", 4), "ab😀");
        assert_eq!(utf16_prefix("é", 1), "é");
        let emoji = "😀".repeat(5000);
        let request = Request {
            turns: vec![("q".into(), emoji.clone())],
            question: emoji.clone(),
            context: format!("x{emoji}"),
            ..Default::default()
        };
        let body = ask_body(&request);
        assert_eq!(units(body["question"].as_str().unwrap()), 4000);
        assert_eq!(units(body["context"].as_str().unwrap()), 5999);
        assert_eq!(units(body["history"][1]["content"].as_str().unwrap()), 8000);
    }

    #[test]
    fn gateway_body_interleaves_turns_and_puts_context_with_the_first_question() {
        let request = Request {
            turns: vec![("first?".into(), "one".into())],
            question: "second?".into(),
            context: "App: Firefox".into(),
            image: Some(vec![1, 2, 3]),
            platform: Platform::Windows,
            ..Default::default()
        };
        let body = gateway_body(&request, "m", false);
        let messages = body["messages"].as_array().unwrap();
        assert!(messages[0]["content"].as_str().unwrap().contains("user's Windows PC"));
        let first = messages[1]["content"].as_str().unwrap();
        assert!(first.starts_with("first?\n\nWhere this is on my computer:\nApp: Firefox"));
        assert!(first.ends_with("(A screenshot was taken but this model can't see images.)"));
        assert_eq!(messages[2]["content"], "one");
        assert_eq!(messages[3]["content"], "second?");
        assert!(gateway_body(&request, "m", true)["messages"][1]["content"].is_array());
    }

    #[test]
    fn models_prefer_vision_then_known_families() {
        let models = |list: &[(&str, Option<bool>)]| list.iter().map(|(id, v)| (id.to_string(), *v)).collect::<Vec<_>>();
        assert_eq!(
            choose(&models(&[("tiny", Some(true)), ("claude-x", None), ("gemini-pro", Some(true))])),
            Some(Model { id: "gemini-pro".into(), vision: true })
        );
        assert_eq!(choose(&models(&[("tiny", None), ("gpt-5-mini", Some(false))])), Some(Model { id: "gpt-5-mini".into(), vision: false }));
        assert_eq!(choose(&[]), None);
    }

    #[test]
    fn gateway_events_stop_at_done_and_surface_errors() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\ndata: [DONE]\ndata: {\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n";
        let (pieces, result) = collect(|sink| read_gateway_events(sse.as_bytes(), sink));
        assert_eq!((pieces, result), (vec![Piece::Text("Hi".into())], Ok(())));
        let (_, result) = collect(|sink| read_gateway_events("data: {\"error\":{\"message\":\"nope\"}}\n".as_bytes(), sink));
        assert_eq!(result, Err(Failure::Unavailable("nope".into())));
    }
}
