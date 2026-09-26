//! Replies, keypresses and focus requests run here, off the scan loop: one worker per
//! destination (a terminal, a tmux pane, an October node). A slow Automation prompt in one
//! terminal holds up neither the scan nor other terminals, and two sends to one terminal never
//! interleave.
//!
//! Every action carries a deadline and a ticket. Whoever waits for it can cancel it; the worker
//! claims the ticket immediately before typing, after the final check. So an action is either
//! canceled or expired (nothing typed), or started, and a started action's outcome is reported as
//! done, failed, or uncertain when Lantern can't tell whether it went in.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, SendError, Sender, channel};
use std::time::{Duration, Instant};

use crate::deliver::{self, Key, Uncertain};
use crate::model::{Agent, Route};
use crate::october_link::Link;

pub enum Op {
    Text(String),
    /// Keypresses; `prompt` is the permission prompt they answer, checked right before pressing.
    Keys {
        keys: Vec<Key>,
        prompt: Option<String>,
    },
    Focus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done,
    /// October accepted the entire message for its queue; final typing is not confirmed.
    Queued,
    /// Nothing was typed.
    Failed {
        code: &'static str,
        message: String,
    },
    /// Typing started but Lantern can't tell whether it went in. Sending again could repeat it.
    Uncertain(String),
}

impl Outcome {
    pub fn failed(code: &'static str, message: impl Into<String>) -> Outcome {
        Outcome::Failed { code, message: message.into() }
    }
}

const QUEUED: u8 = 0;
const CANCELED: u8 = 1;
const STARTED: u8 = 2;

/// Shared by the worker and whoever waits for the action.
#[derive(Default)]
pub struct Ticket(AtomicU8);

impl Ticket {
    pub fn new() -> Arc<Ticket> {
        Arc::new(Ticket(AtomicU8::new(QUEUED)))
    }

    /// Cancels the action if it hasn't started. `true` means it never will; `false` means it
    /// already started (its outcome is still coming).
    pub fn cancel(&self) -> bool {
        match self.0.compare_exchange(QUEUED, CANCELED, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => true,
            Err(state) => state == CANCELED,
        }
    }

    /// Claims the action for running; `false` when it was canceled.
    pub fn start(&self) -> bool {
        self.0.compare_exchange(QUEUED, STARTED, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }

    pub(crate) fn canceled(&self) -> bool {
        self.0.load(Ordering::SeqCst) == CANCELED
    }
}

pub struct Action {
    pub agent: Agent,
    pub op: Op,
    /// Typing must start before this, or the action expires.
    pub deadline: Instant,
    pub ticket: Arc<Ticket>,
    pub done: Box<dyn FnOnce(Outcome) + Send>,
}

pub struct Executor {
    link: Link,
    workers: HashMap<String, Sender<Action>>,
}

impl Executor {
    pub fn new(link: Link) -> Executor {
        Executor { link, workers: HashMap::new() }
    }

    pub fn submit(&mut self, action: Action) {
        let key = destination(&action.agent);
        let link = self.link.clone();
        let tx = self.workers.entry(key.clone()).or_insert_with(|| {
            let (tx, rx) = channel();
            std::thread::spawn(move || worker(rx, link));
            tx
        });
        if let Err(SendError(action)) = tx.send(action) {
            self.workers.remove(&key);
            (action.done)(Outcome::failed("internal", "Lantern's delivery worker stopped"));
        }
    }

    /// Lets workers for destinations no agent uses any more finish their queue and exit.
    pub fn retain(&mut self, agents: &[Agent]) {
        let live: Vec<String> = agents.iter().map(destination).collect();
        self.workers.retain(|k, _| live.contains(k));
    }
}

/// One worker per terminal: a tmux pane, a cmux surface, a tty, or an October node.
fn destination(agent: &Agent) -> String {
    match &agent.route {
        Route::Tmux => agent.tmux.as_ref().map(|p| format!("tmux:{:?}:{}", p.socket, p.pane_id)).unwrap_or_else(|| agent.id.clone()),
        Route::Cmux { surface, .. } => format!("cmux:{surface}"),
        Route::Terminal { tty } | Route::Iterm { tty } => format!("tty:{tty}"),
        Route::October { node_id, .. } => format!("october:{node_id}"),
        Route::None => agent.id.clone(),
    }
}

fn worker(rx: Receiver<Action>, link: Link) {
    while let Ok(action) = rx.recv() {
        let Action { agent, op, deadline, ticket, done } = action;
        done(perform(&agent, op, deadline, &ticket, &link));
    }
}

/// Runs one action. Public for tests.
pub fn perform(agent: &Agent, op: Op, deadline: Instant, ticket: &Ticket, link: &Link) -> Outcome {
    let gone = |ticket: &Ticket| -> Option<Outcome> {
        if ticket.canceled() {
            return Some(Outcome::failed("canceled", "Canceled before Lantern typed it."));
        }
        (Instant::now() >= deadline).then(|| Outcome::failed("expired", "Lantern couldn't get to it in time. Nothing was typed."))
    };
    if let Some(o) = gone(ticket) {
        return o;
    }
    // May wait while the person answers macOS's Automation prompt; the checks below come after.
    if !matches!(agent.route, Route::October { .. })
        && let Err(e) = deliver::prepare(agent, deadline.saturating_duration_since(Instant::now()).max(Duration::from_secs(1)))
    {
        return Outcome::failed("send_failed", format!("{e:#}"));
    }
    if let Some(o) = gone(ticket) {
        return o;
    }
    if let Op::Keys { prompt: Some(prompt), .. } = &op
        && crate::hooks::current_prompt(agent.pid).as_ref() != Some(prompt)
    {
        return Outcome::failed("prompt_changed", "That prompt was already answered or replaced. Check the terminal.");
    }
    if !ticket.start() {
        return Outcome::failed("canceled", "Canceled before Lantern typed it.");
    }
    let result = match op {
        Op::Text(text) => match agent.route {
            Route::October { .. } => {
                return match link.send_text(agent, &text) {
                    Ok(delivery) if delivery == "delivered" => Outcome::Done,
                    Ok(_) => Outcome::Queued,
                    Err(outcome) => outcome,
                };
            }
            _ => deliver::send_text(agent, &text).map_err(classify),
        },
        Op::Keys { keys, .. } => {
            let mut result = Ok(());
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    std::thread::sleep(Duration::from_millis(40));
                }
                if let Err(e) = deliver::send_key(agent, key) {
                    result = Err(if i > 0 { Outcome::Uncertain(format!("only some keys went in ({e:#})")) } else { classify(e) });
                    break;
                }
            }
            result
        }
        Op::Focus => match link.focus(agent) {
            Ok(true) => Ok(()),
            Ok(false) => deliver::focus(agent).map_err(classify),
            Err(o) => Err(o),
        },
    };
    result.err().unwrap_or(Outcome::Done)
}

fn classify(e: anyhow::Error) -> Outcome {
    match e.downcast_ref::<Uncertain>() {
        Some(u) => Outcome::Uncertain(u.0.clone()),
        None => Outcome::failed("send_failed", format!("{e:#}")),
    }
}
