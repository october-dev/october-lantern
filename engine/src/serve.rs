//! The stdio protocol the app talks to. See `protocol/README.md`.

use std::io::{BufRead, Write};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::hooks::now_ms;
use crate::launch;
use crate::model::{Agent, Kind, Route, State};
use crate::october_link::LinkState;
use std::sync::{Arc, Mutex, mpsc::Sender};
use crate::scanner::Scanner;
use crate::deliver;

const SCAN_EVERY: Duration = Duration::from_millis(1500);
const HEARTBEAT: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Request {
    Refresh,
    #[serde(rename_all = "camelCase")]
    Reply { request_id: String, agent_id: String, text: String },
    #[serde(rename_all = "camelCase")]
    Launch { request_id: String, kind: Kind, cwd: String, prompt: Option<String>, background: bool },
    #[serde(rename_all = "camelCase")]
    History { request_id: String, agent_id: String },
    /// Single keypresses, e.g. "1" or "Escape" to answer a permission prompt.
    #[serde(rename_all = "camelCase")]
    Keys { request_id: String, agent_id: String, keys: Vec<String> },
    /// Ask October Desktop to allow Lantern (it shows a code to compare), cancel that, or forget it.
    #[serde(rename = "october.pair")]
    OctoberPair,
    #[serde(rename = "october.cancelPair")]
    OctoberCancelPair,
    #[serde(rename = "october.forget")]
    OctoberForget,
    /// Bring the agent's own tab (or tmux pane) to the front.
    #[serde(rename_all = "camelCase")]
    Focus { request_id: String, agent_id: String },
    /// Show an agent that runs in a detached tmux session in a Terminal window.
    #[serde(rename_all = "camelCase")]
    Attach { request_id: String, agent_id: String },

    // MARK: Phone (see engine/src/mobile)
    /// Start acting as an October host for the phone app, with the user's October access token.
    #[serde(rename = "phone.start", rename_all = "camelCase")]
    PhoneStart { access_token: String },
    /// A refreshed access token.
    #[serde(rename = "phone.token", rename_all = "camelCase")]
    PhoneToken { access_token: String },
    #[serde(rename = "phone.pair")]
    PhonePair,
    #[serde(rename = "phone.decide")]
    PhoneDecide { allow: bool },
    #[serde(rename = "phone.revoke")]
    PhoneRevoke { bind: String },
    #[serde(rename = "phone.stop")]
    PhoneStop,
}

fn emit(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

pub fn run() -> Result<()> {
    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(l) => {
                    if tx.send(Some(l)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        // stdin closed: the app is gone.
        let _ = tx.send(None);
    });

    emit(&json!({"type": "hello", "protocol": 1, "version": env!("CARGO_PKG_VERSION")}));
    crate::hooks::refresh_hook_binary();
    // Checking installed agents runs a login shell, so do it off the main loop.
    std::thread::spawn(|| emit(&json!({"type": "installed", "installed": launch::installed()})));

    let (october, october_tx) = crate::october_link::start();
    let mut last_october = String::new();
    let mut scanner = Scanner::new();
    let mut agents: Vec<Agent> = Vec::new();
    let mut last_emit = Instant::now() - HEARTBEAT;
    let mut next_scan = Instant::now();
    let mut phone: Option<crate::mobile::host::MobileHost> = None;

    loop {
        if Instant::now() >= next_scan {
            let mut fresh = scanner.scan();
            if let Some(p) = &phone {
                p.update_agents(&fresh);
            }
            let link = october.lock().map(|s| s.clone()).unwrap_or_default();
            merge_october(&mut fresh, &link);
            let summary = serde_json::to_string(&link).unwrap_or_default();
            if summary != last_october {
                emit(&json!({"type": "october", "october": link}));
                last_october = summary;
            }
            if fresh != agents || last_emit.elapsed() >= HEARTBEAT {
                agents = fresh;
                emit(&json!({"type": "snapshot", "generatedAt": now_ms(), "agents": agents}));
                last_emit = Instant::now();
            }
            next_scan = Instant::now() + SCAN_EVERY;
        }

        let wait = next_scan.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(Some(line)) => match serde_json::from_str::<Request>(&line) {
                Ok(Request::Refresh) => next_scan = Instant::now(),
                Ok(Request::Reply { request_id, agent_id, text }) => {
                    let result = match agents.iter().find(|a| a.id == agent_id) {
                        None => Err(("unknown_agent", format!("no agent {agent_id}"))),
                        Some(a) if !a.can_reply => Err(("not_reachable", "Lantern can't type into this terminal yet".to_string())),
                        Some(a) if matches!(a.route, Route::October { .. }) => {
                            send_via_october(&october, &october_tx, a, &text).map_err(|m| ("send_failed", m))
                        }
                        Some(a) => deliver::send_text(a, &text).map_err(|e| ("send_failed", format!("{e:#}"))),
                    };
                    match result {
                        Ok(()) => emit(&json!({"type": "replyResult", "requestId": request_id, "ok": true})),
                        Err((code, message)) => emit(&json!({
                            "type": "replyResult", "requestId": request_id, "ok": false,
                            "error": code, "message": message
                        })),
                    }
                    next_scan = Instant::now() + Duration::from_millis(300);
                }
                Ok(Request::Launch { request_id, kind, cwd, prompt, background }) => {
                    let mode = if background { launch::Mode::Background } else { launch::Mode::Terminal };
                    match launch::launch(kind, std::path::Path::new(&cwd), prompt.as_deref(), mode) {
                        Ok(l) => emit(&json!({"type": "launchResult", "requestId": request_id, "ok": true, "session": l.session})),
                        Err(e) => emit(&json!({"type": "launchResult", "requestId": request_id, "ok": false, "message": format!("{e:#}")})),
                    }
                    next_scan = Instant::now() + Duration::from_millis(1500);
                }
                Ok(Request::History { request_id, agent_id }) => {
                    let messages = agents.iter().find(|a| a.id == agent_id).and_then(|a| scanner.history(a));
                    emit(&json!({
                        "type": "historyResult", "requestId": request_id, "agentId": agent_id,
                        "supported": messages.is_some(), "messages": messages.unwrap_or_default()
                    }));
                }
                Ok(Request::Keys { request_id, agent_id, keys }) => {
                    let result = match agents.iter().find(|a| a.id == agent_id) {
                        None => Err(format!("no agent {agent_id}")),
                        Some(a) => keys.iter().try_for_each(|k| {
                            let key = deliver::Key::parse(k).ok_or_else(|| anyhow::anyhow!("unknown key {k}"))?;
                            std::thread::sleep(Duration::from_millis(40));
                            deliver::send_key(a, key)
                        })
                        .map_err(|e| format!("{e:#}")),
                    };
                    emit(&json!({"type": "replyResult", "requestId": request_id, "ok": result.is_ok(), "message": result.err()}));
                    next_scan = Instant::now() + Duration::from_millis(300);
                }
                Ok(Request::OctoberPair) => { let _ = october_tx.send(crate::october_link::Command::Pair); }
                Ok(Request::OctoberCancelPair) => { let _ = october_tx.send(crate::october_link::Command::CancelPair); }
                Ok(Request::OctoberForget) => { let _ = october_tx.send(crate::october_link::Command::Forget); }
                Ok(Request::Focus { request_id, agent_id }) => {
                    if let Some(oa) = agents.iter().find(|a| a.id == agent_id).and_then(|a| october_agent(&october, a)) {
                        let _ = october_tx.send(crate::october_link::Command::Focus { agent: oa });
                    }
                    let result = match agents.iter().find(|a| a.id == agent_id) {
                        None => Err(format!("no agent {agent_id}")),
                        Some(a) => deliver::focus(a).map_err(|e| format!("{e:#}")),
                    };
                    emit(&json!({"type": "attachResult", "requestId": request_id, "ok": result.is_ok(), "message": result.err()}));
                }
                Ok(Request::Attach { request_id, agent_id }) => {
                    let result = match agents.iter().find(|a| a.id == agent_id).and_then(|a| a.tmux.as_ref()) {
                        Some(pane) => launch::attach_in_terminal(pane).map_err(|e| format!("{e:#}")),
                        None => Err("agent is not in a tmux session".to_string()),
                    };
                    emit(&json!({"type": "attachResult", "requestId": request_id, "ok": result.is_ok(), "message": result.err()}));
                }
                // MARK: Phone
                Ok(Request::PhoneStart { access_token }) => match &phone {
                    Some(p) => p.send(crate::mobile::host::Cmd::Token(access_token)),
                    None => {
                        let p = crate::mobile::host::MobileHost::start(access_token, emit);
                        p.update_agents(&agents);
                        phone = Some(p);
                    }
                },
                Ok(Request::PhoneToken { access_token }) => {
                    if let Some(p) = &phone {
                        p.send(crate::mobile::host::Cmd::Token(access_token));
                    }
                }
                Ok(Request::PhonePair) => {
                    if let Some(p) = &phone {
                        p.send(crate::mobile::host::Cmd::Pair);
                    }
                }
                Ok(Request::PhoneDecide { allow }) => {
                    if let Some(p) = &phone {
                        p.send(crate::mobile::host::Cmd::Decide(allow));
                    }
                }
                Ok(Request::PhoneRevoke { bind }) => {
                    if let Some(p) = &phone {
                        p.send(crate::mobile::host::Cmd::Revoke(bind));
                    }
                }
                Ok(Request::PhoneStop) => {
                    if let Some(p) = phone.take() {
                        p.send(crate::mobile::host::Cmd::Stop);
                    }
                }
                Err(e) => eprintln!("lantern-engine: bad request {line:?}: {e}"),
            },
            Ok(None) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// October's view of a Lantern agent that runs inside October Desktop, matched by folder and
/// harness (October's terminals are ordinary processes Lantern also sees).
fn match_october<'a>(link: &'a LinkState, a: &Agent) -> Option<&'a crate::october_core::OctoberAgent> {
    if a.host.as_ref().is_none_or(|h| h.app != "October") {
        return None;
    }
    let cwd = a.cwd.as_deref()?;
    let same_dir: Vec<_> = link.agents.iter().filter(|o| o.cwd.as_deref() == Some(cwd)).collect();
    let by_harness: Vec<_> = same_dir
        .iter()
        .filter(|o| o.harness.as_deref().is_some_and(|h| h.contains(a.kind.as_str())))
        .copied()
        .collect();
    match (by_harness.len(), same_dir.len()) {
        (1, _) => Some(by_harness[0]),
        (0, 1) => Some(same_dir[0]),
        _ => None,
    }
}

fn october_agent(link: &Arc<Mutex<LinkState>>, a: &Agent) -> Option<crate::october_core::OctoberAgent> {
    let link = link.lock().ok()?;
    match_october(&link, a).cloned()
}

/// When Lantern is paired with October, agents inside October get their October name and state,
/// and replies go through October's safe delivery.
fn merge_october(agents: &mut [Agent], link: &LinkState) {
    if link.agents.is_empty() {
        return;
    }
    for a in agents.iter_mut() {
        let Some(o) = match_october(link, a).cloned() else { continue };
        if a.title.is_none() {
            a.title = o.name.clone();
        }
        if o.state == "needs-user" {
            a.state = State::NeedsInput;
            a.question = o.attention.clone().or(a.question.take());
        }
        if link.paired {
            a.route = Route::October { canvas_id: o.canvas_id.clone(), node_id: o.node_id.clone() };
            a.can_reply = true;
        }
    }
}

fn send_via_october(link: &Arc<Mutex<LinkState>>, tx: &Sender<crate::october_link::Command>, a: &Agent, text: &str) -> Result<(), String> {
    let agent = october_agent(link, a).ok_or("October no longer lists this agent")?;
    let (done, wait) = std::sync::mpsc::channel();
    tx.send(crate::october_link::Command::Send { agent, text: text.to_string(), done }).map_err(|_| "October link stopped")?;
    wait.recv_timeout(Duration::from_secs(15)).map_err(|_| "October didn't answer in time".to_string())?.map(|_| ())
}
