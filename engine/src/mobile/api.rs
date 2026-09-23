//! Answers the phone's October core requests with Lantern's agents.
//!
//! The phone speaks October core's API (`coreProtocol.ts`, api version 2). Lantern presents
//! one canvas ("Lantern") whose nodes are the agents it sees, and supports what the phone
//! needs to list them and reply; everything else is refused with PERMISSION_DENIED.

use hmac::{KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::model::{Agent, State};

use super::control::iso;

pub struct Context<'a> {
    pub agents: &'a [Agent],
    pub canvas_id: &'a str,
    pub host_id: &'a str,
    pub instance_id: &'a str,
    pub process_start: &'a str,
    pub credential: &'a str,
}

/// What the host must do after answering (typing a reply runs outside the handler).
pub enum Action {
    None,
    Deliver { agent_id: String, text: String },
}

/// A stable UUID for an agent's node (the phone expects UUID-shaped ids).
pub fn node_id(agent_id: &str) -> String {
    use sha2::Digest;
    let h = Sha256::digest(format!("lantern-node:{agent_id}").as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&h[..16]);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(b).hyphenated().to_string()
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
    let since = agent.state_since.unwrap_or_else(crate::hooks::now_ms);
    let name = match &agent.title {
        Some(t) => format!("@{} · {t}", agent.handle),
        None => format!("@{}", agent.handle),
    };
    let mut n = json!({
        "id": node_id(&agent.id), "kind": "terminal", "displayName": name, "harness": harness(agent),
        "cwd": agent.cwd, "status": if agent.state == State::Working { "live" } else { "idle" },
        "lastSeen": iso(crate::hooks::now_ms()), "createdAt": iso(since),
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

fn err(request_id: &Value, code: &str, message: &str) -> (u16, Value) {
    (400, json!({"apiVersion": 2, "requestId": request_id, "ok": false, "error": {"code": code, "message": message, "retryable": false}}))
}

pub fn proof(credential: &str, challenge: &str, instance_id: &str, process_start: &str) -> String {
    use base64::Engine;
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(credential.as_bytes()).expect("hmac key");
    mac.update(format!("{challenge}:{instance_id}:{process_start}:2").as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// Handles one request envelope. Returns (HTTP-like status, response envelope, follow-up).
pub fn handle(ctx: &Context, request: &Value) -> (u16, Value, Action) {
    let id = &request["requestId"];
    let payload = &request["payload"];
    let op = payload["operation"].as_str().unwrap_or("");
    let with = |(s, v): (u16, Value)| (s, v, Action::None);
    if request["apiVersion"] != 2 {
        return with(err(id, "INCOMPATIBLE_VERSION", "api version 2 is required"));
    }
    match request["method"].as_str().unwrap_or("") {
        "core.handshake" => {
            let (min, max) = (payload["apiMin"].as_i64().unwrap_or(0), payload["apiMax"].as_i64().unwrap_or(0));
            if min > 2 || max < 2 {
                return with(err(id, "INCOMPATIBLE_VERSION", "Lantern speaks api version 2"));
            }
            let challenge = payload["challenge"].as_str().unwrap_or("");
            with(ok(id, json!({
                "instanceId": ctx.instance_id, "processStart": ctx.process_start,
                "coreVersion": format!("lantern-{}", env!("CARGO_PKG_VERSION")), "apiVersion": 2,
                "generation": 1, "schemaVersion": 8,
                "challengeProof": proof(ctx.credential, challenge, ctx.instance_id, ctx.process_start),
            })))
        }
        "core.status" => {
            let attention = ctx.agents.iter().filter(|a| wants(a)).count();
            with(ok(id, json!({
                "coreVersion": format!("lantern-{}", env!("CARGO_PKG_VERSION")), "apiVersion": 2, "ready": true,
                "agentCount": ctx.agents.len(), "attentionCount": attention, "terminalCount": 0,
                "remote": {"desktopVisible": true, "openCanvasId": ctx.canvas_id},
            })))
        }
        "bus.query" => match op {
            "listCanvases" => with(ok(id, json!([ctx.canvas_id]))),
            "currentSnapshot" if payload["args"][0] == ctx.canvas_id => with(ok(id, snapshot(ctx))),
            "currentSnapshot" => with(err(id, "NOT_FOUND", "unknown canvas")),
            _ => with(err(id, "PERMISSION_DENIED", "not available in Lantern")),
        },
        "facts.query" => match op {
            "listNotifications" => with(ok(id, notifications(ctx))),
            "listNodeWorkflows" | "listPrObservations" => with(ok(id, json!([]))),
            _ => with(err(id, "PERMISSION_DENIED", "not available in Lantern")),
        },
        "facts.mutate" if op == "markNotificationSeen" => with(ok(id, json!({"ok": true}))),
        "ui.list" | "terminal.list" | "agent.list" | "devServer.list" | "chat.history" => with(ok(id, json!([]))),
        "bus.mutate" if op == "userSend" => {
            let node = payload["args"][1]["id"].as_str().unwrap_or("");
            let text = payload["args"][2].as_str().unwrap_or("").trim().to_string();
            match ctx.agents.iter().find(|a| node_id(&a.id) == node) {
                None => with(err(id, "NOT_FOUND", "that agent isn't running any more")),
                Some(_) if text.is_empty() || text.len() > 8000 => with(err(id, "INVALID_ARGUMENT", "message must be 1–8000 characters")),
                Some(a) if !a.can_reply => with(ok(id, json!({"accepted": false, "reason": "Lantern can't type into this terminal yet"}))),
                Some(a) => {
                    let (s, v) = ok(id, json!({"accepted": true, "delivery": "delivered"}));
                    (s, v, Action::Deliver { agent_id: a.id.clone(), text })
                }
            }
        }
        _ => with(err(id, "PERMISSION_DENIED", "not available in Lantern")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Route, StateSource};

    fn agent(state: State) -> Agent {
        Agent {
            id: "claude:42".into(), kind: Kind::Claude, handle: "claude-1".into(), pid: 42, tty: None,
            cwd: Some("/x".into()), project: Some("x".into()), title: Some("Fix tests".into()), session_id: None,
            state, state_since: Some(1_790_000_000_000), last_message: Some("Done.".into()), question: None,
            host: None, tmux: None, can_reply: true, route: Route::Tmux, state_source: StateSource::Transcript,
        }
    }

    fn req(method: &str, payload: Value) -> Value {
        json!({"apiVersion": 2, "requestId": "r1", "deadlineAt": 0, "principal": {"kind": "remote", "id": "b"}, "method": method, "payload": payload})
    }

    #[test]
    fn serves_the_phone_surface() {
        let agents = vec![agent(State::Waiting)];
        let ctx = Context { agents: &agents, canvas_id: "c", host_id: "h", instance_id: "i", process_start: "p", credential: "cred" };
        let (s, v, _) = handle(&ctx, &req("core.handshake", json!({"challenge": "abc", "clientVersion": "0.1.0", "apiMin": 2, "apiMax": 2})));
        assert_eq!(s, 200);
        assert_eq!(v["result"]["challengeProof"], proof("cred", "abc", "i", "p"));
        let (_, v, _) = handle(&ctx, &req("bus.query", json!({"operation": "listCanvases", "args": []})));
        assert_eq!(v["result"], json!(["c"]));
        let (_, v, _) = handle(&ctx, &req("bus.query", json!({"operation": "currentSnapshot", "args": ["c"]})));
        let n = &v["result"]["nodes"][node_id("claude:42")];
        assert_eq!((n["execution"]["state"].as_str(), n["attention"]["message"].as_str()), (Some("needs-user"), Some("Done.")));
        let (_, v, _) = handle(&ctx, &req("facts.query", json!({"operation": "listNotifications", "args": [{}]})));
        assert_eq!(v["result"].as_array().unwrap().len(), 1);
        let (_, v, action) = handle(&ctx, &req("bus.mutate", json!({"operation": "userSend", "args": ["c", {"id": node_id("claude:42"), "kind": "terminal"}, "yes"]})));
        assert_eq!(v["result"]["accepted"], true);
        assert!(matches!(action, Action::Deliver { .. }));
        let (s, v, _) = handle(&ctx, &req("terminal.kill", json!({})));
        assert_eq!((s, v["error"]["code"].as_str()), (400, Some("PERMISSION_DENIED")));
        let (_, v, _) = handle(&ctx, &req("core.handshake", json!({"challenge": "x", "apiMin": 3, "apiMax": 3})));
        assert_eq!(v["error"]["code"], "INCOMPATIBLE_VERSION");
    }
}
