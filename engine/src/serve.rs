//! The stdio protocol the app talks to. See `protocol/README.md`.

use std::io::{BufRead, Write};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender, SyncSender};
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
    Shutdown,
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
        /// Images added to the task (pasted or dropped), also in the screenshots folder.
        #[serde(default)]
        images: Vec<String>,
        /// A model id for the agent's `--model`; absent for the agent's own default.
        model: Option<String>,
        /// What the person is working in (a task started from an app), added to the first message.
        context: Option<String>,
        /// Add the toolkit list to the first message.
        #[serde(default)]
        toolkit: bool,
        /// Connect the agent to October Bus with the other agents Lantern started.
        #[serde(default)]
        bus: bool,
        /// Let the agent use the computer's apps (Cua Driver). On unless the app says otherwise.
        #[serde(default = "yes")]
        computer: bool,
        /// The agent asks before each action, instead of starting in its own auto-approve mode
        /// (Settings: "Ask before agents act"). Off unless the app says otherwise.
        #[serde(default)]
        ask_first: bool,
        background: bool,
        #[serde(default = "launch_timeout")]
        timeout_ms: u64,
    },
    /// Turn Cua Driver on (download it if needed and start Lantern's driver) or off (stop it);
    /// answered with `computer` events.
    #[serde(rename = "computer.prepare", rename_all = "camelCase")]
    ComputerPrepare {
        enabled: bool,
        /// Stop a running driver first (it checks its permissions when it starts).
        #[serde(default)]
        restart: bool,
    },
    /// Download October Bus now if it isn't here (answered with `bus`), so the first session with
    /// the Bus doesn't wait for it.
    #[serde(rename = "bus.prepare")]
    BusPrepare,
    /// Install Lantern's October Harness now if `october` isn't on PATH and it isn't here yet
    /// (answered with `harness`, once it's ready or has failed).
    #[serde(rename = "harness.prepare", rename_all = "camelCase")]
    HarnessPrepare {
        request_id: Option<String>,
    },
    /// The October sign-in for October launches: the current access token and when it expires
    /// (unix seconds), or no token when signed out. Kept in memory only.
    #[serde(rename = "october.session", rename_all = "camelCase")]
    OctoberSession {
        access_token: Option<String>,
        expires_at: Option<u64>,
    },
    /// Rebuild the toolkit list now (answered with `toolkit`).
    #[serde(rename = "toolkit.refresh")]
    ToolkitRefresh,
    /// Look for installed agents again (answered with `installed`).
    #[serde(rename = "installed.refresh")]
    InstalledRefresh,
    /// The models to offer when starting `kind` (answered with `models`).
    #[serde(rename_all = "camelCase")]
    Models {
        request_id: String,
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
    /// A message from Ask Lantern for a running agent: through October Bus when the agent is on
    /// it (plus a nudge in its terminal), else typed in like `reply`. Answered with `sendResult`.
    #[serde(rename_all = "camelCase")]
    Send {
        request_id: String,
        agent_id: String,
        text: String,
        /// The send's own id, the same when it's tried again (defaults to the request id).
        #[serde(default)]
        key: Option<String>,
    },
    /// Answers the question an agent is asking (a permission, Claude's own question, its plan) with
    /// what the person chose; the engine presses the terminal's keys for it (see `answer_keys`).
    #[serde(rename_all = "camelCase")]
    Answer {
        request_id: String,
        agent_id: String,
        prompt_id: String,
        answer: Answer,
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

    /// The person's answer to a card: one of its options, their own words, or for a hand-off
    /// whether they did it. `revision` is the card's revision they saw. Answered with `cardResult`.
    #[serde(rename = "card.answer", rename_all = "camelCase")]
    CardAnswer {
        request_id: String,
        card_id: String,
        revision: i64,
        answer: crate::cards::Answer,
    },

    #[serde(rename = "task.control")]
    Control {
        #[serde(flatten)]
        command: crate::control::Command,
    },
    #[serde(rename = "routine")]
    Routine {
        #[serde(flatten)]
        command: crate::routines::Command,
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
        #[serde(rename = "intentId")]
        intent_id: String,
        bind: String,
        code: String,
        allow: bool,
    },
    #[serde(rename = "phone.retry")]
    PhoneRetry,
    #[serde(rename = "phone.revoke")]
    PhoneRevoke {
        bind: String,
    },
    #[serde(rename = "phone.stop")]
    PhoneStop,
    // MARK: Show me (see show.rs)
    #[serde(rename = "show.start", rename_all = "camelCase")]
    ShowStart {
        run_id: String,
    },
    #[serde(rename = "show.captured")]
    ShowCaptured(crate::show::Captured),
    #[serde(rename = "show.result", rename_all = "camelCase")]
    ShowResult {
        run_id: String,
        round: u32,
        capture_id: String,
        result: Value,
    },
    #[serde(rename = "show.next", rename_all = "camelCase")]
    ShowNext {
        run_id: String,
        step: u32,
    },
    #[serde(rename = "show.again", rename_all = "camelCase")]
    ShowAgain {
        run_id: String,
    },
    #[serde(rename = "show.changed", rename_all = "camelCase")]
    ShowChanged {
        run_id: String,
        capture_id: String,
        step: u32,
    },
    #[serde(rename = "show.stop", rename_all = "camelCase")]
    ShowStop {
        run_id: String,
    },
}

fn yes() -> bool {
    true
}

fn launch_timeout() -> u64 {
    45_000
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
    Scanned(Vec<Agent>),
    LaunchFinished(Value),
    /// A card or a launch's status changed (see `cards`).
    CardsChanged,
    HistoryRead {
        request_id: String,
        agent_id: String,
        messages: Option<Vec<crate::history::ChatMessage>>,
    },
}

enum ReadRequest {
    Scan,
    History { request_id: String, agent: Box<Agent> },
}

/// One bounded reader owns the scanner and its caches. Slow file/helper reads never occupy
/// command dispatch; at most one scan and one history read are outstanding.
fn reader(tx: Sender<Incoming>, mut read: impl FnMut(ReadRequest) -> Incoming + Send + 'static) -> SyncSender<ReadRequest> {
    let (send, receive) = mpsc::sync_channel(2);
    std::thread::spawn(move || {
        while let Ok(request) = receive.recv() {
            // A panic here would leave scans stopped and history always busy: exit instead, and
            // the app starts a fresh engine.
            let Ok(result) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(request))) else {
                std::process::exit(70);
            };
            if tx.send(result).is_err() {
                break;
            }
        }
    });
    send
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
    crate::elog::line(format_args!("engine {} started (pid {})", env!("CARGO_PKG_VERSION"), std::process::id()));
    crate::hooks::refresh_hook_binary();
    // The first discovery harvests the login environment once; keep it off the main loop.
    std::thread::spawn(emit_installed);
    // The toolkit list is kept, not rebuilt before each start: refresh it now if it's old.
    std::thread::spawn(crate::toolkit::refresh_if_stale);

    // Cards: the app gets every change as a `cards` event.
    let control_db = crate::control::open();
    if control_db.is_err() {
        crate::computer::hold();
    }
    let cards = crate::cards::start();
    if let Some(c) = cards {
        let changed = tx.clone();
        c.on_change(move || {
            let _ = changed.send(Incoming::CardsChanged);
        });
    }
    let show = crate::show::start(emit);
    let control = crate::control::worker(control_db, show.clone(), emit);
    let routines = crate::routines::worker(emit);
    let _ = routines.try_send(crate::routines::Command::List);
    let mut last_cards = String::new();
    let emit_cards = move |last: &mut String| {
        if let Some(c) = cards {
            let launches = c.snapshot();
            let summary = launches.to_string();
            if summary != *last {
                emit(&json!({"type": "cards", "launches": launches}));
                *last = summary;
            }
        }
    };
    emit_cards(&mut last_cards);

    let link = october_link::start();
    let mut executor = Executor::new(link.clone());
    let mut last_october = String::new();
    let reads = reader(tx.clone(), {
        let mut scanner = Scanner::new();
        let mut known = Vec::<Agent>::new();
        move |request| match request {
            ReadRequest::Scan => {
                known = scanner.scan();
                Incoming::Scanned(known.clone())
            }
            ReadRequest::History { request_id, agent } => {
                let messages = known.iter().find(|a| a.id == agent.id).and_then(|a| scanner.history(a));
                Incoming::HistoryRead { request_id, agent_id: agent.id, messages }
            }
        }
    });
    let mut scan_pending = false;
    let mut history_pending = false;
    let mut launch_pending = false;
    let mut launch_started = Instant::now();
    let mut agents: Vec<Agent> = Vec::new();
    let mut last_emit = Instant::now() - HEARTBEAT;
    let mut next_scan = Instant::now();
    let mut phone: Option<crate::mobile::host::MobileHost> = None;

    loop {
        if Instant::now() >= next_scan {
            if !scan_pending && reads.try_send(ReadRequest::Scan).is_ok() {
                scan_pending = true;
            }
            next_scan = Instant::now() + SCAN_EVERY;
        }
        let october = link.snapshot();
        let summary = serde_json::to_string(&october).unwrap_or_default();
        if summary != last_october {
            emit(&json!({"type": "october", "october": october}));
            last_october = summary;
        }

        let wait = next_scan.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(Incoming::LaunchFinished(result)) => {
                launch_pending = false;
                match result["ok"].as_bool() {
                    Some(true) => crate::elog::line(format_args!(
                        "launch {}: started after {} ms, session {}{}",
                        result["requestId"].as_str().unwrap_or_default(),
                        launch_started.elapsed().as_millis(),
                        result["session"].as_str().unwrap_or("none (a terminal window)"),
                        result["warning"].as_str().map(|w| format!("; {w}")).unwrap_or_default()
                    )),
                    _ => crate::elog::line(format_args!(
                        "launch {}: failed after {} ms: {}",
                        result["requestId"].as_str().unwrap_or_default(),
                        launch_started.elapsed().as_millis(),
                        result["message"].as_str().unwrap_or_default()
                    )),
                }
                emit(&result);
            }
            Ok(Incoming::CardsChanged) => emit_cards(&mut last_cards),
            Ok(Incoming::Scanned(mut fresh)) => {
                scan_pending = false;
                october_link::merge(&mut fresh, &link.snapshot());
                if let Some(p) = &phone {
                    p.update_agents(&fresh.iter().filter(|a| a.live).cloned().collect::<Vec<_>>());
                }
                if let Some(c) = cards {
                    c.bind(&fresh);
                    c.sweep();
                }
                for a in fresh.iter().filter(|a| !agents.iter().any(|b| b.id == a.id)) {
                    crate::elog::line(format_args!("agent seen: {}", crate::elog::agent(a)));
                }
                for a in agents.iter().filter(|a| !fresh.iter().any(|b| b.id == a.id)) {
                    crate::elog::line(format_args!("agent gone: {}", a.id));
                }
                if fresh != agents || last_emit.elapsed() >= HEARTBEAT {
                    agents = fresh;
                    executor.retain(&agents);
                    emit(&json!({"type": "snapshot", "generatedAt": now_ms(), "agents": agents}));
                    last_emit = Instant::now();
                }
            }
            Ok(Incoming::HistoryRead { request_id, agent_id, messages }) => {
                history_pending = false;
                emit(&json!({"type": "historyResult", "requestId": request_id, "agentId": agent_id,
                    "supported": messages.is_some(), "messages": messages.unwrap_or_default()}));
            }
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
                Ok(Request::Launch {
                    request_id,
                    kind,
                    cwd,
                    prompt,
                    screenshot,
                    images,
                    model,
                    context,
                    toolkit,
                    bus,
                    computer,
                    ask_first,
                    background,
                    timeout_ms,
                }) => {
                    if launch_pending {
                        crate::elog::line(format_args!("launch {request_id}: refused, another is still starting"));
                        emit(&json!({"type": "launchResult", "requestId": request_id, "ok": false,
                            "message": "Another session is still starting. Wait for its outcome before starting another."}));
                        continue;
                    }
                    launch_pending = true;
                    launch_started = Instant::now();
                    crate::elog::line(format_args!(
                        "launch {request_id}: {} in {}, {}, message {}, {} images, bus {bus}, computer {computer}",
                        kind.as_str(),
                        crate::elog::folder(std::path::Path::new(&cwd)),
                        if background { "in the background" } else { "in a terminal window" },
                        if prompt.as_deref().is_some_and(|p| !p.is_empty()) { "given" } else { "none" },
                        images.len() + usize::from(screenshot.is_some()),
                    ));
                    let finished = tx.clone();
                    let deadline = Instant::now() + Duration::from_millis(timeout_ms.min(45_000));
                    // Off the loop: it may wait for the agent to start before typing its first message.
                    std::thread::spawn(move || {
                        let mode = if background { launch::Mode::Background } else { launch::Mode::Terminal };
                        let screenshot = screenshot.map(std::path::PathBuf::from);
                        let images: Vec<std::path::PathBuf> = images.into_iter().map(std::path::PathBuf::from).collect();
                        let toolkit = if toolkit { crate::toolkit::text() } else { None };
                        let extras = launch::Extras {
                            screenshot: screenshot.as_deref(),
                            images: &images,
                            context: context.as_deref(),
                            toolkit: toolkit.as_deref(),
                            bus,
                            bus_note: None,
                            computer,
                            computer_note: None,
                            cards_note: None,
                            operation_id: None,
                            run_once: false,
                            ask_first,
                        };
                        let result = match launch::launch(
                            kind,
                            std::path::Path::new(&cwd),
                            prompt.as_deref(),
                            extras,
                            model.as_deref(),
                            mode,
                            deadline,
                        ) {
                            Ok(l) => json!({
                                "type": "launchResult", "requestId": request_id, "ok": true, "session": l.session, "launch": l.launch,
                                "warning": l.bus_problem.map(|p| format!("Started without October Bus: {p}"))
                            }),
                            Err(e) => json!({"type": "launchResult", "requestId": request_id, "ok": false, "message": format!("{e:#}")}),
                        };
                        let _ = finished.send(Incoming::LaunchFinished(result));
                    });
                    next_scan = Instant::now() + Duration::from_millis(1500);
                }
                Ok(Request::ComputerPrepare { enabled, restart }) => {
                    if enabled && crate::computer::held() {
                        emit(&crate::computer::status_json("off", Some("App access is paused; choose Continue app access.".into())));
                        continue;
                    }
                    // Each request supersedes the ones before it: an earlier start still downloading
                    // or starting gives up instead of turning the driver on after an off.
                    let generation = crate::computer::want(enabled);
                    if !enabled || restart {
                        // Show me lets go of the driver before it stops.
                        let _ = show.send(crate::show::Cmd::DriverOff);
                    }
                    std::thread::spawn(move || {
                        use crate::computer::{self, Started};
                        if (!enabled || restart)
                            && let Err(error) = computer::stop_for(generation)
                        {
                            if computer::latest(generation) {
                                emit(&computer::status_json("failed", Some(format!("App-access stopping is unconfirmed: {error:#}"))));
                            }
                            return;
                        }
                        if !computer::latest(generation) {
                            return;
                        }
                        if !enabled {
                            if computer::latest(generation) {
                                emit(&computer::status_json("off", None));
                            }
                            return;
                        }
                        if !computer::ready() {
                            emit(&computer::status_json("installing", None));
                        }
                        match computer::start(generation) {
                            Ok(Started::Ready) => emit(&computer::status_json("ready", None)),
                            Ok(Started::Superseded) => {}
                            Err(e) if computer::latest(generation) => emit(&computer::status_json("failed", Some(format!("{e:#}")))),
                            Err(_) => {}
                        }
                    });
                }
                Ok(Request::BusPrepare) => {
                    std::thread::spawn(|| match crate::bus::ensure_installed() {
                        Ok(_) => emit(&json!({"type": "bus", "ready": true})),
                        Err(e) => emit(&json!({"type": "bus", "ready": false, "message": format!("{e:#}")})),
                    });
                }
                Ok(Request::HarnessPrepare { request_id }) => {
                    std::thread::spawn(move || {
                        let now = harness_json(request_id.as_deref());
                        if now["status"] == "ready" {
                            return emit(&now);
                        }
                        let mut installing = now;
                        installing["status"] = json!("installing");
                        if let Some(m) = installing.as_object_mut() {
                            m.remove("message");
                        }
                        emit(&installing);
                        crate::harness::prepare(move |result| {
                            let mut v = harness_json(request_id.as_deref());
                            if let Err(e) = result {
                                v["status"] = json!("failed");
                                v["message"] = json!(format!("{e:#}"));
                            }
                            emit(&v);
                            emit_installed();
                        });
                    });
                }
                Ok(Request::OctoberSession { access_token, expires_at }) => {
                    crate::harness::set_session(access_token.zip(expires_at));
                }
                Ok(Request::InstalledRefresh) => {
                    launch::refresh();
                    std::thread::spawn(emit_installed);
                }
                Ok(Request::ToolkitRefresh) => {
                    std::thread::spawn(|| match crate::toolkit::refresh() {
                        Ok(path) => emit(&json!({"type": "toolkit", "ok": true, "path": path})),
                        Err(e) => emit(&json!({"type": "toolkit", "ok": false, "message": format!("{e:#}")})),
                    });
                }
                Ok(Request::Models { request_id, kind }) => {
                    // Listing runs the agent's own command; off the loop.
                    std::thread::spawn(move || {
                        let (models, error) = match crate::models::list(kind) {
                            Ok(models) => (models, None),
                            Err(error) => (Vec::new(), Some(error.to_string())),
                        };
                        let choosable = crate::models::flag(kind).is_some();
                        emit(
                            &json!({"type": "models", "requestId": request_id, "kind": kind, "choosable": choosable, "models": models, "error": error}),
                        );
                    });
                }
                Ok(Request::History { request_id, agent_id }) => {
                    let agent = agents.iter().find(|a| a.id == agent_id).cloned();
                    if let Some(agent) = agent {
                        if !history_pending
                            && reads.try_send(ReadRequest::History { request_id: request_id.clone(), agent: Box::new(agent) }).is_ok()
                        {
                            history_pending = true;
                        } else {
                            emit(&json!({"type": "historyResult", "requestId": request_id, "agentId": agent_id,
                                "busy": true, "supported": true, "messages": []}));
                        }
                    } else {
                        emit(&json!({"type": "historyResult", "requestId": request_id, "agentId": agent_id,
                            "supported": false, "messages": []}));
                    }
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
                Ok(Request::Send { request_id, agent_id, text, key }) => {
                    let key = key.filter(|k| !k.is_empty() && k.len() <= 64).unwrap_or_else(|| request_id.clone());
                    match reachable(&agents, &agent_id) {
                        Ok(agent) => executor.submit(app_action(agent, Op::Bus { text, key }, "sendResult", request_id)),
                        Err(o) => emit(&result_json("sendResult", &request_id, &o)),
                    }
                    next_scan = Instant::now() + Duration::from_millis(300);
                }
                Ok(Request::Answer { request_id, agent_id, prompt_id, answer }) => {
                    let checked = reachable(&agents, &agent_id)
                        .and_then(|a| answer_keys(a, &answer).and_then(|keys| keys_for(a, keys, Some(prompt_id))).map(|op| (a, op)));
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
                    let result = match agents.iter().find(|a| a.id == agent_id) {
                        Some(a) if a.tmux.is_some() => launch::attach_in_terminal(a.tmux.as_ref().unwrap()).map_err(|e| format!("{e:#}")),
                        // A Lantern session (Windows): its window, or a new one.
                        Some(a) if matches!(a.route, crate::model::Route::Pty { .. }) => {
                            crate::deliver::focus(a).map_err(|e| format!("{e:#}"))
                        }
                        _ => Err("agent is not in a tmux session".to_string()),
                    };
                    emit(&json!({"type": "attachResult", "requestId": request_id, "ok": result.is_ok(), "message": result.err()}));
                }
                Ok(Request::Control { command }) => {
                    if let Err(error) = control.try_send(command) {
                        let (std::sync::mpsc::TrySendError::Full(command) | std::sync::mpsc::TrySendError::Disconnected(command)) = error;
                        emit(
                            &json!({"type":"controlResult","requestId":command.request_id,"ok":false,"message":"Control is busy or unavailable."}),
                        );
                    }
                }
                Ok(Request::Routine { command }) => {
                    if let Err(error) = routines.try_send(command) {
                        let (std::sync::mpsc::TrySendError::Full(command) | std::sync::mpsc::TrySendError::Disconnected(command)) = error;
                        if let Some(id) = command.request_id() {
                            emit(
                                &json!({"type":"routineResult","requestId":id,"ok":false,"message":"Routines are busy or unavailable; try again."}),
                            );
                        } else {
                            emit(&json!({"type":"routines","items":[],"error":"Routines are busy or unavailable; try again."}));
                        }
                    }
                }
                Ok(Request::CardAnswer { request_id, card_id, revision, answer }) => {
                    let result = match cards {
                        Some(c) => c.answer(&card_id, revision, &answer, "app").map_err(|e| format!("{e:#}")),
                        None => Err("Cards aren't available.".to_string()),
                    };
                    emit(&json!({"type": "cardResult", "requestId": request_id, "cardId": card_id, "ok": result.is_ok(),
                        "message": result.err()}));
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
                Ok(Request::PhoneDecide { intent_id, bind, code, allow }) => {
                    if let Some(p) = &phone {
                        p.send(crate::mobile::host::Cmd::Decide { intent_id, bind, code, allow });
                    }
                }
                Ok(Request::PhoneRetry) => {
                    if let Some(p) = &phone {
                        p.send(crate::mobile::host::Cmd::Retry);
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
                Ok(Request::ShowStart { run_id }) => {
                    let _ = show.send(crate::show::Cmd::Start { run_id });
                }
                Ok(Request::ShowCaptured(c)) => {
                    let _ = show.send(crate::show::Cmd::Captured(c));
                }
                Ok(Request::ShowResult { run_id, round, capture_id, result }) => {
                    let _ = show.send(crate::show::Cmd::Result { run_id, round, capture_id, result });
                }
                Ok(Request::ShowNext { run_id, step }) => {
                    let _ = show.send(crate::show::Cmd::Next { run_id, step });
                }
                Ok(Request::ShowAgain { run_id }) => {
                    let _ = show.send(crate::show::Cmd::Again { run_id });
                }
                Ok(Request::ShowChanged { run_id, capture_id, step }) => {
                    let _ = show.send(crate::show::Cmd::Changed { run_id, capture_id, step });
                }
                Ok(Request::Shutdown) => {
                    crate::computer::want(false);
                    let _ = show.send(crate::show::Cmd::DriverOff);
                    crate::computer::stop();
                    return Ok(());
                }
                Ok(Request::ShowStop { run_id }) => {
                    let _ = show.send(crate::show::Cmd::Stop { run_id });
                }
                // The line itself isn't logged: some requests carry an access token.
                Err(e) => eprintln!("lantern-engine: bad request ({e})"),
            },
            Ok(Incoming::Closed) | Err(RecvTimeoutError::Disconnected) => {
                crate::computer::want(false);
                let _ = show.send(crate::show::Cmd::DriverOff);
                crate::computer::stop();
                return Ok(());
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// The `installed` event: agents that can be started, tmux, and October Harness's status.
fn emit_installed() {
    let installed = launch::installed();
    let on_path = launch::on_path().kinds.contains(&Kind::October);
    emit(&json!({"type": "installed", "installed": installed, "harness": crate::harness::status(on_path)}));
}

/// The `harness` event, for `harness.prepare`.
fn harness_json(request_id: Option<&str>) -> Value {
    let mut v = crate::harness::status(launch::on_path().kinds.contains(&Kind::October));
    v["type"] = json!("harness");
    if let Some(id) = request_id {
        v["requestId"] = json!(id);
    }
    v
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

/// What the person chose for an agent's question.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Answer {
    /// Allow once; approve the plan.
    Yes,
    /// The permission prompt's "don't ask again" option.
    Always,
    /// Deny; keep planning; don't answer Claude's question (it's told you declined).
    No,
    /// The options picked for each of Claude's questions, in order (0-based).
    Picks { picks: Vec<Vec<usize>> },
}

/// The keys that give `answer` in the agent's terminal, as Claude Code's prompts take them:
/// a permission's options are 1 (yes), 2 (don't ask again, when offered) and Escape; a plan's 1
/// and Escape. Claude's questions take an option's number; one single-choice question is answered
/// by it, otherwise each pick-many question is ticked and left with Tab, and 1 submits them all.
pub(crate) fn answer_keys(agent: &Agent, answer: &Answer) -> Result<Vec<deliver::Key>, Outcome> {
    use deliver::Key;
    let bad = |m: &str| Outcome::failed("bad_request", m.to_string());
    let digit = |i: usize| char::from_digit(i as u32 + 1, 10).map(Key::Char).ok_or_else(|| bad("that option can't be picked from Lantern"));
    let choices = agent.choices.clone().unwrap_or_default();
    match (agent.question_kind, answer) {
        (Some(QuestionKind::Permission | QuestionKind::Plan | QuestionKind::Choice), Answer::No) => Ok(vec![Key::Escape]),
        (Some(QuestionKind::Permission | QuestionKind::Plan), Answer::Yes) => Ok(vec![Key::Char('1')]),
        (Some(QuestionKind::Permission), Answer::Always) if choices.always.is_some() => Ok(vec![Key::Char('2')]),
        (Some(QuestionKind::Choice), Answer::Picks { picks }) => {
            let questions = &choices.questions;
            if questions.is_empty() || picks.len() != questions.len() {
                return Err(bad("answer every question"));
            }
            let mut keys = Vec::new();
            for (q, picked) in questions.iter().zip(picks) {
                let mut picked = picked.clone();
                picked.sort_unstable();
                picked.dedup();
                if picked.iter().any(|i| *i >= q.options.len()) || (!q.multi_select && picked.len() != 1) {
                    return Err(bad("pick one of the options for each question"));
                }
                for i in &picked {
                    keys.push(digit(*i)?);
                }
                if q.multi_select {
                    keys.push(Key::Tab);
                }
            }
            if questions.len() > 1 || questions[0].multi_select {
                keys.push(Key::Char('1'));
            }
            Ok(keys)
        }
        _ => Err(Outcome::failed("prompt_changed", "That question has changed. Check the terminal.")),
    }
}

/// Keys for a permission prompt must name the prompt the person saw, and it must still be the
/// agent's current one.
pub(crate) fn keys_for(agent: &Agent, keys: Vec<deliver::Key>, prompt_id: Option<String>) -> Result<Op, Outcome> {
    let changed = || Outcome::failed("prompt_changed", "That prompt was already answered or replaced. Check the terminal.");
    match (&agent.question_kind, prompt_id) {
        (Some(k), None) if k.answered_with_keys() => Err(Outcome::failed("bad_request", "keys for a prompt must name the prompt")),
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
        Outcome::Queued => json!({"type": kind, "requestId": request_id, "ok": true, "delivery": "queued", "message": "Queued in October"}),
        Outcome::OnBus => {
            json!({"type": kind, "requestId": request_id, "ok": true, "delivery": "bus", "message": "Sent through October Bus"})
        }
        Outcome::Failed { code, message } => json!({"type": kind, "requestId": request_id, "ok": false, "error": code, "message": message}),
        Outcome::Uncertain(message) => json!({
            "type": kind, "requestId": request_id, "ok": false, "error": "uncertain",
            "message": format!("Not sure it went in: {message}. Check the terminal before sending again.")
        }),
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;

    #[test]
    fn a_blocked_reader_does_not_block_incoming_commands() {
        let (events, incoming) = mpsc::channel();
        let (release, wait) = mpsc::channel::<()>();
        let (started, did_start) = mpsc::channel();
        let jobs = reader(events.clone(), move |_| {
            let _ = started.send(());
            let _ = wait.recv();
            Incoming::Scanned(Vec::new())
        });
        assert!(jobs.try_send(ReadRequest::Scan).is_ok());
        assert!(did_start.recv_timeout(Duration::from_secs(1)).is_ok());
        assert!(jobs.try_send(ReadRequest::Scan).is_ok());
        assert!(jobs.try_send(ReadRequest::Scan).is_ok());
        assert!(jobs.try_send(ReadRequest::Scan).is_err(), "The slow-work queue must stay bounded");
        assert!(events.send(Incoming::Line("{\"type\":\"phone.stop\"}".into())).is_ok());
        assert!(
            matches!(incoming.recv_timeout(Duration::from_secs(1)), Ok(Incoming::Line(_))),
            "Dispatch must receive controls while the reader is blocked"
        );
        drop(release);
        drop(jobs);
    }
}
