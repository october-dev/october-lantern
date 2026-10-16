//! New Session and starting agents (Swift `AppModel.startNew`, `launch`, `harnessChanged`,
//! `actionFinished`, `openLaunched`, `waitForLaunched`, `closePreview`, `retryPreviewInTerminal`).
//!
//! One deliberate difference from the Mac (CONTRACT §9.1): an In Lantern launch switches to the
//! list BEFORE showing its preview. On the Mac, setting the panel afterwards closes the chat, which
//! ends the preview it just showed (and its retry).
use super::{Lantern, Pending, assistant::LaunchOutcome};
use crate::model::{AgentKind, Capture, HarnessState, LaunchPreview, PanelMode};
use crate::os::Event;
use crate::ui::SessionRequest;
use serde_json::json;
use std::time::{Duration, Instant};

/// How long a started session's conversation may take to show up before the preview gives way.
const PREVIEW_WAIT: Duration = Duration::from_secs(30);

/// A launch in progress, beyond what the model shows.
#[derive(Default)]
pub(super) struct Launching {
    /// The session Lantern just started: its conversation opens as soon as its agent appears.
    open_when_seen: Option<(String, Instant)>,
    /// The person closed the preview: the session still starts, but its chat doesn't open.
    preview_closed: bool,
    /// Starts the previewed session again in a Terminal window (after it failed In Lantern).
    retry: Option<SessionRequest>,
    /// The session started; the preview gives way if its conversation hasn't by then.
    preview_until: Option<Instant>,
    /// October needs October Harness first; this launch starts when it's ready.
    after_harness: Option<SessionRequest>,
    /// The screenshot capture whose result counts (older ones are stale).
    capture: u64,
    /// Ask Lantern's launch in flight (the turn it's for): told how it went, and the panel stays
    /// where it is (Swift `launchReport`).
    report: Option<u64>,
}

impl Lantern {
    /// Opens New Session (or closes it), reading the app you're in first, before anything of
    /// Lantern's comes to the front.
    pub(super) fn start_new_session(&mut self) {
        if self.model.panel == Some(PanelMode::NewSession) {
            return self.set_panel(None);
        }
        let task_app = crate::os::foreground_app().filter(|app| !app.is_terminal);
        let prefs = &self.model.prefs;
        let installed = self.model.new_session.installed.clone().unwrap_or_default();
        let kind = installed.first().cloned();
        let form = &mut self.inputs.new_session;
        form.task = task_app.is_some();
        form.screenshot = form.task || prefs.screenshot_new_sessions;
        form.kind = kind.clone();
        form.model = kind.as_ref().and_then(|kind| prefs.model_by_kind.get(kind.as_str()).cloned());
        form.open_in_terminal = prefs.open_in_terminal || !self.model.new_session.in_lantern_available;
        // No folder chosen yet: the form applies the Mac's precedence itself (a Task's document
        // folder, then that app's last task folder, then Documents/Lantern Tasks/<app>, else the
        // first offered folder), so a pre-filled folder must not override it.
        form.folder = None;
        self.model.new_session.task_app = task_app;
        self.model.new_session.launch_error = None;
        if let Some(kind) = &kind {
            self.load_models(kind);
        }
        self.set_panel(Some(PanelMode::NewSession));
        if self.inputs.new_session.screenshot {
            self.capture_screen();
        } else {
            self.model.new_session.screenshot = Capture::None;
        }
    }

    pub(super) fn load_models(&mut self, kind: &AgentKind) {
        if !self.model.new_session.models.contains_key(kind.as_str()) {
            self.request(json!({"type": "models", "kind": kind.as_str()}));
        }
    }

    /// The agent changed: its last model comes back, and its models are listed.
    pub(super) fn choose_session_agent(&mut self, kind: AgentKind) {
        self.inputs.new_session.model = self.model.prefs.model_by_kind.get(kind.as_str()).cloned();
        self.load_models(&kind);
        self.inputs.new_session.kind = Some(kind);
    }

    /// Captures the screen for New Session, with Lantern's own windows hidden for the moment it
    /// takes (the Mac leaves its windows out of the capture).
    pub(super) fn capture_screen(&mut self) {
        self.launching.capture += 1;
        let generation = self.launching.capture;
        self.model.new_session.screenshot = Capture::Capturing;
        self.windows.hide_for_capture(move |tx| {
            crate::os::task(tx, crate::os::capture_screen, move |result| Event::Screenshot { owner: "__new".into(), generation, result });
        });
    }

    pub(super) fn screenshot_captured(&mut self, generation: u64, result: Result<std::path::PathBuf, String>) {
        super::trace!("capture {generation} (current {}): {result:?}", self.launching.capture);
        if generation != self.launching.capture {
            return; // a newer capture (or turning it off) took over
        }
        self.model.new_session.screenshot = match result {
            Ok(path) if self.inputs.new_session.screenshot => Capture::Ready { path },
            Ok(_) => Capture::None,
            Err(message) if message.contains("denied") || message.contains("cancelled") => Capture::PermissionNeeded,
            Err(message) => Capture::Failed { message },
        };
    }

    pub(super) fn set_session_screenshot(&mut self, on: bool) {
        self.inputs.new_session.screenshot = on;
        if on {
            self.capture_screen();
        } else {
            self.launching.capture += 1;
            self.model.new_session.screenshot = Capture::None;
        }
    }

    /// Starts an agent. In Lantern, its conversation shows right away and the real one replaces it
    /// once the agent is up. False when another launch is still under way.
    pub(super) fn launch(&mut self, request: SessionRequest) -> bool {
        if self.model.new_session.launching || self.pending.values().any(|p| matches!(p, Pending::Launch)) {
            return false;
        }
        // Ask Lantern's launch shows in its own conversation: no preview, the panel stays.
        self.launching.report = request.for_turn;
        if let Some(turn) = request.for_turn {
            super::trace!("launch for Ask Lantern's turn {turn}");
        } else if request.background {
            // §9.1: the list first (that closes any chat), then the preview.
            if !self.model.panel.is_some_and(PanelMode::is_list) {
                self.set_panel(Some(PanelMode::Inbox));
            }
            self.close_chat();
            self.model.launch_preview = Some(LaunchPreview {
                kind: request.kind.clone(),
                prompt: request.prompt.clone(),
                folder: request.folder.clone(),
                error: None,
            });
            self.launching.preview_closed = false;
            self.launching.retry = Some(SessionRequest { background: false, ..request.clone() });
        } else if self.model.launch_preview.is_some() {
            self.end_preview();
        }
        // October without October Harness yet: install it first, then launch (the engine's launch
        // deadline is too short for a first install).
        let installed_known = self.model.new_session.installed.is_some();
        if request.kind.as_str() == AgentKind::OCTOBER && installed_known && !self.october_ready() {
            self.model.new_session.launching = true;
            self.model.new_session.launch_error = None;
            if self.model.harness.as_ref().is_none_or(|h| h.status != HarnessState::Installing) {
                self.request(json!({"type": "harness.prepare"}));
            }
            self.launching.after_harness = Some(request);
            return true;
        }
        // A task's own folder (Documents/Lantern Tasks/<app>) is made when its first task starts.
        if let Err(error) = make_tasks_folder(&request.folder, &crate::settings::home()) {
            self.launch_failed(error);
            return true;
        }
        self.remember_launch(&request);
        let context = request.context.as_ref().map(|context| context.summary(self.model.caps.platform));
        let sent = self.request(json!({
            "type": "launch", "kind": request.kind.as_str(), "cwd": request.folder, "prompt": request.prompt,
            "screenshot": request.screenshot, "model": request.model, "context": context,
            "toolkit": request.toolkit, "bus": request.bus, "computer": self.model.prefs.computer_use, "background": request.background,
        }));
        match sent {
            Some(id) => {
                self.model.new_session.launching = true;
                self.model.new_session.launch_error = None;
                self.pending.insert(id, Pending::Launch);
            }
            None => self.launch_failed("Lantern's engine isn't running.".into()),
        }
        true
    }

    /// Task folder per app, last model per agent, recent folders and agents.
    fn remember_launch(&mut self, request: &SessionRequest) {
        let prefs = &mut self.model.prefs;
        if let Some(app) = &request.task_app {
            prefs.task_folder_by_app.insert(app.clone(), request.folder.clone());
        }
        match &request.model {
            Some(model) => prefs.model_by_kind.insert(request.kind.0.clone(), model.clone()),
            None => prefs.model_by_kind.remove(request.kind.as_str()),
        };
        prefs.recent_folders.retain(|folder| folder != &request.folder);
        prefs.recent_folders.insert(0, request.folder.clone());
        prefs.recent_folders.truncate(8);
        prefs.recent_kinds.retain(|kind| kind != request.kind.as_str());
        prefs.recent_kinds.insert(0, request.kind.0.clone());
        self.save_prefs();
        self.refresh_folders();
    }

    pub(super) fn refresh_folders(&mut self) {
        let home = crate::settings::home().to_string_lossy().into_owned();
        self.model.new_session.folders = crate::model::offered_folders(&self.model.prefs, &self.model.agents, &home);
    }

    /// October can start now: on PATH, or October Harness installed.
    fn october_ready(&self) -> bool {
        self.model.harness.as_ref().is_some_and(|h| h.status == HarnessState::Ready)
            || self.model.new_session.installed.as_ref().is_some_and(|kinds| kinds.iter().any(|k| k.as_str() == AgentKind::OCTOBER))
    }

    /// The harness changed: a launch that was waiting for it goes ahead, or fails.
    pub(super) fn harness_changed(&mut self) {
        let Some(status) = self.model.harness.as_ref().map(|h| (h.status, h.message.clone())) else { return };
        if self.launching.after_harness.is_none() {
            return;
        }
        match status {
            (HarnessState::Ready, _) => {
                let request = self.launching.after_harness.take().expect("checked");
                self.model.new_session.launching = false;
                self.launch(request);
            }
            (HarnessState::Failed, message) => {
                self.launching.after_harness = None;
                self.launch_failed(format!("Couldn't install October Harness: {}", message.unwrap_or_else(|| "unknown error".into())));
            }
            _ => {}
        }
    }

    fn launch_failed(&mut self, message: String) {
        self.model.new_session.launching = false;
        if let Some(turn) = self.launching.report.take() {
            return self.assistant_launched(turn, LaunchOutcome::Failed(message));
        }
        self.model.new_session.launch_error = Some(message.clone());
        if let Some(preview) = &mut self.model.launch_preview {
            preview.error = Some(message);
        }
    }

    /// The engine's answer to a launch.
    pub(super) fn launch_result(&mut self, ok: bool, message: Option<String>, session: Option<String>) {
        self.model.new_session.launching = false;
        if !ok {
            return self.launch_failed(message.unwrap_or_else(|| "unknown error".into()));
        }
        if let Some(turn) = self.launching.report.take() {
            return self.assistant_launched(turn, LaunchOutcome::Started { session });
        }
        if let Some(session) = session.filter(|_| !self.launching.preview_closed) {
            self.launching.open_when_seen = Some((session, Instant::now() + PREVIEW_WAIT));
            self.open_launched();
        }
        if self.model.launch_preview.is_some() {
            self.launching.preview_until = Some(Instant::now() + PREVIEW_WAIT);
        } else {
            if !self.launching.preview_closed {
                self.set_panel(Some(PanelMode::Agents));
            }
            self.launching.preview_closed = false;
            self.show(message.map_or("Session started".into(), |message| format!("Session started. {message}")));
        }
    }

    /// Opens the conversation of the session Lantern just started, once it appears.
    pub(super) fn open_launched(&mut self) {
        let Some((session, until)) = &self.launching.open_when_seen else { return };
        if Instant::now() > *until {
            self.launching.open_when_seen = None;
            return;
        }
        let Some(id) = self.model.agents.iter().find(|agent| agent.in_session(session)).map(|agent| agent.id.clone()) else { return };
        self.launching.open_when_seen = None;
        // The preview stays until the real conversation has something to show (history_result).
        let preview = self.model.launch_preview.take();
        self.open_chat(&id);
        self.model.launch_preview = preview;
    }

    /// Called every frame: a preview whose conversation never showed up gives way.
    pub(super) fn launch_tick(&mut self) {
        if self.launching.preview_until.is_some_and(|until| Instant::now() >= until) {
            self.launching.preview_until = None;
            if self.model.launch_preview.is_some() {
                self.end_preview();
                if self.model.chat.is_none() {
                    self.set_panel(Some(PanelMode::Agents));
                    self.show("Session started. It shows in Your agents when it's up.");
                }
            }
        }
    }

    /// Closes a starting session's preview; it keeps starting (and shows in Your agents) but its
    /// conversation won't open by itself.
    pub(super) fn close_preview(&mut self) {
        if self.model.new_session.launching || self.pending.values().any(|p| matches!(p, Pending::Launch)) {
            self.launching.preview_closed = true;
        }
        self.launching.open_when_seen = None;
        self.end_preview();
        self.close_chat();
        if self.model.panel == Some(PanelMode::Inbox) {
            self.set_panel(Some(PanelMode::Agents));
        }
    }

    /// Starts the previewed session again, in a Terminal window (after it failed In Lantern).
    pub(super) fn retry_preview_in_terminal(&mut self) {
        if self.model.new_session.launching {
            return;
        }
        if let Some(retry) = self.launching.retry.take() {
            self.end_preview();
            self.launch(retry);
        }
    }

    pub(super) fn end_preview(&mut self) {
        self.launching.preview_until = None;
        self.launching.retry = None;
        self.model.launch_preview = None;
    }
}

/// Creates `folder` when it's one of Lantern's own task folders (under Documents/Lantern Tasks)
/// that doesn't exist yet. Any other folder is left as it is.
fn make_tasks_folder(folder: &str, home: &std::path::Path) -> Result<(), String> {
    let path = std::path::Path::new(folder);
    if path.exists() || !path.starts_with(home.join("Documents").join("Lantern Tasks")) {
        return Ok(());
    }
    std::fs::create_dir_all(path).map_err(|error| format!("Couldn't create {folder}: {error}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_tasks_folder_is_made_when_its_first_task_starts() {
        let home = std::env::temp_dir().join(format!("lantern-home-{}", uuid::Uuid::new_v4().simple()));
        let chrome = home.join("Documents").join("Lantern Tasks").join("Google Chrome");
        super::make_tasks_folder(&chrome.to_string_lossy(), &home).unwrap();
        assert!(chrome.is_dir());
        let elsewhere = home.join("project");
        super::make_tasks_folder(&elsewhere.to_string_lossy(), &home).unwrap();
        assert!(!elsewhere.exists(), "only Lantern's own task folders are made");
        std::fs::remove_dir_all(&home).unwrap();
    }

    use super::*;
    use crate::model::{Agent, AgentState, TmuxPane};
    use serde_json::Value;

    fn lantern() -> Lantern {
        let (engine, _) = crate::sidecar::test_sender();
        let mut lantern = Lantern::new(engine, None);
        lantern.connected = true;
        lantern.model.new_session.installed = Some(vec![AgentKind::new("claude")]);
        lantern.model.new_session.in_lantern_available = true;
        lantern
    }

    fn request(background: bool) -> SessionRequest {
        SessionRequest { kind: AgentKind::new("claude"), folder: "/p".into(), prompt: "fix it".into(), background, ..Default::default() }
    }

    /// The engine answers the pending launch (through the real event path).
    fn answer(lantern: &mut Lantern, ok: bool, message: Option<&str>, session: Option<&str>) {
        let id = lantern.pending.iter().find(|(_, p)| matches!(p, Pending::Launch)).map(|(id, _)| id.clone()).expect("a launch is pending");
        lantern.engine_event(json!({"type": "launchResult", "requestId": id, "ok": ok, "message": message, "session": session}));
    }

    /// What the coordinator sent the (echo) engine, skipping the hello.
    fn sent(lantern: &Lantern) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(Ok(event)) = lantern.engine.events.recv_timeout(Duration::from_millis(300)) {
            out.push(event);
        }
        out
    }

    #[test]
    fn an_in_lantern_launch_from_new_session_keeps_its_preview_and_retry() {
        let mut lantern = lantern();
        lantern.set_panel(Some(PanelMode::NewSession));
        lantern.launch(request(true));
        // The Mac's ordering would have closed the chat after showing the preview, ending it.
        assert_eq!(lantern.model.panel, Some(PanelMode::Inbox));
        assert_eq!(lantern.model.launch_preview.as_ref().map(|p| p.prompt.as_str()), Some("fix it"));
        assert!(lantern.launching.retry.as_ref().is_some_and(|retry| !retry.background));
        assert!(lantern.model.new_session.launching);
        let launch = sent(&lantern).into_iter().find(|event| event["type"] == "launch").expect("launch sent");
        assert_eq!(launch["background"], true);
        assert_eq!(launch["cwd"], "/p");
    }

    #[test]
    fn duplicate_launches_are_refused_while_one_is_pending() {
        let mut lantern = lantern();
        lantern.launch(request(false));
        lantern.launch(request(false));
        assert_eq!(sent(&lantern).iter().filter(|event| event["type"] == "launch").count(), 1);
    }

    #[test]
    fn a_failed_launch_keeps_the_preview_with_its_error_and_retry() {
        let mut lantern = lantern();
        lantern.launch(request(true));
        answer(&mut lantern, false, Some("tmux couldn't start the session"), None);
        let preview = lantern.model.launch_preview.as_ref().unwrap();
        assert_eq!(preview.error.as_deref(), Some("tmux couldn't start the session"));
        assert!(!lantern.model.new_session.launching);
        lantern.retry_preview_in_terminal();
        assert!(lantern.model.launch_preview.is_none());
        let launches: Vec<Value> = sent(&lantern).into_iter().filter(|event| event["type"] == "launch").collect();
        assert_eq!(launches.len(), 2);
        assert_eq!(launches[1]["background"], false, "the retry opens a Terminal window");
    }

    #[test]
    fn the_started_session_opens_when_its_agent_appears_and_the_preview_waits_for_history() {
        let mut lantern = lantern();
        lantern.launch(request(true));
        answer(&mut lantern, true, None, Some("claude-p-1"));
        let agent = Agent {
            id: "claude:9:9".into(),
            kind: AgentKind::new("claude"),
            state: AgentState::Working,
            tmux: Some(TmuxPane { target: "claude-p-1:0.0".into(), ..Default::default() }),
            ..Default::default()
        };
        lantern.engine_event(json!({"type": "snapshot", "agents": [agent]}));
        assert_eq!(lantern.model.chat.as_ref().map(|chat| chat.agent_id.as_str()), Some("claude:9:9"));
        assert!(lantern.model.launch_preview.is_some(), "the preview stays until the conversation has something to show");
    }

    #[test]
    fn closing_the_preview_keeps_the_launch_but_not_its_chat() {
        let mut lantern = lantern();
        lantern.launch(request(true));
        lantern.close_preview();
        assert!(lantern.model.launch_preview.is_none());
        assert_eq!(lantern.model.panel, Some(PanelMode::Agents));
        answer(&mut lantern, true, None, Some("claude-p-1"));
        assert!(lantern.launching.open_when_seen.is_none(), "a closed preview never reopens by itself");
    }

    #[test]
    fn a_terminal_launch_goes_to_your_agents_with_a_toast() {
        let mut lantern = lantern();
        lantern.set_panel(Some(PanelMode::NewSession));
        lantern.launch(request(false));
        answer(&mut lantern, true, Some("Started without October Bus"), None);
        assert_eq!(lantern.model.panel, Some(PanelMode::Agents));
        assert_eq!(lantern.model.toast.as_ref().map(|t| t.text.as_str()), Some("Session started. Started without October Bus"));
    }

    #[test]
    fn october_waits_for_its_harness_then_starts() {
        let mut lantern = lantern();
        let october = SessionRequest { kind: AgentKind::new("october"), ..request(false) };
        lantern.launch(october);
        let first = sent(&lantern);
        assert!(first.iter().any(|event| event["type"] == "harness.prepare"));
        assert!(!first.iter().any(|event| event["type"] == "launch"));
        lantern.engine_event(json!({"type": "harness", "status": "ready", "source": "bundled"}));
        assert!(sent(&lantern).iter().any(|event| event["type"] == "launch" && event["kind"] == "october"));
    }

    #[test]
    fn launching_remembers_folder_model_and_agent() {
        let mut lantern = lantern();
        lantern.launch(SessionRequest { model: Some("opus".into()), task_app: Some("resolve".into()), ..request(false) });
        let prefs = &lantern.model.prefs;
        assert_eq!(prefs.recent_folders.first().map(String::as_str), Some("/p"));
        assert_eq!(prefs.model_by_kind.get("claude").map(String::as_str), Some("opus"));
        assert_eq!(prefs.task_folder_by_app.get("resolve").map(String::as_str), Some("/p"));
        assert_eq!(prefs.recent_kinds.first().map(String::as_str), Some("claude"));
    }

    #[test]
    fn new_session_leaves_the_folder_to_the_forms_precedence() {
        let mut lantern = lantern();
        lantern.inputs.new_session.folder = Some("/old/choice".into());
        lantern.start_new_session();
        assert_eq!(lantern.model.panel, Some(PanelMode::NewSession));
        assert_eq!(lantern.inputs.new_session.folder, None);
        assert_eq!(lantern.inputs.new_session.kind.as_ref().map(AgentKind::as_str), Some("claude"));
        lantern.start_new_session();
        assert_eq!(lantern.model.panel, None, "+ again closes it");
    }

    #[test]
    fn a_stale_capture_never_overwrites_a_newer_one() {
        let mut lantern = lantern();
        lantern.inputs.new_session.screenshot = true;
        lantern.launching.capture = 2;
        lantern.screenshot_captured(1, Ok("/old.png".into()));
        assert_eq!(lantern.model.new_session.screenshot, Capture::None);
        lantern.screenshot_captured(2, Ok("/new.png".into()));
        assert_eq!(lantern.model.new_session.screenshot, Capture::Ready { path: "/new.png".into() });
    }
}
