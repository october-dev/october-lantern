//! Keeps a connection to October Desktop in the background: finds october-core, checks it,
//! reads its agents every few seconds, and runs pairing. The serve loop reads the shared state and
//! sends it commands; nothing here blocks the scan.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::actions::{Outcome, Ticket};
use crate::hooks::support_dir;
use crate::model::{Agent, Route, State};
use crate::october_core::{self, Client, OctoberAgent};

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkState {
    /// notInstalled | notRunning | readOnly | connected | pairing | error
    pub status: String,
    pub core_version: Option<String>,
    pub paired: bool,
    /// The 6-digit code October shows while it asks the user to allow Lantern.
    pub pairing_code: Option<String>,
    pub message: Option<String>,
    #[serde(skip)]
    pub agents: Vec<OctoberAgent>,
    pub agent_count: usize,
}

pub enum Command {
    Pair,
    CancelPair,
    Forget,
    Send { agent: OctoberAgent, text: String, ticket: Arc<Ticket>, done: Sender<Result<String, String>> },
    Focus { agent: OctoberAgent, done: Sender<Result<(), String>> },
}

/// The serve loop's handle on the link: its latest state, and a way to send it commands.
#[derive(Clone)]
pub struct Link {
    state: Arc<Mutex<LinkState>>,
    tx: Sender<Command>,
}

impl Link {
    pub fn snapshot(&self) -> LinkState {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    /// Resolve the addressed node exactly once a route exists. Never retarget a queued action
    /// to another node that happens to run the same harness in the same folder.
    pub fn agent_for(&self, a: &Agent) -> Option<OctoberAgent> {
        let state = self.state.lock().ok()?;
        match_october(&state, a).cloned()
    }

    /// Types a reply through October's safe delivery. A request that October hasn't answered in
    /// time is canceled if it hasn't gone out yet, and reported as uncertain if it has.
    pub fn send_text(&self, a: &Agent, text: &str) -> Result<String, Outcome> {
        october_core::validate_message(text).map_err(|e| Outcome::failed("message_too_long", e.to_string()))?;
        let agent = self.agent_for(a).ok_or_else(|| Outcome::failed("send_failed", "October no longer lists this agent"))?;
        let (done, wait) = channel();
        let ticket = Ticket::new();
        self.tx
            .send(Command::Send { agent, text: text.to_string(), ticket: ticket.clone(), done })
            .map_err(|_| Outcome::failed("send_failed", "October link stopped"))?;
        let answer = wait.recv_timeout(Duration::from_secs(10)).or_else(|_| {
            if ticket.cancel() {
                return Err(Outcome::failed("send_failed", "October was busy. Nothing was sent."));
            }
            wait.recv_timeout(Duration::from_secs(20)).map_err(|_| Outcome::Uncertain("October didn't confirm the message".into()))
        })?;
        answer.map_err(|e| {
            if e.contains("timed out") || e.contains("Timeout") || e.contains("timeout") {
                Outcome::Uncertain(format!("October didn't confirm the message ({e})"))
            } else {
                Outcome::failed("send_failed", e)
            }
        })
    }

    /// Shows the agent on October's canvas. `Ok(false)` when October doesn't list the agent.
    pub fn focus(&self, a: &Agent) -> Result<bool, Outcome> {
        let Some(agent) = self.agent_for(a) else { return Ok(false) };
        let (done, wait) = channel();
        self.tx.send(Command::Focus { agent, done }).map_err(|_| Outcome::failed("send_failed", "October link stopped"))?;
        wait.recv_timeout(Duration::from_secs(8))
            .map_err(|_| Outcome::failed("send_failed", "October didn't answer in time"))?
            .map(|_| true)
            .map_err(|e| Outcome::failed("send_failed", e))
    }
}

/// Which October node an agent is:
/// - its terminal's own node (`OCTOBER_BUS_NODE`, on October's Bus): exact;
/// - an existing route: kept only while that same node is still listed;
/// - otherwise exactly one node in the agent's folder running its harness, on the terminal's
///   canvas when it's known (`OCTOBER_BUS_CANVAS`) or across every listed canvas when it isn't
///   (October keeps some terminals off the Bus). A node whose harness October doesn't report
///   counts only when it's the only node in that folder. Anything else is ambiguous, and an
///   ambiguous match must not route a reply.
///
/// Every canvas with a running terminal is listed, so the agent's own node is always a
/// candidate: "exactly one" can't pick another node while the right one is missing.
fn match_october<'a>(link: &'a LinkState, a: &Agent) -> Option<&'a OctoberAgent> {
    let host = a.host.as_ref().filter(|h| h.app == "October")?;
    let canvas = host.canvas.as_deref();
    let on_canvas = |o: &&OctoberAgent| canvas.is_none_or(|c| o.canvas_id == c);
    if let Some(node) = host.node.as_deref() {
        return link.agents.iter().filter(on_canvas).find(|o| o.node_id == node);
    }
    let cwd = a.cwd.as_deref()?;
    if let Route::October { canvas_id, node_id } = &a.route {
        return link.agents.iter().filter(on_canvas).find(|o| {
            o.canvas_id == *canvas_id
                && o.node_id == *node_id
                && o.cwd.as_deref() == Some(cwd)
                && o.harness.as_deref().is_none_or(|h| h.contains(a.kind.as_str()))
        });
    }
    let same_dir: Vec<_> = link.agents.iter().filter(on_canvas).filter(|o| o.cwd.as_deref() == Some(cwd)).collect();
    let by_harness: Vec<_> =
        same_dir.iter().filter(|o| o.harness.as_deref().is_some_and(|h| h.contains(a.kind.as_str()))).copied().collect();
    match (by_harness.len(), same_dir.as_slice()) {
        (1, _) => Some(by_harness[0]),
        (0, [only]) if only.harness.is_none() => Some(only),
        _ => None,
    }
}

/// When Lantern is paired with October, agents inside October get their October name and state,
/// and replies go through October's safe delivery.
pub fn merge(agents: &mut [Agent], link: &LinkState) {
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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    client_id: String,
    credential: String,
}

fn saved_path() -> std::path::PathBuf {
    support_dir().join("october-client.json")
}

fn load_saved() -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(saved_path()).ok()?).ok()
}

fn save(s: &Saved) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::create_dir_all(support_dir());
    let path = saved_path();
    if std::fs::write(&path, serde_json::to_vec(s).unwrap_or_default()).is_ok() {
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
}

fn client_id() -> String {
    load_saved().map(|s| s.client_id).unwrap_or_else(|| uuid::Uuid::new_v4().hyphenated().to_string())
}

/// A link that never connects, for tests.
#[cfg(test)]
pub fn detached() -> Link {
    let (tx, _) = channel();
    Link { state: Arc::new(Mutex::new(LinkState { status: "notRunning".into(), ..Default::default() })), tx }
}

pub fn start() -> Link {
    let state = Arc::new(Mutex::new(LinkState { status: "notRunning".into(), ..Default::default() }));
    let (tx, rx) = channel();
    let s = state.clone();
    std::thread::spawn(move || run(s, rx));
    Link { state, tx }
}

fn run(state: Arc<Mutex<LinkState>>, rx: Receiver<Command>) {
    let mut client: Option<Client> = None;
    let mut pairing: Option<(String, String, Instant)> = None; // (requestId, clientId, started)
    let mut next_poll = Instant::now();
    loop {
        // Commands first.
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Command::Pair => match client.as_ref() {
                    Some(c) => {
                        let id = client_id();
                        match c.pair_request(&id) {
                            Ok((req, code)) => {
                                pairing = Some((req, id, Instant::now()));
                                update(&state, |s| {
                                    s.status = "pairing".into();
                                    s.pairing_code = Some(code);
                                    s.message = None;
                                });
                            }
                            Err(e) => update(&state, |s| s.message = Some(format!("{e:#}"))),
                        }
                    }
                    None => update(&state, |s| s.message = Some("Open October Desktop first.".into())),
                },
                Command::CancelPair => {
                    pairing = None;
                    update(&state, |s| s.pairing_code = None);
                }
                Command::Forget => {
                    let _ = std::fs::remove_file(saved_path());
                    client = None;
                    next_poll = Instant::now();
                }
                Command::Send { agent, text, ticket, done } => {
                    // Skipped when the caller gave up waiting before it went out.
                    let r = match client.as_mut() {
                        _ if !ticket.start() => continue,
                        Some(c) => c.send(&agent, &text).map_err(|e| format!("{e:#}")),
                        None => Err("October isn't running".into()),
                    };
                    let _ = done.send(r);
                }
                Command::Focus { agent, done } => {
                    let r = match client.as_mut() {
                        Some(c) => c.focus(&agent).map_err(|e| format!("{e:#}")),
                        None => Err("October isn't running".into()),
                    };
                    let _ = done.send(r);
                }
            }
        }

        // Pairing: poll until October answers.
        if let (Some((req, id, started)), Some(c)) = (pairing.clone(), client.as_ref()) {
            match c.pair_poll(&id, &req) {
                Ok((st, Some(credential))) if st == "approved" => {
                    save(&Saved { client_id: id, credential });
                    pairing = None;
                    client = None; // reconnect with the new credential
                    update(&state, |s| {
                        s.pairing_code = None;
                        s.message = None;
                    });
                    next_poll = Instant::now();
                }
                Ok((st, _)) if st == "denied" || st == "expired" => {
                    pairing = None;
                    update(&state, |s| {
                        s.pairing_code = None;
                        s.message = Some(if st == "denied" {
                            "October declined the connection.".into()
                        } else {
                            "The request expired. Try again.".into()
                        });
                    });
                }
                Err(e) if started.elapsed() > Duration::from_secs(150) => {
                    pairing = None;
                    update(&state, |s| {
                        s.pairing_code = None;
                        s.message = Some(format!("{e:#}"));
                    });
                }
                _ => {}
            }
        }

        if Instant::now() >= next_poll {
            let slow_down = refresh(&state, &mut client, pairing.is_some());
            // Every 3 seconds, or slower when a poll needs many reads: at most about 300 a minute,
            // half of what October allows a connected app. If October still says it's too
            // often, wait out its one-minute window.
            let reads = client.as_ref().map(|c| c.last_reads).unwrap_or(1) as u64;
            let every = if slow_down { 60_000 } else { (reads * 200).max(3_000) };
            next_poll = Instant::now() + Duration::from_millis(every);
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

/// Re-reads October's agents. True when October asked Lantern to slow down.
fn refresh(state: &Arc<Mutex<LinkState>>, client: &mut Option<Client>, pairing: bool) -> bool {
    let Some(run) = october_core::discover() else {
        *client = None;
        let status = if october_core::installed() { "notRunning" } else { "notInstalled" };
        update(state, |s| {
            s.status = status.into();
            s.paired = false;
            s.agents.clear();
            s.agent_count = 0;
            s.core_version = None;
        });
        return false;
    };
    // (Re)connect when core restarted or we have no client.
    if client.as_ref().is_none_or(|c| c.run.instance_id != run.instance_id) {
        let saved = load_saved().map(|s| (s.client_id, s.credential));
        let mut c = Client::new(run.clone(), saved.clone());
        let hs = c.handshake();
        let hs = match hs {
            Err(e) if saved.is_some() && format!("{e:#}").contains("revoked") => {
                // October revoked Lantern: fall back to read-only.
                let _ = std::fs::remove_file(saved_path());
                c = Client::new(run.clone(), None);
                update(state, |s| s.message = Some("October disconnected Lantern. Connect again to reply through October.".into()));
                c.handshake()
            }
            other => other,
        };
        if let Err(e) = hs {
            *client = None;
            update(state, |s| {
                s.status = "error".into();
                s.paired = false;
                s.agents.clear();
                s.agent_count = 0;
                s.message = Some(format!("{e:#}"));
            });
            return false;
        }
        *client = Some(c);
    }
    let c = client.as_mut().unwrap();
    match c.agents() {
        Ok(agents) => {
            let paired = c.paired();
            let version = c.run.core_version.clone();
            update(state, |s| {
                s.status = if pairing {
                    "pairing".into()
                } else if paired {
                    "connected".into()
                } else {
                    "readOnly".into()
                };
                s.paired = paired;
                s.core_version = Some(version);
                s.agent_count = agents.len();
                s.agents = agents;
            });
            false
        }
        Err(e) => {
            // Without a fresh list nothing may route through October: a stale node id could
            // deliver to the wrong agent.
            let mut msg = format!("{e:#}");
            if msg.contains("revoked") {
                *client = None;
            }
            let slow_down = msg.contains("BACKPRESSURE") || msg.contains("rate limit");
            if slow_down {
                msg = "October asked Lantern to slow down. Trying again in a minute.".into();
            }
            update(state, |s| {
                s.status = "error".into();
                s.paired = false;
                s.agents.clear();
                s.agent_count = 0;
                s.message = Some(msg);
            });
            slow_down
        }
    }
}

fn update(state: &Arc<Mutex<LinkState>>, f: impl FnOnce(&mut LinkState)) {
    if let Ok(mut s) = state.lock() {
        f(&mut s);
    }
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    #[test]
    fn audit_queued_reply_must_not_retarget_replacement_node() {
        let mut a = crate::tests::agent_for(123, 1, Some("ttys001"));
        a.cwd = Some("/project".into());
        a.host = Some(crate::model::HostApp {
            app: "October".into(),
            pid: 1,
            bundle_path: "/Applications/October.app".into(),
            canvas: Some("canvas-A".into()),
            node: None,
        });
        a.route = Route::October { canvas_id: "canvas-A".into(), node_id: "original-node".into() };
        let (tx, _) = channel();
        let link = Link {
            tx,
            state: Arc::new(Mutex::new(LinkState {
                status: "connected".into(),
                paired: true,
                agents: vec![OctoberAgent {
                    canvas_id: "canvas-A".into(),
                    node_id: "replacement-node".into(),
                    kind: "terminal".into(),
                    name: None,
                    harness: Some("claude".into()),
                    cwd: Some("/project".into()),
                    state: "working".into(),
                    attention: None,
                }],
                ..Default::default()
            })),
        };
        assert!(link.agent_for(&a).is_none(), "The original route vanished; a new node in the same folder is not the addressed recipient");
        {
            let mut state = link.state.lock().unwrap();
            let mut original = state.agents[0].clone();
            original.node_id = "original-node".into();
            state.agents.push(original);
        }
        assert_eq!(link.agent_for(&a).unwrap().node_id, "original-node", "An addressed node remains valid even with another matching node");
        a.route = Route::None;
        assert!(link.agent_for(&a).is_none(), "An unaddressed ambiguous terminal must not be guessed");
        link.state.lock().unwrap().agents[0].canvas_id = "different-canvas".into();
        assert_eq!(link.agent_for(&a).unwrap().node_id, "original-node", "Association stays within the terminal's canvas");
    }
}
