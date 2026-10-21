//! Anonymous usage counts, sent to PostHog (US Cloud), as the Mac app does (Swift `Analytics`): how
//! many people use Lantern each day and which features they use. Never content: no messages,
//! prompts, commands, folder or project names, file paths or agent titles. Every property sent is
//! listed at its call site.
//!
//! Installs are identified by a random id. Someone signed in to October is linked to their October
//! account (id and email), so usage can be tied to an account.
//!
//! On by default, off in Settings; nothing is sent (or kept) while off. Builds without a PostHog key
//! (`LANTERN_POSTHOG_KEY` when building, set by the release workflow) send nothing at all.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

const ENDPOINT: &str = "https://us.i.posthog.com/batch/";
const KEY: Option<&str> = option_env!("LANTERN_POSTHOG_KEY");

/// What's kept between launches, in `analytics.json` in Lantern's data folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    enabled: bool,
    install_id: String,
    /// The October account id while signed in.
    user_id: Option<String>,
    /// The day `lantern_active` was last sent (`2026-10-21`).
    last_active: String,
}

impl Default for Saved {
    fn default() -> Self {
        Self { enabled: true, install_id: String::new(), user_id: None, last_active: String::new() }
    }
}

struct State {
    saved: Saved,
    queue: Vec<Value>,
    /// When the queue is sent next.
    flush_at: Option<Instant>,
    sending: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn path() -> PathBuf {
    crate::settings::data_dir().join("analytics.json")
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(|| {
        let mut saved: Saved = std::fs::read_to_string(path()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        if saved.install_id.is_empty() {
            saved.install_id = uuid::Uuid::new_v4().to_string();
            save(&saved);
        }
        State { saved, queue: Vec::new(), flush_at: None, sending: false }
    });
    f(state)
}

fn save(saved: &Saved) {
    if cfg!(test) {
        return;
    }
    if let Ok(text) = serde_json::to_string(saved) {
        let _ = crate::settings::private_write(&path(), text.as_bytes());
    }
}

/// Whether this build can send anything (a release build with a key).
pub fn available() -> bool {
    KEY.is_some_and(|key| !key.is_empty()) && !cfg!(test)
}

pub fn enabled() -> bool {
    available() && with(|s| s.saved.enabled)
}

pub fn set_enabled(on: bool) {
    with(|s| {
        s.saved.enabled = on;
        if !on {
            s.queue.clear();
        }
        save(&s.saved);
    });
}

fn platform() -> &'static str {
    if cfg!(windows) { "windows" } else { "linux" }
}

pub fn capture(event: &str, properties: Value) {
    if !enabled() {
        return;
    }
    with(|s| {
        let mut props = properties.as_object().cloned().unwrap_or_default();
        props.insert("distinct_id".into(), json!(s.saved.user_id.clone().unwrap_or_else(|| s.saved.install_id.clone())));
        props.insert("$lib".into(), json!(format!("lantern-{}", platform())));
        props.insert("app_version".into(), json!(env!("CARGO_PKG_VERSION")));
        props.insert("os".into(), json!(platform()));
        props.insert("arch".into(), json!(std::env::consts::ARCH));
        // PostHog derives an approximate location (country, city) from the IP address it sees.
        s.queue.push(json!({"event": event, "properties": Value::Object(props), "timestamp": now_iso()}));
        if s.queue.len() > 200 {
            let extra = s.queue.len() - 200;
            s.queue.drain(..extra);
        }
        let wait = Duration::from_secs(if s.queue.len() >= 20 { 1 } else { 60 });
        let at = Instant::now() + wait;
        if s.flush_at.is_none_or(|t| at < t) {
            s.flush_at = Some(at);
        }
    });
}

/// Links this install's usage to an October account (after sign-in, or on launch while signed in).
pub fn identify(access_token: &str, email: &str) {
    let Some(user_id) = user_id(access_token) else { return };
    if !enabled() || with(|s| s.saved.user_id.as_deref() == Some(user_id.as_str())) {
        return;
    }
    let anonymous = with(|s| {
        let anonymous = s.saved.user_id.clone().unwrap_or_else(|| s.saved.install_id.clone());
        s.saved.user_id = Some(user_id.clone());
        save(&s.saved);
        anonymous
    });
    let mut set = Map::new();
    if email.contains('@') {
        set.insert("email".into(), json!(email));
    }
    capture("$identify", json!({"$anon_distinct_id": anonymous, "$set": set}));
}

/// Signed out: later usage is anonymous again.
pub fn forget_user() {
    with(|s| {
        s.saved.user_id = None;
        save(&s.saved);
    });
}

/// Once per calendar day while the app runs: what DAU and MAU are counted from. `kinds` is how
/// many agents of each harness are running.
pub fn daily_active(kinds: &[(String, usize)]) {
    if !enabled() {
        return;
    }
    let today = now_iso()[..10].to_string();
    let first = with(|s| {
        let first = s.saved.last_active != today;
        if first {
            s.saved.last_active = today.clone();
            save(&s.saved);
        }
        first
    });
    if first {
        let mut props = Map::new();
        props.insert("agents_running".into(), json!(kinds.iter().map(|(_, n)| n).sum::<usize>()));
        for (kind, n) in kinds {
            props.insert(format!("agents_{kind}"), json!(n));
        }
        capture("lantern_active", Value::Object(props));
    }
}

/// Sends what's queued when it's due (called from the app's tick). Failed sends stay queued.
pub fn tick() {
    let Some(key) = KEY.filter(|_| enabled()) else { return };
    let batch = with(|s| {
        if s.sending || s.queue.is_empty() || s.flush_at.is_none_or(|t| Instant::now() < t) {
            return None;
        }
        s.sending = true;
        s.flush_at = None;
        Some(s.queue.clone())
    });
    let Some(batch) = batch else { return };
    std::thread::spawn(move || {
        let sent = ureq::post(ENDPOINT)
            .config()
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .send_json(json!({"api_key": key, "batch": batch}))
            .is_ok_and(|response| response.status().is_success());
        with(|s| {
            s.sending = false;
            if sent {
                let n = batch.len().min(s.queue.len());
                s.queue.drain(..n);
            } else {
                s.flush_at = Some(Instant::now() + Duration::from_secs(300));
            }
        });
    });
}

/// The October account id: the `sub` claim of its access token.
fn user_id(access_token: &str) -> Option<String> {
    use base64::Engine;
    let payload = access_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims["sub"].as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Now, as UTC ISO 8601 (`2026-10-21T09:30:00Z`).
fn now_iso() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builds_send_nothing() {
        assert!(!available());
        assert!(!enabled());
        capture("lantern_active", json!({}));
        assert!(with(|s| s.queue.is_empty()));
    }

    #[test]
    fn the_account_id_comes_from_the_access_token() {
        use base64::Engine;
        let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"sub":"user-1","email":"a@b.c"}"#);
        assert_eq!(user_id(&format!("h.{body}.s")).as_deref(), Some("user-1"));
        assert_eq!(user_id("not a token"), None);
    }

    #[test]
    fn dates_are_utc_iso() {
        let now = now_iso();
        assert_eq!(now.len(), 20);
        assert!(now.starts_with("20") && now.ends_with('Z') && &now[10..11] == "T");
    }
}
