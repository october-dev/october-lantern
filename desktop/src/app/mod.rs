//! The new app (CONTRACT §2): `Lantern` owns the engine, the model, what's being typed and the
//! windows. Engine events update the model (`engine.rs`); what people do arrives as `UiAction`s
//! and becomes effects in exactly one place (`apply.rs`); the pill and panel windows live in
//! `windows.rs`. Runs with `--shell` until it replaces the legacy UI.

/// Event trace for hands-on checks: `LANTERN_DEBUG=1`.
macro_rules! trace {
    ($($arg:tt)*) => {
        if std::env::var_os("LANTERN_DEBUG").is_some() {
            eprintln!("lantern {:>8.2}s: {}", $crate::app::uptime(), format!($($arg)*));
        }
    };
}
pub(crate) use trace;

static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
pub(crate) fn uptime() -> f64 {
    STARTED.get_or_init(Instant::now).elapsed().as_secs_f64()
}

mod apply;
mod assistant;
mod desktop;
mod dialogs;
mod dictation;
mod engine;
pub mod fixture;
mod logs;
mod menu;
mod point_ask;
mod preferences;
mod select;
mod session;
mod shots;
mod uninstall;
mod windows;

use crate::model::{Inputs, Model, Prefs, drafts::Ticket, inbox::Turns};
use crate::os::Event;
use crate::sidecar::Sidecar;
use crate::ui::{Assets, UiAction, UiState};
use anyhow::Result;
use eframe::egui;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

/// A request the engine hasn't answered yet.
#[derive(Clone, Debug)]
enum Pending {
    /// The ticket says which draft revision was sent, so only that is cleared when it arrives.
    Reply(Ticket),
    Keys {
        agent_id: String,
    },
    Focus {
        agent_id: String,
    },
    History,
    Launch,
    /// A message sent without the composer (Point & Ask).
    Direct {
        agent_id: String,
    },
}

pub struct Lantern {
    engine: Sidecar,
    connected: bool,
    model: Model,
    inputs: Inputs,
    turns: Turns,
    view: UiState,
    assets: Option<Assets>,
    windows: windows::Windows,
    next_request: u64,
    pending: HashMap<String, Pending>,
    /// The history request whose answer may update the open chat; older answers are stale.
    history_request: Option<String>,
    history_at: Instant,
    toast_until: Option<Instant>,
    actions: Vec<UiAction>,
    launching: session::Launching,
    asking: assistant::Asking,
    pointing: point_ask::Pointing,
    upkeep: preferences::Upkeep,
    dictating: dictation::Dictating,
    /// Where Lantern was installed, read when uninstalling starts (the preferences go with it).
    installed: Option<std::path::PathBuf>,
    /// The October account, once signed in (its access token asks October's AI).
    session: Option<crate::account::Session>,
    /// Sign-in attempts: a newer one (or signing out) makes older results stale.
    auth: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// The sign-in you started (not a restored one), until it finishes.
    signing_in: Option<u64>,
    /// The system keyring's sign-in store.
    vault: Option<Sender<crate::account::VaultCommand>>,
    /// What the launch asked for (`--new-session`, `--settings`…), once the engine has said which
    /// agents are installed.
    on_ready: Option<String>,
    /// Results of work done off the frame loop (captures, the folder picker).
    tx: Sender<Event>,
    events: Receiver<Event>,
    started: Instant,
    /// When the engine was last restarted after stopping on its own.
    engine_restarts: Vec<Instant>,
}

impl Lantern {
    /// `ctx`: woken when background work finishes (none in tests).
    fn new(engine: Sidecar, ctx: Option<egui::Context>) -> Self {
        let (prefs, turns) = load_state();
        let mut model = Model { caps: capabilities(), prefs, ..Default::default() };
        model.system.version = env!("CARGO_PKG_VERSION").into();
        // Ask Lantern's saved conversations (never from tests).
        let conversations = (!cfg!(test)).then(assistant::conversations_path);
        model.assistant.conversations = conversations.as_deref().map(assistant::load_conversations).unwrap_or_default();
        let (tx, events) = match ctx {
            Some(ctx) => crate::os::event_pump(move |_| ctx.request_repaint()),
            None => std::sync::mpsc::channel(),
        };
        Self {
            engine,
            connected: false,
            model,
            inputs: Inputs::default(),
            turns,
            view: UiState::default(),
            assets: None,
            windows: windows::Windows::default(),
            next_request: 0,
            pending: HashMap::new(),
            history_request: None,
            history_at: Instant::now(),
            toast_until: None,
            actions: Vec::new(),
            launching: session::Launching::default(),
            asking: assistant::Asking::saving_to(conversations),
            pointing: point_ask::Pointing::default(),
            upkeep: preferences::Upkeep::default(),
            installed: None,
            dictating: dictation::Dictating::default(),
            session: None,
            auth: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
            signing_in: None,
            vault: None,
            on_ready: None,
            tx,
            events,
            started: Instant::now(),
            engine_restarts: Vec::new(),
        }
    }

    /// Saves preferences and Waiting's seen/dismissed turns (never from tests).
    fn save_prefs(&self) {
        // Never from tests, and never once uninstalling has begun (the settings are being removed).
        if cfg!(test) || self.model.system.uninstall != crate::model::Uninstall::Idle {
            return;
        }
        let state = serde_json::json!({"prefs": self.model.prefs, "turns": self.turns});
        if let Err(error) = crate::settings::private_write(&state_path(), state.to_string().as_bytes()) {
            eprintln!("Lantern: couldn't save preferences: {error:#}");
        }
    }

    /// Results of background work.
    fn integration(&mut self, event: Event) {
        match event {
            Event::Screenshot { owner, generation, result } if owner == "__new" => {
                self.windows.capture_done();
                self.screenshot_captured(generation, result);
            }
            Event::Screenshot { owner, generation, result } if owner == "__point" => {
                self.windows.capture_done();
                self.point_ask_captured(generation, result);
            }
            #[cfg(windows)]
            Event::Frozen { generation, result } => {
                self.windows.capture_done();
                self.point_ask_frozen(generation, result);
            }
            #[cfg(windows)]
            Event::PointAskShot { generation, result } => self.point_ask_shot(generation, result),
            Event::PointAsk { generation, turn, piece } => self.point_ask_piece(generation, turn, piece),
            Event::PointAskDone { generation, turn, error } => self.point_ask_done(generation, turn, error),
            Event::PointAskSent { generation, agent_id, text } => self.point_ask_sent(generation, agent_id, text),
            Event::Folder(result) => {
                self.windows.native_dialog = false;
                match result {
                    Ok(path) => self.inputs.new_session.folder = Some(path.to_string_lossy().into_owned()),
                    Err(error) if !error.contains("cancelled") => self.show(error),
                    Err(_) => {}
                }
            }
            Event::Status(text) => self.show(text),
            Event::Activate(id) => self.activate(&id),
            Event::Quit => self.windows.quit(),
            Event::Shortcuts(available) => self.model.caps.global_hotkeys = available,
            Event::Tray(available) => self.model.caps.tray = available,
            Event::Hooks(result) => self.hooks_result(result),
            Event::Uninstalled(failures) => self.uninstalled(failures),
            Event::LogsSent(result) => self.logs_sent(result),
            Event::NotificationsAllowed(allowed) => self.model.system.notifications_allowed = allowed,
            Event::Transcript { owner, generation, result } => self.transcript(&owner, generation, result),
            Event::SpeechSetup(step) => self.speech_setup(step),
            Event::Update(result) => self.update_result(result),
            Event::UpdateInstalled { result, quietly } => self.update_installed(result, quietly),
            Event::Assistant { generation, turn, piece } => self.assistant_piece(generation, turn, piece),
            Event::AssistantDone { generation, turn, error } => self.assistant_done(generation, turn, error),
            Event::Account { generation, result } if generation == self.auth.load(std::sync::atomic::Ordering::SeqCst) => {
                self.model.account.pending = false;
                trace!(
                    "account: {}",
                    match &result {
                        Ok(session) => format!("signed in, plan {:?}", session.plan["plan"].as_str().or(session.plan["tier"].as_str())),
                        Err(error) => format!("not signed in ({error})"),
                    }
                );
                // Only the first result of a sign-in you started; refreshes and restores are quiet.
                let welcome = self.signing_in.take_if(|started| *started == generation).is_some();
                match result {
                    Ok(session) => {
                        let plan = session.plan["plan"].as_str().or(session.plan["tier"].as_str()).map(str::to_string);
                        self.model.account.signed_in = Some(crate::model::Account { email: session.email.clone(), plan });
                        self.model.account.error = None;
                        crate::analytics::identify(&session.access, &session.email);
                        self.session = Some(session);
                        if welcome {
                            crate::analytics::capture("october_signed_in", serde_json::json!({}));
                            self.signed_in();
                        }
                    }
                    Err(error) => {
                        self.session = None;
                        self.model.account.signed_in = None;
                        self.model.account.error = Some(error);
                    }
                }
            }
            _ => {}
        }
    }

    /// Back from signing in (Swift `signInWatch`): Lantern comes forward where you were (Ask
    /// Lantern if nothing was open), with your question still in its box, and says who you are.
    fn signed_in(&mut self) {
        self.windows.show_pill(true);
        if self.model.panel.is_none() {
            self.set_panel(Some(crate::model::PanelMode::Assistant));
        }
        self.windows.raise_panel();
        self.model.compose_focus += 1;
        let who = self.model.account.signed_in.as_ref().map(|account| account.email.trim()).filter(|email| email.contains('@'));
        self.show(who.map_or_else(|| "Signed in to October".to_string(), |email| format!("Signed in as {email}")));
    }

    /// Restores the October sign-in kept in the system keyring, and keeps it fresh.
    fn restore_account(&mut self) {
        let generation = self.auth.load(std::sync::atomic::Ordering::SeqCst);
        let current = self.auth.clone();
        self.account_flow(generation).start(move || crate::account::restore(generation, current));
    }

    /// A sign-in that runs (and refreshes itself) off the frame loop; newer attempts make it stale.
    fn account_flow(&mut self, generation: u64) -> crate::account::Flow {
        let vault = self.vault.get_or_insert_with(|| crate::account::vault(self.tx.clone())).clone();
        crate::account::Flow {
            generation,
            current: self.auth.clone(),
            engine: self.engine.sender(),
            vault,
            remember: true,
            events: self.tx.clone(),
        }
    }

    /// Starts a sign-in (browser or email); its result arrives as `Event::Account`.
    fn sign_in(
        &mut self,
        work: impl FnOnce(u64, std::sync::Arc<std::sync::atomic::AtomicU64>) -> Result<crate::account::Session> + Send + 'static,
    ) {
        let generation = self.auth.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let current = self.auth.clone();
        self.model.account.pending = true;
        self.model.account.error = None;
        self.signing_in = Some(generation);
        self.account_flow(generation).start(move || work(generation, current));
    }

    fn cancel_sign_in(&mut self) {
        self.auth.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.signing_in = None;
        self.model.account.pending = false;
    }

    /// Signs out: forgets the keyring sign-in, tells the engine (phone and October sessions stop),
    /// and ends Ask Lantern's conversation.
    fn sign_out(&mut self) {
        let generation = self.auth.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        crate::analytics::capture("october_signed_out", serde_json::json!({}));
        crate::analytics::forget_user();
        if let Some(session) = self.session.take() {
            std::thread::spawn(move || crate::account::logout(session.access));
        }
        let vault = self.vault.clone();
        let _ = self.engine.sender().credentials(generation, &self.auth, None, || {
            if let Some(vault) = vault {
                let _ = vault.send(crate::account::VaultCommand::Clear);
            }
        });
        self.model.account = Default::default();
        self.clear_assistant();
    }

    /// The engine stopped: Lantern starts it again (up to three times in five minutes), except
    /// while uninstalling, which stops it on purpose.
    fn engine_stopped(&mut self, error: String) {
        self.connected = false;
        self.engine_restarts.retain(|at| at.elapsed() < Duration::from_secs(300));
        if self.model.system.uninstall != crate::model::Uninstall::Idle || self.engine_restarts.len() >= 3 {
            self.model.engine = crate::model::EngineHealth::Failed(error);
            return;
        }
        self.restart_engine();
    }

    fn restart_engine(&mut self) {
        self.engine_restarts.push(Instant::now());
        self.connected = false;
        self.model.engine = match self.engine.restart() {
            Ok(()) => crate::model::EngineHealth::Restarting,
            Err(error) => crate::model::EngineHealth::Failed(format!("couldn't restart Lantern's engine: {error:#}")),
        };
    }

    /// A fresh engine (a restart) has no sign-in yet: it gets the current one.
    fn give_engine_sign_in(&self) {
        if let Some(session) = &self.session {
            let generation = self.auth.load(std::sync::atomic::Ordering::SeqCst);
            let _ = self.engine.sender().credentials(generation, &self.auth, Some((&session.access, session.expires)), || {});
        }
    }

    /// Sends a request to the engine with a fresh id; `None` when the engine isn't there.
    fn request(&mut self, mut request: Value) -> Option<String> {
        if !self.connected {
            return None;
        }
        self.next_request += 1;
        let id = format!("shell-{}", self.next_request);
        request["requestId"] = id.clone().into();
        match self.engine.send(&request) {
            Ok(()) => Some(id),
            Err(error) => {
                self.connected = false;
                self.show(format!("Lantern's engine isn't answering: {error:#}"));
                None
            }
        }
    }

    /// A short message at the bottom of the panel, for four seconds.
    fn show(&mut self, text: impl Into<String>) {
        let id = self.model.toast.as_ref().map_or(1, |toast| toast.id + 1);
        self.model.toast = Some(crate::model::Toast { id, text: text.into() });
        self.toast_until = Some(Instant::now() + Duration::from_secs(4));
    }

    fn now_ms() -> f64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    }

    fn tick(&mut self) {
        if self.toast_until.is_some_and(|until| Instant::now() >= until) {
            self.toast_until = None;
            self.model.toast = None;
        }
        // While a conversation is open it refreshes every two seconds, so a long turn shows its
        // progress (snapshots only arrive when an agent's state changes).
        if self.model.chat.is_some() && self.history_at.elapsed() >= Duration::from_secs(2) {
            self.refresh_chat();
        }
        self.launch_tick();
        self.assistant_tick();
        self.point_ask_tick();
        self.upkeep_tick();
        self.publish_tray_menu();
        if self.connected {
            // How many agents of each harness are running, once a day (DAU).
            let mut kinds: Vec<(String, usize)> = Vec::new();
            for agent in &self.model.agents {
                match kinds.iter_mut().find(|(kind, _)| kind == agent.kind.as_str()) {
                    Some((_, n)) => *n += 1,
                    None => kinds.push((agent.kind.as_str().to_string(), 1)),
                }
            }
            crate::analytics::daily_active(&kinds);
        }
        crate::analytics::tick();
    }
}

/// What this platform can do (CONTRACT §8). Only what has been proven is true.
fn capabilities() -> crate::model::Caps {
    use crate::model::{Caps, Platform};
    if cfg!(windows) {
        // Unverified on a real machine until tested (CONTRACT §8.2c); the polled halo is the plan.
        Caps { platform: Platform::Windows, edge_pill: true, hover_halo: true, dictation: true, global_hotkeys: true, ..Default::default() }
    } else {
        Caps {
            platform: Platform::Linux,
            edge_pill: crate::os::shell::x11_available(),
            dense_glass: true,
            dictation: true,
            global_hotkeys: true,
            ..Default::default()
        }
    }
}

impl eframe::App for Lantern {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(event) = self.engine.events.try_recv() {
            match event {
                Ok(event) => self.engine_event(event),
                Err(error) => self.engine_stopped(format!("{error:#}")),
            }
        }
        for event in self.engine.take_latest() {
            self.engine_event(event);
        }
        while let Ok(event) = self.events.try_recv() {
            self.integration(event);
        }
        self.tick();
        if self.model.new_session.installed.is_some()
            && let Some(id) = self.on_ready.take()
        {
            self.activate(&id);
        }
        self.assets.get_or_insert_with(|| Assets::load(&ctx));
        self.frame(ui);
        for action in std::mem::take(&mut self.actions) {
            self.apply(action);
        }
        // TODO(apollo): wake on engine events instead of polling (the legacy event pump).
        ctx.request_repaint_after(Duration::from_millis(if self.windows.animating() { 16 } else { 250 }));
    }
}

/// `--shell`: the Mac-parity app (pill + panel). The legacy window stays the default until it's done.
/// Runs Lantern. `start`: what the launch asked for (an activation, e.g. "new"), opened once the
/// engine is ready.
pub fn run(terminal: &str, start: Option<&str>) -> Result<()> {
    let start = start.map(str::to_string);
    let engine = Sidecar::with_terminal(terminal)?;
    let options = windows::options();
    eframe::run_native(
        crate::os::shell::PILL_TITLE,
        options,
        Box::new(move |cc| {
            // Windows: Segoe UI Variable when the system has it (fixtures keep the bundled fonts).
            crate::ui::theme::install_native(&cc.egui_ctx);
            let mut lantern = Lantern::new(engine, Some(cc.egui_ctx.clone()));
            lantern.on_ready = start;
            lantern.restore_account();
            lantern.first_launch();
            lantern.start_desktop();
            lantern.refresh_hooks();
            lantern.refresh_notifications();
            Ok(Box::new(lantern))
        }),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
}

/// Preferences and Waiting's seen/dismissed turns, kept across launches.
fn state_path() -> std::path::PathBuf {
    crate::settings::config_dir().join("lantern-state.json")
}

fn load_state() -> (Prefs, Turns) {
    if cfg!(test) {
        return Default::default();
    }
    let Ok(text) = std::fs::read_to_string(state_path()) else {
        let legacy = std::fs::read_to_string(crate::settings::config_dir().join("settings.json")).unwrap_or_default();
        return (first_prefs(&legacy), Turns::default());
    };
    let value: Value = serde_json::from_str(&text).unwrap_or_default();
    let prefs = offered_shortcuts(serde_json::from_value(value["prefs"].clone()).unwrap_or_default());
    (prefs, serde_json::from_value(value["turns"].clone()).unwrap_or_default())
}

/// A saved shortcut Settings doesn't offer (an edited file) goes back to the Mac's default.
fn offered_shortcuts(mut prefs: Prefs) -> Prefs {
    let defaults = Prefs::default();
    if !crate::model::MESSAGE_HOTKEYS.contains(&prefs.message_hotkey.as_str()) {
        prefs.message_hotkey = defaults.message_hotkey;
    }
    if !crate::model::POINT_ASK_HOTKEYS.contains(&prefs.point_ask_hotkey.as_str()) {
        prefs.point_ask_hotkey = defaults.point_ask_hotkey;
    }
    prefs
}

/// A first run: the earlier window's choices when it ran here (it always wrote `welcome_seen`;
/// the dictation setup scripts write only their own keys), otherwise the Mac's defaults.
fn first_prefs(legacy: &str) -> Prefs {
    let value: Value = serde_json::from_str(legacy).unwrap_or_default();
    if value.get("welcome_seen").is_none() {
        return Prefs::default();
    }
    serde_json::from_value(value).map(|old| carried_over(&old)).unwrap_or_default()
}

/// A first run after the earlier window: keep the choices people made there.
fn carried_over(old: &crate::settings::Settings) -> Prefs {
    Prefs {
        notifications: old.notifications,
        notify_finished: old.notify_finished,
        toolkit_in_sessions: old.toolkit,
        bus_for_sessions: old.bus,
        automatic_updates: old.check_updates,
        welcomed: old.welcome_seen,
        open_at_login: crate::settings::autostart_enabled(),
        ..Prefs::default()
    }
}

#[cfg(test)]
mod carry_over_tests {
    #[test]
    fn the_earlier_windows_choices_carry_over() {
        let old: crate::settings::Settings = serde_json::from_str(
            r#"{"notifications":false,"notify_finished":true,"toolkit":false,"bus":true,"check_updates":true,"welcome_seen":true,"scale":1.2,"light":true}"#,
        )
        .unwrap();
        let prefs = super::carried_over(&old);
        assert!(!prefs.notifications && prefs.notify_finished && !prefs.toolkit_in_sessions && prefs.bus_for_sessions);
        assert!(prefs.automatic_updates && prefs.welcomed);
        assert_eq!(prefs.pill_top, 0.72, "everything else starts as on the Mac");
        // A new install, or one where only the dictation setup wrote settings.json, starts as
        // on the Mac rather than with the earlier window's defaults.
        let fresh = crate::model::Prefs::default();
        for legacy in ["", r#"{"whisper":"/x/whisper-cli","whisper_model":"/x/m.bin"}"#] {
            let prefs = super::first_prefs(legacy);
            assert_eq!(
                (prefs.automatic_updates, prefs.bus_for_sessions, prefs.toolkit_in_sessions, prefs.notify_finished),
                (fresh.automatic_updates, fresh.bus_for_sessions, fresh.toolkit_in_sessions, fresh.notify_finished)
            );
        }
        let edited = crate::model::Prefs { message_hotkey: "alt+space".into(), point_ask_hotkey: "none".into(), ..Default::default() };
        let offered = super::offered_shortcuts(edited);
        assert_eq!((offered.message_hotkey.as_str(), offered.point_ask_hotkey.as_str()), ("ctrl+alt+space", "none"));
    }
}

#[cfg(test)]
mod sign_in_tests {
    use super::{Event, Lantern};
    use crate::model::PanelMode;
    use std::sync::atomic::Ordering;

    fn session(email: &str) -> crate::account::Session {
        crate::account::Session {
            access: "access".into(),
            refresh: "refresh".into(),
            email: email.into(),
            expires: std::time::Instant::now() + std::time::Duration::from_secs(3600),
            plan: serde_json::Value::Null,
        }
    }

    fn lantern() -> Lantern {
        let (engine, _) = crate::sidecar::test_sender();
        Lantern::new(engine, None)
    }

    #[test]
    fn signing_in_brings_lantern_back_to_ask_lantern_with_your_question() {
        let mut lantern = lantern();
        lantern.inputs.assistant = "Which agent needs me?".into();
        let generation = lantern.auth.load(Ordering::SeqCst);
        lantern.signing_in = Some(generation);
        let focus = lantern.model.compose_focus;
        lantern.integration(Event::Account { generation, result: Ok(session("me@example.test")) });
        assert_eq!(lantern.model.panel, Some(PanelMode::Assistant));
        assert_eq!(lantern.model.toast.as_ref().map(|toast| toast.text.as_str()), Some("Signed in as me@example.test"));
        assert_eq!(lantern.model.compose_focus, focus + 1, "the Ask box takes focus");
        assert!(lantern.windows.raising_panel());
        assert_eq!(lantern.inputs.assistant, "Which agent needs me?");
        assert!(!lantern.model.account.pending && lantern.model.account.signed_in.is_some());
        // A refresh of the same sign-in is quiet.
        lantern.model.toast = None;
        lantern.integration(Event::Account { generation, result: Ok(session("me@example.test")) });
        assert!(lantern.model.toast.is_none());
    }

    #[test]
    fn signing_in_keeps_the_open_panel_and_names_october_without_an_email() {
        let mut lantern = lantern();
        lantern.model.panel = Some(PanelMode::October);
        let generation = lantern.auth.load(Ordering::SeqCst);
        lantern.signing_in = Some(generation);
        lantern.integration(Event::Account { generation, result: Ok(session("October account")) });
        assert_eq!(lantern.model.panel, Some(PanelMode::October));
        assert_eq!(lantern.model.toast.as_ref().map(|toast| toast.text.as_str()), Some("Signed in to October"));
    }

    #[test]
    fn a_restored_sign_in_or_a_failed_one_stays_quiet() {
        let mut lantern = lantern();
        let generation = lantern.auth.load(Ordering::SeqCst);
        lantern.integration(Event::Account { generation, result: Ok(session("me@example.test")) });
        assert!(lantern.model.panel.is_none() && lantern.model.toast.is_none() && !lantern.windows.raising_panel());
        let mut lantern = self::lantern();
        let generation = lantern.auth.load(Ordering::SeqCst);
        lantern.signing_in = Some(generation);
        lantern.integration(Event::Account { generation, result: Err("Access denied Go back to Lantern and try again.".into()) });
        assert!(lantern.model.panel.is_none() && lantern.model.toast.is_none());
        assert!(lantern.signing_in.is_none());
    }
}
