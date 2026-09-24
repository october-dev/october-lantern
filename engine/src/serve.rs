//! The stdio protocol the app talks to. See `protocol/README.md`.

use std::io::{BufRead, Write};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deliver;
use crate::hooks::now_ms;
use crate::launch;
use crate::model::{Agent, Kind, Route};
use crate::october_link::{self, Link};
use crate::scanner::Scanner;

const SCAN_EVERY: Duration = Duration::from_millis(1500);
const HEARTBEAT: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Request {
    Refresh,
    #[serde(rename_all = "camelCase")]
    Reply {
        request_id: String,
        agent_id: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Launch {
        request_id: String,
        kind: Kind,
        cwd: String,
        prompt: Option<String>,
        background: bool,
    },
    #[serde(rename_all = "camelCase")]
    History {
        request_id: String,
        agent_id: String,
    },
    /// Single keypresses, e.g. "1" or "Escape" to answer a permission prompt.
    #[serde(rename_all = "camelCase")]
    Keys {
        request_id: String,
        agent_id: String,
        keys: Vec<String>,
    },
    /// Ask October Desktop to allow Lantern (it shows a code to compare), cancel that, or forget it.
    #[serde(rename = "october.pair")]
    OctoberPair,
    #[serde(rename = "october.cancelPair")]
    OctoberCancelPair,
    #[serde(rename = "october.forget")]
    OctoberForget,
    /// Bring the agent's own tab (or tmux pane) to the front.
    #[serde(rename_all = "camelCase")]
    Focus {
        request_id: String,
        agent_id: String,
    },
    /// Show an agent that runs in a detached tmux session in a Terminal window.
    #[serde(rename_all = "camelCase")]
    Attach {
        request_id: String,
        agent_id: String,
    },

    // MARK: Phone (see engine/src/mobile)
    /// The user's current October access token: starts hosting for the phone app, or hands a
    /// running host the refreshed token.
    #[serde(rename = "phone.token", rename_all = "camelCase")]
    PhoneToken {
        access_token: String,
    },
    #[serde(rename = "phone.pair")]
    PhonePair,
    #[serde(rename = "phone.decide")]
    PhoneDecide {
        allow: bool,
    },
    #[serde(rename = "phone.revoke")]
    PhoneRevoke {
        bind: String,
    },
    #[serde(rename = "phone.stop")]
    PhoneStop,
}

/// A reply from the phone, typed by the serve loop like a reply from the app.
pub struct Deliver {
    pub agent_id: String,
    pub text: String,
    pub done: Sender<Result<(), String>>,
}

/// Everything the serve loop reacts to.
pub enum Incoming {
    Line(String),
    /// stdin closed: the app is gone.
    Closed,
    Deliver(Deliver),
}

fn emit(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

pub fn run() -> Result<()> {
    let (tx, rx) = mpsc::channel::<Incoming>();
    let stdin_tx = tx.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(l) => {
                    if stdin_tx.send(Incoming::Line(l)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = stdin_tx.send(Incoming::Closed);
    });

    emit(&json!({"type": "hello", "protocol": 2, "version": env!("CARGO_PKG_VERSION")}));
    crate::hooks::refresh_hook_binary();
    // Checking installed agents runs a login shell, so do it off the main loop.
    std::thread::spawn(|| emit(&json!({"type": "installed", "installed": launch::installed()})));

    let link = october_link::start();
    let mut last_october = String::new();
    let mut scanner = Scanner::new();
    let mut agents: Vec<Agent> = Vec::new();
    let mut last_emit = Instant::now() - HEARTBEAT;
    let mut next_scan = Instant::now();
    let mut phone: Option<crate::mobile::host::MobileHost> = None;

    loop {
        if Instant::now() >= next_scan {
            let mut fresh = scanner.scan();
            let october = link.snapshot();
            october_link::merge(&mut fresh, &october);
            if let Some(p) = &phone {
                p.update_agents(&fresh);
            }
            let summary = serde_json::to_string(&october).unwrap_or_default();
            if summary != last_october {
                emit(&json!({"type": "october", "october": october}));
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
            Ok(Incoming::Deliver(d)) => {
                let result = deliver_to(&agents, &link, &d.agent_id, &d.text).map_err(|(_, m)| m);
                let _ = d.done.send(result);
                next_scan = Instant::now() + Duration::from_millis(300);
            }
            Ok(Incoming::Line(line)) => match serde_json::from_str::<Request>(&line) {
                Ok(Request::Refresh) => next_scan = Instant::now(),
                Ok(Request::Reply { request_id, agent_id, text }) => {
                    match deliver_to(&agents, &link, &agent_id, &text) {
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
                        Some(a) => keys
                            .iter()
                            .try_for_each(|k| {
                                let key = deliver::Key::parse(k).ok_or_else(|| anyhow::anyhow!("unknown key {k}"))?;
                                std::thread::sleep(Duration::from_millis(40));
                                deliver::send_key(a, key)
                            })
                            .map_err(|e| format!("{e:#}")),
                    };
                    emit(&json!({"type": "replyResult", "requestId": request_id, "ok": result.is_ok(), "message": result.err()}));
                    next_scan = Instant::now() + Duration::from_millis(300);
                }
                Ok(Request::OctoberPair) => link.send(october_link::Command::Pair),
                Ok(Request::OctoberCancelPair) => link.send(october_link::Command::CancelPair),
                Ok(Request::OctoberForget) => link.send(october_link::Command::Forget),
                Ok(Request::Focus { request_id, agent_id }) => {
                    let result = match agents.iter().find(|a| a.id == agent_id) {
                        None => Err(format!("no agent {agent_id}")),
                        Some(a) => match link.focus(a) {
                            Ok(true) => Ok(()),
                            Ok(false) => deliver::focus(a).map_err(|e| format!("{e:#}")),
                            Err(e) => Err(e),
                        },
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
                Ok(Request::PhoneToken { access_token }) => match &phone {
                    Some(p) => p.send(crate::mobile::host::Cmd::Token(access_token)),
                    None => {
                        let p = crate::mobile::host::MobileHost::start(access_token, emit, tx.clone());
                        p.update_agents(&agents);
                        phone = Some(p);
                    }
                },
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
                // The line itself isn't logged: some requests carry an access token.
                Err(e) => eprintln!("lantern-engine: bad request ({e})"),
            },
            Ok(Incoming::Closed) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// The one place replies are typed, whoever asked (the app or a phone): picks the route and
/// checks the target is still the agent Lantern saw.
fn deliver_to(agents: &[Agent], link: &Link, agent_id: &str, text: &str) -> Result<(), (&'static str, String)> {
    match agents.iter().find(|a| a.id == agent_id) {
        None => Err(("unknown_agent", format!("no agent {agent_id}"))),
        Some(a) if !a.can_reply => Err(("not_reachable", "Lantern can't type into this terminal yet".to_string())),
        Some(a) if matches!(a.route, Route::October { .. }) => link.send_text(a, text).map_err(|m| ("send_failed", m)),
        Some(a) => deliver::send_text(a, text).map_err(|e| ("send_failed", format!("{e:#}"))),
    }
}
