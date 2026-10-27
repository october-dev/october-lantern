//! October's AI, for Ask Lantern and Point & Ask (Swift `OctoberAI.swift`).
//!
//! Questions go to Lantern's endpoint on October (`POST /api/lantern/ask`) with the October
//! account's access token: October picks the model by plan, bills paid plans' credit, and the
//! answer streams back as `delta` events and a final `done` (which may carry a task to start and
//! a note about the account). Requests fail explicitly when unavailable; no capability downgrade
//! or text-only retry silently discards attached pictures.
//!
//! What's sent: the question, earlier turns, the context lines (the app and window for Point &
//! Ask, a summary of the agents for Ask Lantern) and Point & Ask's screenshot. Nothing else.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read};
use std::time::Duration;

const ASK: &str = "https://www.october.dev/api/lantern/ask";

#[derive(Clone, Debug, Default)]
pub struct Request {
    /// Earlier questions and answers in this conversation.
    pub turns: Vec<(String, String)>,
    pub question: String,
    /// The screenshot, as JPEG.
    pub image: Option<Vec<u8>>,
    /// Several pictures (Ask Lantern): what was attached, then earlier ones in the conversation.
    pub images: Vec<Vec<u8>>,
    /// The app, window, page and selection, or the agent summary, as lines of text.
    pub context: String,
    /// "chat" for Ask Lantern (its own prompt on the server); `None` for Point & Ask.
    pub mode: Option<String>,
    pub guidance: Option<Value>,
}

/// A piece of the answer, or something to show with it.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Guide {
        guidance: Value,
        goal_id: Option<String>,
    },
    Text(String),
    /// About the account ("3 free questions left today.").
    Note(String),
    /// Work the assistant decided to start (chat mode); agent and project only when named.
    Task {
        prompt: String,
        agent: Option<String>,
        project: Option<String>,
        /// A running agent (its handle) to send it to instead of starting one.
        to: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    SignedOut,
    /// Out of free questions or credit, or the plan doesn't include it: `upgrade` is where to
    /// upgrade.
    Plan {
        message: String,
        upgrade: String,
    },
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
            Self::Plan { message: m, .. } | Self::Limited(m) | Self::Unavailable(m) | Self::Invalid(m) => m,
        })
    }
}

/// October's pricing page.
pub const PRICING: &str = "https://www.october.dev/pricing";

impl Failure {
    /// Where to upgrade, when a plan would help.
    pub fn upgrade(&self) -> Option<&str> {
        match self {
            Self::Plan { upgrade, .. } => Some(upgrade),
            _ => None,
        }
    }
}

/// The server's upgrade link when it's October's own, else the pricing page.
fn upgrade_url(value: &Value) -> String {
    value
        .as_str()
        .and_then(|s| url::Url::parse(s).ok())
        .filter(|url| url.scheme() == "https" && matches!(url.host_str(), Some("www.october.dev" | "october.dev")))
        .map(|url| url.to_string())
        .unwrap_or_else(|| PRICING.into())
}

/// Stream exactly the requested capability. Never downgrade modes or discard pictures.
pub fn stream(request: &Request, token: &str, sink: &mut dyn FnMut(Piece) -> bool) -> Result<(), Failure> {
    ask(request, token, sink)
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
    if let Some(guidance) = &request.guidance {
        body["guidance"] = guidance.clone();
    }
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
    if !request.images.is_empty() {
        use base64::Engine;
        body["images"] = request.images.iter().map(|i| base64::engine::general_purpose::STANDARD.encode(i)).collect::<Vec<_>>().into();
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
                if let Some(problem) = event["taskError"].as_str() {
                    return Err(Failure::Unavailable(problem.to_string()));
                }
                if event.get("guidance").is_some() {
                    sink(Piece::Guide { guidance: event["guidance"].clone(), goal_id: event["goalId"].as_str().map(str::to_string) });
                }
                if let Some(task) = event["task"].as_object()
                    && let Some(prompt) = task.get("prompt").and_then(Value::as_str).filter(|prompt| !prompt.trim().is_empty())
                {
                    let named = |key: &str| task.get(key).and_then(Value::as_str).map(str::to_string);
                    sink(Piece::Task {
                        prompt: prompt.chars().take(4000).collect(),
                        agent: named("agent"),
                        project: named("project"),
                        to: named("to"),
                    });
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
    let plan = |default: &str| Failure::Plan {
        message: message.clone().unwrap_or_else(|| default.into()),
        upgrade: upgrade_url(&error["upgradeUrl"]),
    };
    match (status, code) {
        (401, _) => Failure::SignedOut,
        (_, "plan_required") => plan("This needs an October Pro or Max plan."),
        (402, _) | (_, "credit_exhausted") => plan("You've used this period's October AI credit."),
        (_, "free_limit") => plan("That's today's free questions. Upgrade to October Pro for more."),
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
                Piece::Task { prompt: "fix the wifi".into(), agent: Some("claude".into()), project: None, to: None },
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
        let plan = |message: &str| Failure::Plan { message: message.into(), upgrade: PRICING.into() };
        assert_eq!(failure(403, &body("plan_required", "")), plan(""));
        assert_eq!(failure(402, b"{}"), plan("You've used this period's October AI credit."));
        assert_eq!(failure(429, &body("free_limit", "Out of free questions")), plan("Out of free questions"));
        assert_eq!(failure(429, &body("rate_limited", "Slow down")), Failure::Limited("Slow down".into()), "not a plan problem");
        let ours = json!({"error": {"code": "credit_exhausted", "upgradeUrl": "https://www.october.dev/pricing?from=lantern"}});
        assert_eq!(failure(402, ours.to_string().as_bytes()).upgrade(), Some("https://www.october.dev/pricing?from=lantern"));
        let theirs = json!({"error": {"code": "credit_exhausted", "upgradeUrl": "https://evil.example/pay"}});
        assert_eq!(failure(402, theirs.to_string().as_bytes()).upgrade(), Some(PRICING), "only October's own links");
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

            image: Some(vec![0xFF, 0xD8, 0xFF]),
            ..Default::default()
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
    fn invalid_task_is_an_explicit_failure_not_a_started_task() {
        let sse = "data: {\"type\":\"done\",\"task\":null,\"taskError\":\"No task was started.\"}\n";
        let (pieces, result) = collect(|sink| read_ask_events(sse.as_bytes(), sink));
        assert!(pieces.is_empty());
        assert_eq!(result, Err(Failure::Unavailable("No task was started.".into())));
    }
}
