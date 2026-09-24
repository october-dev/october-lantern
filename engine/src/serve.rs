//! The stdio protocol the app talks to. See `protocol/README.md`.

use std::io::{BufRead, Write};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::actions::{Action, Executor, Op, Outcome, Ticket};
use crate::deliver;
use crate::hooks::now_ms;
use crate::launch;
use crate::model::{Agent, Kind, QuestionKind, Route};
use crate::october_link;
use crate::scanner::Scanner;

const SCAN_EVERY: Duration = Duration::from_millis(1500);
const HEARTBEAT: Duration = Duration::from_secs(10);
/// Typing for the app must start within this (the app waits 60 s for the whole outcome).
const APP_DEADLINE: Duration = Duration::from_secs(15);

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
        /// A screenshot the app saved for this session (in the screenshots folder).
        screenshot: Option<String>,
        /// A model id for the agent's `--model`; absent for the agent's own default.
        model: Option<String>,
        /// What the person is working in (a task started from an app), added to the first message.
        context: Option<String>,
        /// Add the toolkit list to the first message.
        #[serde(default)]
        toolkit: bool,
        background: bool,
    },
    /// Rebuild the toolkit list now (answered with `toolkit`).
    #[serde(rename = "toolkit.refresh")]
    ToolkitRefresh,
    /// The models to offer when starting `kind` (answered with `models`).
    Models {
        kind: Kind,
    },
    #[serde(rename_all = "camelCase")]
    History {
        request_id: String,
        agent_id: String,
    },
    /// Single keypresses, e.g. "1" or "Escape" to answer a permission prompt. `promptId` names the
    /// prompt they answer; keys for a permission prompt without it, or for one that has since
    /// changed, are refused.
    #[serde(rename_all = "camelCase")]
    Keys {
        request_id: String,
        agent_id: String,
        keys: Vec<String>,
        prompt_id: Option<String>,
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
    #[serde(rename = "phone.cancelPair")]
    PhoneCancelPair,
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

/// A reply from the phone, typed like a reply from the app. The phone host keeps `ticket` to
/// cancel it if it gives up waiting.
pub struct Deliver {
    pub agent_id: String,
    pub text: String,
    pub deadline: Instant,
    pub ticket: Arc<Ticket>,
    pub done: Sender<Outcome>,
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

    emit(&json!({"type": "hello", "protocol": 3, "version": env!("CARGO_PKG_VERSION")}));
    crate::hooks::refresh_hook_binary();
    // Checking installed agents runs a login shell, so do it off the main loop.
    std::thread::spawn(|| emit(&json!({"type": "installed", "installed": launch::installed()})));
    // The toolkit list is kept, not rebuilt before each start: refresh it now if it's old.
    std::thread::spawn(crate::toolkit::refresh_if_stale);

    let link = october_link::start();
    let mut executor = Executor::new(link.clone());
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
                // App sessions that aren't running have nothing to reply to.
                p.update_agents(&fresh.iter().filter(|a| a.live).cloned().collect::<Vec<_>>());
            }
            let summary = serde_json::to_string(&october).unwrap_or_default();
            if summary != last_october {
                emit(&json!({"type": "october", "october": october}));
                last_october = summary;
            }
            if fresh != agents || last_emit.elapsed() >= HEARTBEAT {
                agents = fresh;
                executor.retain(&agents);
                emit(&json!({"type": "snapshot", "generatedAt": now_ms(), "agents": agents}));
                last_emit = Instant::now();
            }
            next_scan = Instant::now() + SCAN_EVERY;
        }

        let wait = next_scan.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(Incoming::Deliver(d)) => {
                let done = d.done;
                match reachable(&agents, &d.agent_id) {
                    Ok(agent) => executor.submit(Action {
                        agent: agent.clone(),
                        op: Op::Text(d.text),
                        deadline: d.deadline,
                        ticket: d.ticket,
                        done: Box::new(move |o| {
                            let _ = done.send(o);
                        }),
                    }),
                    Err(o) => {
                        let _ = done.send(o);
                    }
                }
                next_scan = Instant::now() + Duration::from_millis(300);
            }
            Ok(Incoming::Line(line)) => match serde_json::from_str::<Request>(&line) {
                Ok(Request::Refresh) => next_scan = Instant::now(),
                Ok(Request::Reply { request_id, agent_id, text }) => {
                    match reachable(&agents, &agent_id) {
                        Ok(agent) => executor.submit(app_action(agent, Op::Text(text), "replyResult", request_id)),
                        Err(o) => emit(&result_json("replyResult", &request_id, &o)),
                    }
                    next_scan = Instant::now() + Duration::from_millis(300);
                }
                Ok(Request::Launch { request_id, kind, cwd, prompt, screenshot, model, context, toolkit, background }) => {
                    // Off the loop: it may wait for the agent to start before typing its first message.
                    std::thread::spawn(move || {
                        let mode = if background { launch::Mode::Background } else { launch::Mode::Terminal };
                        let screenshot = screenshot.map(std::path::PathBuf::from);
                        let toolkit = if toolkit { crate::toolkit::text() } else { None };
                        let extras =
                            launch::Extras { screenshot: screenshot.as_deref(), context: context.as_deref(), toolkit: toolkit.as_deref() };
                        match launch::launch(kind, std::path::Path::new(&cwd), prompt.as_deref(), extras, model.as_deref(), mode) {
                            Ok(l) => emit(&json!({"type": "launchResult", "requestId": request_id, "ok": true, "session": l.session})),
                            Err(e) => {
                                emit(&json!({"type": "launchResult", "requestId": request_id, "ok": false, "message": format!("{e:#}")}))
                            }
                        }
                    });
                    next_scan = Instant::now() + Duration::from_millis(1500);
                }
                Ok(Request::ToolkitRefresh) => {
                    std::thread::spawn(|| match crate::toolkit::refresh() {
                        Ok(path) => emit(&json!({"type": "toolkit", "ok": true, "path": path})),
                        Err(e) => emit(&json!({"type": "toolkit", "ok": false, "message": format!("{e:#}")})),
                    });
                }
                Ok(Request::Models { kind }) => {
                    // Listing runs the agent's own command; off the loop.
                    std::thread::spawn(move || {
                        let models = crate::models::list(kind);
                        let choosable = crate::models::flag(kind).is_some();
                        emit(&json!({"type": "models", "kind": kind, "choosable": choosable, "models": models}));
                    });
                }
                Ok(Request::History { request_id, agent_id }) => {
                    let messages = agents.iter().find(|a| a.id == agent_id).and_then(|a| scanner.history(a));
                    emit(&json!({
                        "type": "historyResult", "requestId": request_id, "agentId": agent_id,
                        "supported": messages.is_some(), "messages": messages.unwrap_or_default()
                    }));
                }
                Ok(Request::Keys { request_id, agent_id, keys, prompt_id }) => {
                    let parsed: Option<Vec<deliver::Key>> = keys.iter().map(|k| deliver::Key::parse(k)).collect();
                    let checked = reachable(&agents, &agent_id).and_then(|a| match parsed {
                        None => Err(Outcome::failed("bad_request", "unknown key")),
                        Some(keys) => keys_for(a, keys, prompt_id).map(|op| (a, op)),
                    });
                    match checked {
                        Ok((agent, op)) => executor.submit(app_action(agent, op, "replyResult", request_id)),
                        Err(o) => emit(&result_json("replyResult", &request_id, &o)),
                    }
                    next_scan = Instant::now() + Duration::from_millis(300);
                }
                Ok(Request::OctoberPair) => link.send(october_link::Command::Pair),
                Ok(Request::OctoberCancelPair) => link.send(october_link::Command::CancelPair),
                Ok(Request::OctoberForget) => link.send(october_link::Command::Forget),
                Ok(Request::Focus { request_id, agent_id }) => match agents.iter().find(|a| a.id == agent_id) {
                    Some(a) => executor.submit(app_action(a, Op::Focus, "attachResult", request_id)),
                    None => {
                        emit(&result_json("attachResult", &request_id, &Outcome::failed("unknown_agent", format!("no agent {agent_id}"))))
                    }
                },
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
                Ok(Request::PhoneCancelPair) => {
                    if let Some(p) = &phone {
                        p.send(crate::mobile::host::Cmd::CancelPair);
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

/// The agent, if Lantern can type into it at all.
fn reachable<'a>(agents: &'a [Agent], agent_id: &str) -> Result<&'a Agent, Outcome> {
    match agents.iter().find(|a| a.id == agent_id) {
        None => Err(Outcome::failed("unknown_agent", format!("no agent {agent_id}"))),
        Some(a) if !a.can_reply || a.route == Route::None => {
            Err(Outcome::failed("not_reachable", "Lantern can't type into this terminal yet"))
        }
        Some(a) => Ok(a),
    }
}

/// Keys for a permission prompt must name the prompt the person saw, and it must still be the
/// agent's current one.
pub(crate) fn keys_for(agent: &Agent, keys: Vec<deliver::Key>, prompt_id: Option<String>) -> Result<Op, Outcome> {
    let changed = || Outcome::failed("prompt_changed", "That prompt was already answered or replaced. Check the terminal.");
    match (&agent.question_kind, prompt_id) {
        (Some(QuestionKind::Permission), None) => Err(Outcome::failed("bad_request", "keys for a permission prompt must name the prompt")),
        (_, Some(id)) if agent.prompt_id.as_ref() != Some(&id) => Err(changed()),
        (_, prompt) => Ok(Op::Keys { keys, prompt }),
    }
}

/// An action for the app: answered with `<kind>` carrying the request id.
fn app_action(agent: &Agent, op: Op, kind: &'static str, request_id: String) -> Action {
    Action {
        agent: agent.clone(),
        op,
        deadline: Instant::now() + APP_DEADLINE,
        ticket: Ticket::new(),
        done: Box::new(move |o| emit(&result_json(kind, &request_id, &o))),
    }
}

/// `ok`, or `error` with a code (`uncertain` when it may have gone in) and a message.
fn result_json(kind: &str, request_id: &str, outcome: &Outcome) -> Value {
    match outcome {
        Outcome::Done => json!({"type": kind, "requestId": request_id, "ok": true}),
        Outcome::Failed { code, message } => json!({"type": kind, "requestId": request_id, "ok": false, "error": code, "message": message}),
        Outcome::Uncertain(message) => json!({
            "type": kind, "requestId": request_id, "ok": false, "error": "uncertain",
            "message": format!("Not sure it went in: {message}. Check the terminal before sending again.")
        }),
    }
}
