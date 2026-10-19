//! Push notifications to paired phones when an agent needs the person, like October Desktop's
//! `push-sender.ts`: the phone registers an Expo push token (`remote.push.register`), Lantern
//! posts to Expo's push service when an agent starts waiting, and the phone opens that agent
//! when the notification is tapped.
//!
//! One attention episode is one push: an agent that starts waiting (or starts asking a question)
//! is pushed once, after it has stayed that way for a moment, and not again until it has been
//! seen doing something else. Attention that began before this engine started isn't pushed, so a
//! restart doesn't repeat old notifications. The notification says only which agent wants you
//! (its handle, e.g. `@claude-1`), never what it said.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::api;
use crate::model::{Agent, State};

pub const SEND_URL: &str = "https://exp.host/--/api/v2/push/send";
/// How long an agent must keep wanting the person before it's pushed, so a blip isn't.
pub const SETTLE: Duration = Duration::from_secs(2);
/// Attention older than this when first seen is old news, not a transition (e.g. a clock skew).
const FRESH_MS: u64 = 10 * 60_000;
/// Expo accepts at most 100 messages per request.
const BATCH: usize = 100;
const SENT_REMEMBERED: usize = 512;

/// One push to send: the phone deep-links with `data` (October's `RemotePushData`).
#[derive(Debug, Clone, PartialEq)]
pub struct Episode {
    pub notification_id: String,
    pub node_id: String,
    pub kind: &'static str,
    pub handle: String,
}

struct Watch {
    state: State,
    since: Option<u64>,
    seen_at: Instant,
    pushed: bool,
}

/// Turns agent snapshots into attention episodes that are due a push.
pub struct Attention {
    started_ms: u64,
    watching: HashMap<String, Watch>,
    sent: VecDeque<String>,
    sent_set: HashSet<String>,
}

impl Attention {
    pub fn new(started_ms: u64) -> Attention {
        Attention { started_ms, watching: HashMap::new(), sent: VecDeque::new(), sent_set: HashSet::new() }
    }

    /// Episodes that became due with this snapshot; each is returned once.
    pub fn due(&mut self, agents: &[Agent], now: Instant, now_ms: u64) -> Vec<Episode> {
        let wanting: HashMap<&str, &Agent> = agents.iter().filter(|a| api::wants(a)).map(|a| (a.id.as_str(), a)).collect();
        self.watching.retain(|id, w| wanting.get(id.as_str()).is_some_and(|a| a.state == w.state));
        let mut due = Vec::new();
        for (id, agent) in wanting {
            let notification_id = api::notification_id(agent);
            let w = self.watching.entry(id.to_string()).or_insert_with(|| {
                // Only attention that began while this engine was running, and recently.
                let fresh = agent.state_since.is_some_and(|t| t >= self.started_ms && t + FRESH_MS >= now_ms);
                Watch { state: agent.state, since: agent.state_since, seen_at: now, pushed: !fresh }
            });
            if w.since != agent.state_since {
                // Same episode (still wanting the person), newer timestamp: don't push again.
                w.since = agent.state_since;
            }
            if w.pushed || now.duration_since(w.seen_at) < SETTLE {
                continue;
            }
            w.pushed = true;
            if !self.sent_set.insert(notification_id.clone()) {
                continue;
            }
            self.sent.push_back(notification_id.clone());
            if self.sent.len() > SENT_REMEMBERED
                && let Some(old) = self.sent.pop_front()
            {
                self.sent_set.remove(&old);
            }
            let kind = if agent.state == State::NeedsInput { "needs_input" } else { "done" };
            due.push(Episode { notification_id, node_id: api::node_id(&agent.id), kind, handle: agent.handle.clone() });
        }
        due.sort_by(|a, b| a.notification_id.cmp(&b.notification_id));
        due
    }
}

/// Expo push messages for one episode, one per phone token. Generic copy: no agent output.
pub fn messages(episode: &Episode, host_id: &str, canvas_id: &str, tokens: &[String]) -> Vec<Value> {
    let (title, body) = if episode.kind == "needs_input" {
        (format!("@{} needs you", episode.handle), "An agent is waiting for your answer.")
    } else {
        (format!("@{} finished", episode.handle), "An agent is waiting for your reply.")
    };
    let data = json!({
        "hostId": host_id, "canvasId": canvas_id, "nodeId": episode.node_id,
        "kind": episode.kind, "notificationId": episode.notification_id,
    });
    tokens
        .iter()
        .map(|to| {
            json!({"to": to, "sound": "default", "title": title, "body": body, "data": data,
                "priority": "high", "channelId": "default"})
        })
        .collect()
}

/// Tokens Expo says no longer reach a phone (`DeviceNotRegistered`), in send order.
pub fn unregistered(tokens: &[String], response: &Value) -> Vec<String> {
    let tickets = response["data"].as_array().cloned().unwrap_or_default();
    tokens
        .iter()
        .zip(tickets.iter())
        .filter(|(_, t)| t["details"]["error"] == "DeviceNotRegistered")
        .map(|(token, _)| token.clone())
        .collect()
}

/// Sends `episode` to every phone that wants notifications, off the caller's thread. Retries
/// rate limits, server errors and network failures a few times; tokens Expo has dropped are
/// forgotten.
pub fn send(episode: Episode, host_id: String, canvas_id: String) {
    let targets = super::store::push_targets();
    if targets.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        let agent: ureq::Agent =
            ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(15))).build().into();
        for chunk in targets.chunks(BATCH) {
            let tokens: Vec<String> = chunk.iter().map(|(_, t)| t.clone()).collect();
            let body = Value::Array(messages(&episode, &host_id, &canvas_id, &tokens));
            let Some(response) = post(&agent, &body) else { continue };
            for token in unregistered(&tokens, &response) {
                if let Some((bind, _)) = chunk.iter().find(|(_, t)| *t == token) {
                    let _ = super::store::forget_push_token(bind, &token);
                }
            }
        }
    });
}

fn post(agent: &ureq::Agent, body: &Value) -> Option<Value> {
    let raw = serde_json::to_vec(body).ok()?;
    for attempt in 0..3u32 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(250 << attempt));
        }
        match agent.post(SEND_URL).header("content-type", "application/json").header("accept", "application/json").send(&raw[..]) {
            Ok(mut resp) => {
                let status = resp.status().as_u16();
                if (200..300).contains(&status) {
                    return serde_json::from_str(&resp.body_mut().read_to_string().unwrap_or_default()).ok();
                }
                if status != 429 && status < 500 {
                    eprintln!("lantern phone: Expo push refused the request ({status})");
                    return None;
                }
            }
            Err(e) => eprintln!("lantern phone: Expo push: {e}"),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, state: State, since: u64) -> Agent {
        let mut a = crate::tests::agent_for(42, 1, Some("ttys001"));
        a.id = id.into();
        a.handle = "claude-1".into();
        a.state = state;
        a.state_since = Some(since);
        a.last_message = Some("secret output".into());
        a
    }

    #[test]
    fn one_attention_episode_is_one_push_after_it_settles() {
        let t0 = Instant::now();
        let mut att = Attention::new(1_000);
        let waiting = agent("a", State::Waiting, 2_000);
        assert!(att.due(std::slice::from_ref(&waiting), t0, 2_000).is_empty(), "not before it settles");
        let due = att.due(std::slice::from_ref(&waiting), t0 + SETTLE, 4_000);
        assert_eq!(due.len(), 1);
        assert_eq!((due[0].kind, due[0].node_id.as_str()), ("done", api::node_id("a").as_str()));
        assert_eq!(due[0].notification_id, format!("{}-2000", api::node_id("a")));
        assert!(att.due(std::slice::from_ref(&waiting), t0 + SETTLE * 3, 6_000).is_empty(), "pushed once");
        // A newer timestamp while still waiting is the same episode.
        let restamped = agent("a", State::Waiting, 3_000);
        assert!(att.due(std::slice::from_ref(&restamped), t0 + SETTLE * 5, 8_000).is_empty());
        // A question is new attention, even straight after waiting.
        let asking = agent("a", State::NeedsInput, 9_000);
        att.due(std::slice::from_ref(&asking), t0 + SETTLE * 6, 9_000);
        let due = att.due(std::slice::from_ref(&asking), t0 + SETTLE * 7, 11_000);
        assert_eq!(due.iter().map(|e| e.kind).collect::<Vec<_>>(), ["needs_input"]);
        // Working, then waiting again: a new episode.
        att.due(&[agent("a", State::Working, 12_000)], t0 + SETTLE * 8, 12_000);
        att.due(&[agent("a", State::Waiting, 13_000)], t0 + SETTLE * 9, 13_000);
        assert_eq!(att.due(&[agent("a", State::Waiting, 13_000)], t0 + SETTLE * 10, 15_000).len(), 1);
    }

    #[test]
    fn blips_old_attention_and_rescans_are_not_pushed() {
        let t0 = Instant::now();
        let mut att = Attention::new(10_000);
        // Waiting since before the engine started: a restart doesn't repeat it.
        let old = agent("old", State::NeedsInput, 9_000);
        att.due(std::slice::from_ref(&old), t0, 10_000);
        assert!(att.due(std::slice::from_ref(&old), t0 + SETTLE, 12_000).is_empty());
        // Stale by the clock.
        let stale = agent("stale", State::Waiting, 11_000);
        att.due(std::slice::from_ref(&stale), t0, 11_000 + FRESH_MS + 1);
        assert!(att.due(std::slice::from_ref(&stale), t0 + SETTLE, 11_000 + FRESH_MS + 2).is_empty());
        // A blip: waiting, then working before it settles.
        att.due(&[agent("blip", State::Waiting, 20_000)], t0, 20_000);
        att.due(&[agent("blip", State::Working, 20_500)], t0 + SETTLE / 2, 20_500);
        assert!(att.due(&[agent("blip", State::Working, 20_500)], t0 + SETTLE * 2, 23_000).is_empty());
        // Pushed, then missing from one scan, then back unchanged: not pushed twice.
        let back = agent("back", State::Waiting, 30_000);
        att.due(std::slice::from_ref(&back), t0, 30_000);
        assert_eq!(att.due(std::slice::from_ref(&back), t0 + SETTLE, 32_000).len(), 1);
        att.due(&[], t0 + SETTLE * 2, 33_000);
        att.due(std::slice::from_ref(&back), t0 + SETTLE * 3, 34_000);
        assert!(att.due(std::slice::from_ref(&back), t0 + SETTLE * 5, 36_000).is_empty());
    }

    #[test]
    fn messages_carry_the_deep_link_and_no_agent_output() {
        let episode = Episode { notification_id: "n-1".into(), node_id: "node".into(), kind: "needs_input", handle: "codex-2".into() };
        let m = messages(&episode, "host", "canvas", &["ExponentPushToken[a]".into(), "ExponentPushToken[b]".into()]);
        assert_eq!(m.len(), 2);
        assert_eq!(m[1]["to"], "ExponentPushToken[b]");
        assert_eq!(
            m[0]["data"],
            json!({"hostId": "host", "canvasId": "canvas", "nodeId": "node", "kind": "needs_input", "notificationId": "n-1"})
        );
        assert_eq!(m[0]["title"], "@codex-2 needs you");
        assert!(!m[0].to_string().contains("secret"));
    }

    #[test]
    fn expo_device_not_registered_tickets_name_their_tokens() {
        let tokens = vec!["t1".to_string(), "t2".to_string(), "t3".to_string()];
        let response = json!({"data": [
            {"status": "ok", "id": "x"},
            {"status": "error", "details": {"error": "DeviceNotRegistered"}},
            {"status": "error", "details": {"error": "MessageRateExceeded"}},
        ]});
        assert_eq!(unregistered(&tokens, &response), ["t2"]);
        assert!(unregistered(&tokens, &json!({"errors": []})).is_empty());
    }
}
