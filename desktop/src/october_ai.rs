//! October's AI, for Ask Lantern and Point & Ask (Swift `OctoberAI.swift`).
//!
//! Questions go to Lantern's endpoint on October (`POST /api/lantern/ask`) with the October
//! account's access token: October picks the model by plan, bills paid plans' credit, and the
//! answer streams back as `delta` events and a final `done` (which may carry a task to start and
//! a note about the account). Requests fail explicitly when unavailable; no capability downgrade
//! or text-only retry silently discards attached pictures.
//!
//! What's sent: the question, earlier turns, the context lines (the app and window for Point &
//! Ask, a summary of the agents for Ask Lantern) and Point & Ask's screenshot. Nothing else; with
//! your own AI key, also that key, its provider and model (headers October passes on, never keeps).
use crate::model::Fix;
use crate::own_key::OwnKey;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read};
use std::time::Duration;

const ASK: &str = "https://www.october.dev/api/lantern/ask";

#[derive(Clone, Debug)]
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
    /// Your own AI key may answer (when one is given to `stream`); off for "Ask October's AI
    /// instead".
    pub use_own_key: bool,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            turns: Vec::new(),
            question: String::new(),
            image: None,
            images: Vec::new(),
            context: String::new(),
            mode: None,
            guidance: None,
            use_own_key: true,
        }
    }
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
    /// Your own key answered (`start.tier` "byok"): its provider ("OpenAI").
    Via(String),
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
    /// Your provider didn't accept your own key.
    OwnKey(String),
    /// Your provider couldn't use the model set with your key.
    OwnModel(String),
    /// Your provider refused (its rate limit or billing) or failed: October's AI can be asked
    /// instead.
    Provider(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Self::SignedOut => "Sign in to October to ask.",
            Self::NotDeployed => "October's AI isn't available right now.",
            Self::Cancelled => "Cancelled.",
            Self::Plan { message: m, .. }
            | Self::Limited(m)
            | Self::Unavailable(m)
            | Self::Invalid(m)
            | Self::OwnKey(m)
            | Self::OwnModel(m)
            | Self::Provider(m) => m,
        })
    }
}

/// Where Lantern's offers are bought: the Pro trial and Lantern Lifetime.
pub const LANTERN: &str = "https://www.october.dev/lantern";

impl Failure {
    /// What the failed answer offers.
    pub fn fix(&self) -> Option<Fix> {
        match self {
            Self::Plan { upgrade, .. } => Some(Fix::Upgrade(upgrade.clone())),
            Self::OwnKey(_) => Some(Fix::EditKey),
            Self::OwnModel(_) => Some(Fix::OpenSettings),
            Self::Provider(_) => Some(Fix::UseOctober),
            _ => None,
        }
    }
}

/// The server's upgrade link when it's October's own, else Lantern's offers page.
fn upgrade_url(value: &Value) -> String {
    value.as_str().and_then(|s| url::Url::parse(s).ok()).filter(october_url).map(|url| url.to_string()).unwrap_or_else(|| LANTERN.into())
}

fn october_url(url: &url::Url) -> bool {
    url.scheme() == "https" && matches!(url.host_str(), Some("www.october.dev" | "october.dev"))
}

/// The offers, from Lantern: `offer` "trial" or "lifetime" on October's page `base` (its own query
/// replaced), or on Lantern's offers page when `base` isn't October's.
pub fn offer_url(base: &str, offer: &str) -> String {
    let mut url = url::Url::parse(base).ok().filter(october_url).unwrap_or_else(|| url::Url::parse(LANTERN).expect("a valid URL"));
    url.set_query(Some(&format!("offer={offer}&ref=app")));
    url.set_fragment(None);
    url.to_string()
}

/// Stream exactly the requested capability. Never downgrade modes or discard pictures. `own_key`
/// answers when the request allows it.
pub fn stream(request: &Request, token: &str, own_key: Option<&OwnKey>, sink: &mut dyn FnMut(Piece) -> bool) -> Result<(), Failure> {
    ask(request, token, own_key.filter(|_| request.use_own_key), sink)
}

/// Your own key for October to pass on: all three headers, or none; the model only when set.
pub fn byok_headers(request: &Request, own_key: Option<&OwnKey>) -> Vec<(&'static str, String)> {
    let Some(key) = own_key.filter(|_| request.use_own_key) else { return Vec::new() };
    let mut headers = vec![("X-Lantern-Byok-Provider", key.provider.id().to_string()), ("X-Lantern-Byok-Key", key.key.clone())];
    if !key.model.trim().is_empty() {
        headers.push(("X-Lantern-Byok-Model", key.model.trim().to_string()));
    }
    headers
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

fn ask(request: &Request, token: &str, own_key: Option<&OwnKey>, sink: &mut dyn FnMut(Piece) -> bool) -> Result<(), Failure> {
    let mut post = agent().post(ASK).header("Authorization", &format!("Bearer {token}")).header("Accept", "text/event-stream");
    for (name, value) in byok_headers(request, own_key) {
        post = post.header(name, &value);
    }
    let response = post.send_json(ask_body(request)).map_err(|error| Failure::Unavailable(format!("Couldn't reach October: {error}")))?;
    let status = response.status().as_u16();
    if status == 404 || status == 405 {
        return Err(Failure::NotDeployed);
    }
    let mut body = response.into_body().into_reader();
    if status != 200 {
        return Err(failure(status, &read_some(&mut body)));
    }
    read_ask_events(body, own_key.map(|key| key.provider.label()), sink)
}

/// Lantern's endpoint: `start` (who answers), `delta` text, then `done` (with an optional task and
/// account note). `own_key`: the provider of the key sent, named when it answered.
pub fn read_ask_events(reader: impl Read, own_key: Option<&str>, sink: &mut dyn FnMut(Piece) -> bool) -> Result<(), Failure> {
    for line in BufReader::new(reader.take(4 * 1024 * 1024)).lines() {
        let line = line.map_err(|error| Failure::Unavailable(format!("October's answer was cut off: {error}")))?;
        let Some(event) = line.strip_prefix("data:").and_then(|payload| serde_json::from_str::<Value>(payload.trim()).ok()) else {
            continue;
        };
        match event["type"].as_str() {
            Some("start") => {
                if event["tier"] == "byok"
                    && let Some(provider) = own_key
                    && !sink(Piece::Via(provider.into()))
                {
                    return Err(Failure::Cancelled);
                }
            }
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
    Err(Failure::Unavailable("October's answer stopped before it finished. Try again.".into()))
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
    let said = |default: &str| message.clone().unwrap_or_else(|| default.into());
    // Your own key's problems first: their statuses (400, 402, 429) mean other things otherwise.
    match code {
        "byok_requires_plan" => return plan("Your own key needs Lantern Lifetime or the Pro trial."),
        "byok_key_invalid" => return Failure::OwnKey(said("Your AI provider didn't accept your key. Check it in Settings.")),
        "byok_model_unavailable" => {
            return Failure::OwnModel(said("Your AI provider couldn't use that model. Check the model in Settings."));
        }
        "byok_provider_limited" => return Failure::Provider(said("Your AI provider refused the question: its rate limit or billing.")),
        "byok_provider_failed" => {
            return Failure::Provider(said("Your AI provider didn't answer. Try again, or ask October's AI instead."));
        }
        _ => {}
    }
    match (status, code) {
        (401, _) => Failure::SignedOut,
        (_, "plan_required") => plan("This needs an October Pro or Max plan."),
        (402, _) | (_, "credit_exhausted") => plan("You've used this period's October AI credit."),
        (_, "free_limit") => plan("You've used today's free questions. They reset at midnight UTC."),
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
        let (pieces, result) = collect(|sink| read_ask_events(sse.as_bytes(), None, sink));
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
        let (pieces, _) =
            collect(|sink| read_ask_events("data: {\"type\":\"done\",\"task\":{\"prompt\":\"  \"}}\n".as_bytes(), None, sink));
        assert!(pieces.is_empty());
        let (_, result) = collect(|sink| read_ask_events("data: {\"type\":\"error\",\"message\":\"overloaded\"}\n".as_bytes(), None, sink));
        assert_eq!(result, Err(Failure::Unavailable("overloaded".into())));
    }

    #[test]
    fn a_cancelled_sink_stops_the_stream() {
        let sse = "data: {\"type\":\"delta\",\"text\":\"a\"}\ndata: {\"type\":\"delta\",\"text\":\"b\"}\n";
        assert_eq!(read_ask_events(sse.as_bytes(), None, &mut |_| false), Err(Failure::Cancelled));
    }

    #[test]
    fn a_truncated_stream_keeps_partial_text_and_reports_failure() {
        for sse in ["", "data: {\"type\":\"delta\",\"text\":\"Partial answer\"}\n", "data: {\"type\":\"done\""] {
            let (pieces, result) = collect(|sink| read_ask_events(sse.as_bytes(), None, sink));
            assert!(matches!(result, Err(Failure::Unavailable(_))), "EOF without a terminal receipt is not completion");
            if sse.contains("Partial answer") {
                assert_eq!(pieces, [Piece::Text("Partial answer".into())]);
            }
            assert!(!pieces.iter().any(|piece| matches!(piece, Piece::Task { .. } | Piece::Guide { .. })));
        }
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
        let plan = |message: &str| Failure::Plan { message: message.into(), upgrade: LANTERN.into() };
        assert_eq!(failure(403, &body("plan_required", "")), plan(""));
        assert_eq!(failure(402, b"{}"), plan("You've used this period's October AI credit."));
        assert_eq!(failure(429, &body("free_limit", "Out of free questions")), plan("Out of free questions"));
        assert_eq!(failure(429, &body("rate_limited", "Slow down")), Failure::Limited("Slow down".into()), "not a plan problem");
        let ours = json!({"error": {"code": "credit_exhausted", "upgradeUrl": "https://www.october.dev/pricing?from=lantern"}});
        let upgrade = |failure: Failure| match failure.fix() {
            Some(Fix::Upgrade(url)) => url,
            other => panic!("{other:?}"),
        };
        assert_eq!(upgrade(failure(402, ours.to_string().as_bytes())), "https://www.october.dev/pricing?from=lantern");
        let theirs = json!({"error": {"code": "credit_exhausted", "upgradeUrl": "https://evil.example/pay"}});
        assert_eq!(upgrade(failure(402, theirs.to_string().as_bytes())), LANTERN, "only October's own links");
        assert_eq!(failure(429, &body("free_limit", "")), plan(""), "the server's message as it is");
        assert_eq!(
            failure(429, br#"{"error":{"code":"free_limit"}}"#),
            plan("You've used today's free questions. They reset at midnight UTC.")
        );
        assert_eq!(failure(400, b"not json"), Failure::Invalid("October's AI couldn't read the request.".into()));
        assert_eq!(failure(503, b""), Failure::Unavailable("October's AI isn't available right now (503). Try again soon.".into()));
    }

    #[test]
    fn your_own_keys_problems_come_first_and_say_what_to_do() {
        let body = |code: &str, message: Option<&str>| json!({"error": {"code": code, "message": message}}).to_string().into_bytes();
        let needs = failure(402, &body("byok_requires_plan", None));
        assert_eq!(needs.to_string(), "Your own key needs Lantern Lifetime or the Pro trial.");
        assert_eq!(needs.fix(), Some(Fix::Upgrade(LANTERN.into())));
        let cases = [
            (
                400,
                "byok_key_invalid",
                Failure::OwnKey("Your AI provider didn't accept your key. Check it in Settings.".into()),
                Fix::EditKey,
            ),
            (
                400,
                "byok_model_unavailable",
                Failure::OwnModel("Your AI provider couldn't use that model. Check the model in Settings.".into()),
                Fix::OpenSettings,
            ),
            (
                429,
                "byok_provider_limited",
                Failure::Provider("Your AI provider refused the question: its rate limit or billing.".into()),
                Fix::UseOctober,
            ),
            (
                502,
                "byok_provider_failed",
                Failure::Provider("Your AI provider didn't answer. Try again, or ask October's AI instead.".into()),
                Fix::UseOctober,
            ),
        ];
        for (status, code, expected, fix) in cases {
            let got = failure(status, &body(code, None));
            assert_eq!(got, expected, "{code}");
            assert_eq!(got.fix(), Some(fix), "{code}");
        }
        assert_eq!(failure(400, &body("byok_key_invalid", Some("OpenAI said: bad key"))), Failure::OwnKey("OpenAI said: bad key".into()));
    }

    #[test]
    fn offers_open_lanterns_page_on_october_only() {
        assert_eq!(offer_url(LANTERN, "trial"), "https://www.october.dev/lantern?offer=trial&ref=app");
        assert_eq!(offer_url(LANTERN, "lifetime"), "https://www.october.dev/lantern?offer=lifetime&ref=app");
        assert_eq!(offer_url("https://october.dev/lantern?from=x#top", "trial"), "https://october.dev/lantern?offer=trial&ref=app");
        assert_eq!(offer_url("https://evil.example/lantern", "lifetime"), "https://www.october.dev/lantern?offer=lifetime&ref=app");
        assert_eq!(offer_url("http://www.october.dev/lantern", "trial"), "https://www.october.dev/lantern?offer=trial&ref=app");
        let none = json!({"error": {"code": "free_limit"}});
        assert_eq!(failure(429, none.to_string().as_bytes()).fix(), Some(Fix::Upgrade(LANTERN.into())));
    }

    #[test]
    fn your_key_goes_as_three_headers_or_none() {
        let key = |model: &str| OwnKey::new(crate::own_key::AiProvider::Anthropic, "sk-ant-1", model).unwrap();
        let request = Request::default();
        assert!(request.use_own_key, "real questions may use your key");
        let names =
            |headers: Vec<(&'static str, String)>| headers.into_iter().map(|(name, value)| format!("{name}: {value}")).collect::<Vec<_>>();
        assert_eq!(
            names(byok_headers(&request, Some(&key("claude-sonnet-4-5")))),
            ["X-Lantern-Byok-Provider: anthropic", "X-Lantern-Byok-Key: sk-ant-1", "X-Lantern-Byok-Model: claude-sonnet-4-5"]
        );
        assert_eq!(names(byok_headers(&request, Some(&key("")))), ["X-Lantern-Byok-Provider: anthropic", "X-Lantern-Byok-Key: sk-ant-1"]);
        assert!(byok_headers(&request, None).is_empty());
        let october = Request { use_own_key: false, ..Default::default() };
        assert!(byok_headers(&october, Some(&key("x"))).is_empty(), "Ask October's AI instead");
        assert!(!format!("{october:?}").contains("sk-ant"), "the request never carries the key");
    }

    #[test]
    fn a_byok_start_names_the_provider_that_answered() {
        let sse = "data: {\"type\":\"start\",\"tier\":\"byok\",\"model\":\"gpt-4.1\"}\ndata: {\"type\":\"delta\",\"text\":\"Hi\"}\ndata: {\"type\":\"done\"}\n";
        let (pieces, result) = collect(|sink| read_ask_events(sse.as_bytes(), Some("OpenAI"), sink));
        assert_eq!(result, Ok(()));
        assert_eq!(pieces, [Piece::Via("OpenAI".into()), Piece::Text("Hi".into())]);
        let october = sse.replace("byok", "free");
        let (pieces, _) = collect(|sink| read_ask_events(october.as_bytes(), Some("OpenAI"), sink));
        assert_eq!(pieces, [Piece::Text("Hi".into())], "October's AI answered");
        let (pieces, _) = collect(|sink| read_ask_events(sse.as_bytes(), None, sink));
        assert_eq!(pieces, [Piece::Text("Hi".into())], "no key was sent");
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
        let (pieces, result) = collect(|sink| read_ask_events(sse.as_bytes(), None, sink));
        assert!(pieces.is_empty());
        assert_eq!(result, Err(Failure::Unavailable("No task was started.".into())));
    }
}
