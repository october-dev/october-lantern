//! Answers the phone's October core requests with Lantern's agents.
//!
//! The phone speaks October core's API (`coreProtocol.ts`, api version 2). Lantern presents
//! one canvas ("Lantern") whose nodes are the agents it sees, and supports what the phone
//! needs to list them and reply; everything else is refused with PERMISSION_DENIED.

use hmac::{KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::actions::Outcome;
use crate::hooks::now_ms;
use crate::model::{Agent, State, iso};

pub struct Context<'a> {
    pub agents: &'a [Agent],
    pub canvas_id: &'a str,
    pub host_id: &'a str,
    pub instance_id: &'a str,
    pub process_start: &'a str,
    pub credential: &'a str,
}

/// Types a reply into an agent, if typing can start before the deadline (epoch ms).
pub type Deliver<'a> = &'a mut dyn FnMut(&Agent, &str, Option<u64>) -> Outcome;

/// A stable UUID for an agent's node (the phone expects UUID-shaped ids).
pub fn node_id(agent_id: &str) -> String {
    use sha2::Digest;
    let h = Sha256::digest(format!("lantern-node:{agent_id}").as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&h[..16]);
    uuid::Builder::from_random_bytes(b).into_uuid().hyphenated().to_string()
}

fn harness(agent: &Agent) -> &'static str {
    match agent.kind.as_str() {
        "claude" => "claude-code",
        other => other,
    }
}

/// Waiting on the person: finished its turn, or blocked on a question.
fn wants(agent: &Agent) -> bool {
    matches!(agent.state, State::Waiting | State::NeedsInput)
}

fn exec_state(agent: &Agent) -> &'static str {
    match agent.state {
        State::Working => "working",
        State::NeedsInput | State::Waiting => "needs-user",
        _ => "idle",
    }
}

pub fn node(agent: &Agent, host_id: &str) -> Value {
    let since = agent.state_since.unwrap_or_else(now_ms);
    let name = match &agent.title {
        Some(t) => format!("@{} · {t}", agent.handle),
        None => format!("@{}", agent.handle),
    };
    let mut n = json!({
        "id": node_id(&agent.id), "kind": "terminal", "displayName": name, "harness": harness(agent),
        "cwd": agent.cwd, "status": if agent.state == State::Working { "live" } else { "idle" },
        "lastSeen": iso(now_ms()), "createdAt": iso(since),
        "execution": {"ownerDeviceId": host_id, "revision": since, "state": exec_state(agent)},
    });
    if wants(agent) {
        let message = agent.question.clone().or_else(|| agent.last_message.clone()).unwrap_or_else(|| "Your turn".into());
        n["attention"] = json!({"message": message, "at": iso(since), "deliveredTo": []});
    }
    n
}

fn notifications(ctx: &Context) -> Value {
    ctx.agents
        .iter()
        .filter(|a| wants(a))
        .map(|a| {
            let at = a.state_since.unwrap_or(0);
            let title = if a.state == State::NeedsInput { format!("@{} needs you", a.handle) } else { format!("@{} finished", a.handle) };
            json!({
                "id": format!("{}-{at}", node_id(&a.id)), "type": "needs_input", "title": title,
                "body": a.question.clone().or_else(|| a.last_message.clone()).unwrap_or_default(),
                "createdAt": iso(at), "canvasId": ctx.canvas_id, "nodeId": node_id(&a.id), "sessionKind": "terminal"
            })
        })
        .collect()
}

pub fn snapshot(ctx: &Context) -> Value {
    let nodes: serde_json::Map<String, Value> = ctx.agents.iter().map(|a| (node_id(&a.id), node(a, ctx.host_id))).collect();
    json!({"canvasId": ctx.canvas_id, "nodes": nodes, "edges": [], "tasks": [], "messages": [], "summaries": {}})
}

fn ok(request_id: &Value, result: Value) -> (u16, Value) {
    (200, json!({"apiVersion": 2, "requestId": request_id, "ok": true, "result": result}))
}

pub(super) fn err(request_id: &Value, code: &str, message: &str) -> (u16, Value) {
    (400, json!({"apiVersion": 2, "requestId": request_id, "ok": false, "error": {"code": code, "message": message, "retryable": false}}))
}

pub(super) fn delivery_result(id: &Value, outcome: Outcome) -> (u16, Value) {
    match outcome {
        Outcome::Done => ok(id, json!({"accepted": true, "delivery": "delivered"})),
        Outcome::Queued => ok(id, json!({"accepted": true, "delivery": "queued"})),
        Outcome::Failed { message, .. } => ok(id, json!({"accepted": false, "reason": message})),
        Outcome::Uncertain(_) => ok(id, json!({"accepted": true, "delivery": "queued", "reason": "not-confirmed-check-the-terminal"})),
    }
}

/// Answers already given to requests with an idempotency key, so a retried mutation is answered
/// again without being performed again. Remembers the last `cap` keys, for as long as the host
/// runs: a retry after that (or after Lantern restarts) is treated as a new request.
pub struct Answers {
    cap: usize,
    entries: std::collections::VecDeque<(String, String, u16, Value)>,
}

impl Answers {
    pub fn new(cap: usize) -> Answers {
        Answers { cap, entries: Default::default() }
    }

    /// What the request is, for telling a retry from a different request under the same key.
    fn fingerprint(request: &Value) -> String {
        use sha2::Digest;
        let text = format!("{}|{}", request["method"], request["payload"]);
        Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The answer for a retry, re-addressed to this request's id; an error when the key was used
    /// for a different request; `None` for a key not seen.
    pub fn replay(&self, key: &str, request: &Value) -> Option<(u16, Value)> {
        let (_, print, status, body) = self.entries.iter().find(|(k, ..)| k == key)?;
        let id = &request["requestId"];
        if *print != Self::fingerprint(request) {
            return Some(err(id, "INVALID_ARGUMENT", "this idempotency key was already used for a different request"));
        }
        let mut body = body.clone();
        body["requestId"] = id.clone();
        Some((*status, body))
    }

    pub fn remember(&mut self, key: String, request: &Value, status: u16, body: Value) {
        if self.entries.len() >= self.cap {
            self.entries.pop_front();
        }
        self.entries.push_back((key, Self::fingerprint(request), status, body));
    }
}

pub fn proof(credential: &str, challenge: &str, instance_id: &str, process_start: &str) -> String {
    use base64::Engine;
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(credential.as_bytes()).expect("hmac key");
    mac.update(format!("{challenge}:{instance_id}:{process_start}:2").as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// Handles one request envelope. Returns (HTTP-like status, response envelope). A `userSend` is
/// delivered before it is answered, so the answer says what actually happened.
pub fn handle(ctx: &Context, request: &Value, deliver: Deliver) -> (u16, Value) {
    let id = &request["requestId"];
    let payload = &request["payload"];
    let op = payload["operation"].as_str().unwrap_or("");
    if request["apiVersion"] != 2 {
        return err(id, "INCOMPATIBLE_VERSION", "api version 2 is required");
    }
    // `deadlineAt` is epoch ms; 0 or absent means none.
    let deadline = match &request["deadlineAt"] {
        Value::Null => None,
        v => match v.as_u64() {
            Some(0) => None,
            Some(d) => Some(d),
            None => return err(id, "INVALID_ARGUMENT", "deadlineAt must be a number of milliseconds"),
        },
    };
    if deadline.is_some_and(|d| d < now_ms()) {
        return err(id, "DEADLINE_EXCEEDED", "the request expired before Lantern handled it");
    }
    match request["method"].as_str().unwrap_or("") {
        "core.handshake" => {
            let (min, max) = (payload["apiMin"].as_i64().unwrap_or(0), payload["apiMax"].as_i64().unwrap_or(0));
            if min > 2 || max < 2 {
                return err(id, "INCOMPATIBLE_VERSION", "Lantern speaks api version 2");
            }
            let challenge = payload["challenge"].as_str().unwrap_or("");
            ok(
                id,
                json!({
                    "instanceId": ctx.instance_id, "processStart": ctx.process_start,
                    "coreVersion": format!("lantern-{}", env!("CARGO_PKG_VERSION")), "apiVersion": 2,
                    "generation": 1, "schemaVersion": 8,
                    "challengeProof": proof(ctx.credential, challenge, ctx.instance_id, ctx.process_start),
                }),
            )
        }
        "core.status" => {
            let attention = ctx.agents.iter().filter(|a| wants(a)).count();
            ok(
                id,
                json!({
                    "coreVersion": format!("lantern-{}", env!("CARGO_PKG_VERSION")), "apiVersion": 2, "ready": true,
                    "agentCount": ctx.agents.len(), "attentionCount": attention, "terminalCount": 0,
                    "remote": {"desktopVisible": true, "openCanvasId": ctx.canvas_id},
                }),
            )
        }
        "bus.query" => match op {
            "listCanvases" => ok(id, json!([ctx.canvas_id])),
            "currentSnapshot" if payload["args"][0] == ctx.canvas_id => ok(id, snapshot(ctx)),
            "currentSnapshot" => err(id, "NOT_FOUND", "unknown canvas"),
            _ => err(id, "PERMISSION_DENIED", "not available in Lantern"),
        },
        "facts.query" => match op {
            "listNotifications" => ok(id, notifications(ctx)),
            "listNodeWorkflows" | "listPrObservations" => ok(id, json!([])),
            _ => err(id, "PERMISSION_DENIED", "not available in Lantern"),
        },
        "facts.mutate" if op == "markNotificationSeen" => ok(id, json!({"ok": true})),
        "ui.list" | "terminal.list" | "agent.list" | "devServer.list" | "chat.history" => ok(id, json!([])),
        "bus.mutate" if op == "userSend" => {
            if payload["args"][0] != ctx.canvas_id {
                return err(id, "NOT_FOUND", "unknown canvas");
            }
            let args = &payload["args"];
            let (Some(node), Some(text)) = (args[1]["id"].as_str(), args[2].as_str()) else {
                return err(id, "INVALID_ARGUMENT", "userSend takes a canvas, a node and a message");
            };
            if args[1]["kind"] != "terminal" {
                return err(id, "INVALID_ARGUMENT", "Lantern's agents are terminal nodes");
            }
            // Optional 4th argument: how long the message stays worth typing.
            let expires = match &args[3] {
                Value::Null => None,
                v => match v.as_u64() {
                    Some(ms) => Some(now_ms() + ms),
                    None => return err(id, "INVALID_ARGUMENT", "expiresInMs must be a number of milliseconds"),
                },
            };
            let deadline = [deadline, expires].into_iter().flatten().min();
            let text = text.trim().to_string();
            match ctx.agents.iter().find(|a| node_id(&a.id) == node) {
                None => err(id, "NOT_FOUND", "that agent isn't running any more"),
                Some(_) if text.is_empty() || text.chars().count() > 8000 => {
                    err(id, "INVALID_ARGUMENT", "message must be 1–8000 characters")
                }
                Some(a) if !a.can_reply => ok(id, json!({"accepted": false, "reason": "Lantern can't type into this terminal yet"})),
                Some(a) => delivery_result(id, deliver(a, &text, deadline)),
            }
        }
        _ => err(id, "PERMISSION_DENIED", "not available in Lantern"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Route, StateSource};

    fn agent(state: State) -> Agent {
        Agent {
            id: "claude:42:1".into(),
            kind: Kind::Claude,
            handle: "claude-1".into(),
            pid: 42,
            start_time: 1,
            exe: None,
            comm: String::new(),
            tty: None,
            cwd: Some("/x".into()),
            project: Some("x".into()),
            title: Some("Fix tests".into()),
            session_id: None,
            state,
            state_since: Some(1_790_000_000_000),
            last_message: Some("Done.".into()),
            question: None,
            question_kind: None,
            question_detail: None,
            prompt_id: None,
            session_match: crate::model::SessionMatch::Exact,
            source: None,
            live: true,
            host: None,
            tmux: None,
            can_reply: true,
            route: Route::Tmux,
            state_source: StateSource::Transcript,
        }
    }

    fn req(method: &str, payload: Value) -> Value {
        json!({"apiVersion": 2, "requestId": "r1", "deadlineAt": 0, "principal": {"kind": "remote", "id": "b"}, "method": method, "payload": payload})
    }

    #[test]
    fn serves_the_phone_surface() {
        let agents = vec![agent(State::Waiting)];
        let ctx = Context { agents: &agents, canvas_id: "c", host_id: "h", instance_id: "i", process_start: "p", credential: "cred" };
        let typed: std::cell::RefCell<Vec<(String, String)>> = Default::default();
        let mut deliver = |a: &Agent, t: &str, _: Option<u64>| {
            typed.borrow_mut().push((a.id.clone(), t.to_string()));
            Outcome::Done
        };
        let (s, v) = handle(
            &ctx,
            &req("core.handshake", json!({"challenge": "abc", "clientVersion": "0.1.0", "apiMin": 2, "apiMax": 2})),
            &mut deliver,
        );
        assert_eq!(s, 200);
        assert_eq!(v["result"]["challengeProof"], proof("cred", "abc", "i", "p"));
        let (_, v) = handle(&ctx, &req("bus.query", json!({"operation": "listCanvases", "args": []})), &mut deliver);
        assert_eq!(v["result"], json!(["c"]));
        let (_, v) = handle(&ctx, &req("bus.query", json!({"operation": "currentSnapshot", "args": ["c"]})), &mut deliver);
        let n = &v["result"]["nodes"][node_id("claude:42:1")];
        assert_eq!((n["execution"]["state"].as_str(), n["attention"]["message"].as_str()), (Some("needs-user"), Some("Done.")));
        let (_, v) = handle(&ctx, &req("facts.query", json!({"operation": "listNotifications", "args": [{}]})), &mut deliver);
        assert_eq!(v["result"].as_array().unwrap().len(), 1);
        let send = |canvas: &str| {
            req("bus.mutate", json!({"operation": "userSend", "args": [canvas, {"id": node_id("claude:42:1"), "kind": "terminal"}, "yes"]}))
        };
        let (_, v) = handle(&ctx, &send("c"), &mut deliver);
        assert_eq!(v["result"]["accepted"], true);
        assert_eq!(*typed.borrow(), [("claude:42:1".to_string(), "yes".to_string())]);
        let (_, v) = handle(&ctx, &send("other-canvas"), &mut deliver);
        assert_eq!(v["error"]["code"], "NOT_FOUND");
        let (s, v) = handle(&ctx, &req("terminal.kill", json!({})), &mut deliver);
        assert_eq!((s, v["error"]["code"].as_str()), (400, Some("PERMISSION_DENIED")));
        let (_, v) = handle(&ctx, &req("core.handshake", json!({"challenge": "x", "apiMin": 3, "apiMax": 3})), &mut deliver);
        assert_eq!(v["error"]["code"], "INCOMPATIBLE_VERSION");
    }

    #[test]
    fn expired_requests_and_failed_delivery_are_reported() {
        let agents = vec![agent(State::Waiting)];
        let ctx = Context { agents: &agents, canvas_id: "c", host_id: "h", instance_id: "i", process_start: "p", credential: "cred" };
        let mut failing = |_: &Agent, _: &str, _: Option<u64>| Outcome::failed("send_failed", "the agent has exited");
        let mut expired = req("core.status", json!({}));
        expired["deadlineAt"] = json!(now_ms() - 1);
        let (_, v) = handle(&ctx, &expired, &mut failing);
        assert_eq!(v["error"]["code"], "DEADLINE_EXCEEDED");
        let send =
            req("bus.mutate", json!({"operation": "userSend", "args": ["c", {"id": node_id("claude:42:1"), "kind": "terminal"}, "yes"]}));
        let (_, v) = handle(&ctx, &send, &mut failing);
        assert_eq!((v["result"]["accepted"].as_bool(), v["result"]["reason"].as_str()), (Some(false), Some("the agent has exited")));
    }

    #[test]
    fn malformed_mutations_are_refused_and_deadlines_reach_delivery() {
        let agents = vec![agent(State::Waiting)];
        let ctx = Context { agents: &agents, canvas_id: "c", host_id: "h", instance_id: "i", process_start: "p", credential: "cred" };
        let calls: std::cell::RefCell<Vec<Option<u64>>> = Default::default();
        let mut deliver = |_: &Agent, _: &str, d: Option<u64>| {
            calls.borrow_mut().push(d);
            Outcome::Uncertain("gave up".into())
        };
        let send = |kind: &str, extra: Value| {
            let mut args = vec![json!("c"), json!({"id": node_id("claude:42:1"), "kind": kind}), json!("yes")];
            if !extra.is_null() {
                args.push(extra);
            }
            req("bus.mutate", json!({"operation": "userSend", "args": args}))
        };
        let mut bad_deadline = send("terminal", Value::Null);
        bad_deadline["deadlineAt"] = json!("yesterday");
        assert_eq!(handle(&ctx, &bad_deadline, &mut deliver).1["error"]["code"], "INVALID_ARGUMENT");
        assert_eq!(handle(&ctx, &send("sticky", Value::Null), &mut deliver).1["error"]["code"], "INVALID_ARGUMENT");
        assert_eq!(handle(&ctx, &send("terminal", json!("soon")), &mut deliver).1["error"]["code"], "INVALID_ARGUMENT");
        assert!(calls.borrow().is_empty(), "nothing malformed reaches delivery");
        // An uncertain outcome must not read as "not accepted" (the phone would offer a retry).
        let (_, v) = handle(&ctx, &send("terminal", json!(5000)), &mut deliver);
        assert_eq!((v["result"]["accepted"].as_bool(), v["result"]["delivery"].as_str()), (Some(true), Some("queued")));
        let d = calls.borrow()[0].expect("expiresInMs becomes a deadline");
        assert!(d > now_ms() && d <= now_ms() + 5000);
    }

    #[test]
    fn retries_replay_the_answer_under_their_own_id() {
        let mut answers = Answers::new(2);
        let mut first = req("bus.mutate", json!({"operation": "userSend", "args": ["c", {"id": "n", "kind": "terminal"}, "yes"]}));
        first["requestId"] = json!("first");
        answers.remember(
            "b|k".into(),
            &first,
            200,
            json!({"apiVersion": 2, "requestId": "first", "ok": true, "result": {"accepted": true}}),
        );
        let mut retry = first.clone();
        retry["requestId"] = json!("retry");
        let (status, body) = answers.replay("b|k", &retry).unwrap();
        assert_eq!((status, body["requestId"].as_str(), body["result"]["accepted"].as_bool()), (200, Some("retry"), Some(true)));
        let mut other = retry.clone();
        other["payload"]["args"][2] = json!("no");
        assert_eq!(answers.replay("b|k", &other).unwrap().1["error"]["code"], "INVALID_ARGUMENT");
        assert!(answers.replay("b|other", &retry).is_none());
    }
}
