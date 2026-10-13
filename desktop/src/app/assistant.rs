//! Ask Lantern (Swift `Assistant.swift` and `AppModel.defaultAgentKind`): a chat with October's AI
//! that knows what your agents are doing, and starts a task when the answer asks for one, after a
//! three-second window to cancel.
//!
//! One deliberate difference from the Mac (CONTRACT §9.2): "Started <agent> in <folder>" is written
//! only once the launch succeeded; a failed one shows its error instead.
use super::Lantern;
use crate::model::{AgentKind, AgentState, AssistantTurn, HarnessState, PendingTask, Platform};
use crate::october_ai::{self, Failure, Piece};
use crate::os::Event;
use crate::ui::SessionRequest;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Seconds a proposed task waits before it starts.
const CANCEL_WINDOW: u32 = 3;

#[derive(Default)]
pub(super) struct Asking {
    /// The question being answered; a newer one (or New question) makes older answers stale.
    generation: Arc<AtomicU64>,
    next_id: u64,
    /// When the pending task's countdown started.
    countdown: Option<Instant>,
    /// The turn whose task is launching, and its "Started …" text for when it succeeds.
    pub launching: Option<(u64, String)>,
}

impl Lantern {
    pub(super) fn submit_assistant(&mut self) {
        let question = self.inputs.assistant.trim().to_string();
        let Some(token) = self.session.as_ref().map(|session| session.access.clone()) else { return };
        if question.is_empty() || self.model.assistant.asking {
            return;
        }
        self.inputs.assistant.clear();
        self.asking.next_id += 1;
        let turn = self.asking.next_id;
        let earlier: Vec<(String, String)> =
            self.model.assistant.turns.iter().filter(|t| !t.failed).map(|t| (t.question.clone(), t.answer.clone())).collect();
        self.model.assistant.turns.push(AssistantTurn { id: turn, question: question.clone(), ..Default::default() });
        self.model.assistant.asking = true;
        let request = october_ai::Request {
            turns: earlier,
            question,
            context: agent_summary(&self.model.agents, self.model.caps.platform),
            mode: Some("chat".into()),
            platform: self.model.caps.platform,
            ..Default::default()
        };
        let generation = self.asking.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let current = self.asking.generation.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let pieces = tx.clone();
            let result = october_ai::stream(&request, &token, &mut |piece| {
                current.load(Ordering::SeqCst) == generation && pieces.send(Event::Assistant { generation, turn, piece }).is_ok()
            });
            let error = result.err().filter(|failure| *failure != Failure::Cancelled).map(|failure| failure.to_string());
            let _ = tx.send(Event::AssistantDone { generation, turn, error });
        });
    }

    pub(super) fn assistant_piece(&mut self, generation: u64, turn: u64, piece: Piece) {
        if generation != self.asking.generation.load(Ordering::SeqCst) {
            return;
        }
        let Some(index) = self.model.assistant.turns.iter().position(|t| t.id == turn) else { return };
        match piece {
            Piece::Text(text) => self.model.assistant.turns[index].answer.push_str(&text),
            Piece::Note(note) => self.model.assistant.turns[index].note = Some(note),
            Piece::Task { prompt, agent, project } => self.propose(prompt, agent, project, turn),
        }
    }

    pub(super) fn assistant_done(&mut self, generation: u64, turn: u64, error: Option<String>) {
        if generation != self.asking.generation.load(Ordering::SeqCst) {
            return;
        }
        if let Some(t) = self.model.assistant.turns.iter_mut().find(|t| t.id == turn) {
            match error {
                Some(error) => {
                    t.answer = error;
                    t.failed = true;
                }
                None if t.answer.is_empty() => {
                    t.answer = "No answer came back. Try again.".into();
                    t.failed = true;
                }
                None => {}
            }
        }
        self.model.assistant.asking = false;
    }

    /// The answer asked for work: start it after a short window to cancel. The agent gets your own
    /// words (see `task_prompt`).
    fn propose(&mut self, prompt: String, agent: Option<String>, project: Option<String>, turn: u64) {
        let installed = self.model.new_session.installed.clone().unwrap_or_default();
        let named = agent.map(AgentKind).filter(|kind| installed.contains(kind));
        let Some(kind) = named.or_else(|| self.default_agent_kind()) else { return };
        let folder = self.task_folder(project.as_deref());
        let turns = &self.model.assistant.turns;
        let prompt = match turns.iter().position(|t| t.id == turn) {
            Some(i) => task_prompt(&turns[i].question, prompt, i > 0),
            None => prompt,
        };
        self.asking.next_id += 1;
        self.model.assistant.pending_task =
            Some(PendingTask { id: self.asking.next_id, turn, prompt, kind, folder, seconds_left: CANCEL_WINDOW });
        self.asking.countdown = Some(Instant::now());
    }

    /// Counts the pending task down, and starts it at zero.
    pub(super) fn assistant_tick(&mut self) {
        let (Some(started), Some(pending)) = (self.asking.countdown, self.model.assistant.pending_task.as_mut()) else { return };
        let left = CANCEL_WINDOW.saturating_sub(started.elapsed().as_secs() as u32);
        pending.seconds_left = left.max(1);
        if left == 0 {
            self.start_pending_task();
        }
    }

    pub(super) fn cancel_assistant_task(&mut self) {
        self.asking.countdown = None;
        self.model.assistant.pending_task = None;
    }

    pub(super) fn start_pending_task(&mut self) {
        self.asking.countdown = None;
        let Some(task) = self.model.assistant.pending_task.take() else { return };
        // In Lantern when possible (the chat opens here); otherwise a Terminal window.
        let request = SessionRequest {
            kind: task.kind.clone(),
            folder: task.folder.clone(),
            prompt: task.prompt,
            toolkit: true,
            bus: self.model.prefs.bus_for_sessions,
            background: self.model.new_session.in_lantern_available,
            ..Default::default()
        };
        if self.launch(request) {
            let folder = std::path::Path::new(&task.folder).file_name().map_or(task.folder.clone(), |f| f.to_string_lossy().into_owned());
            self.asking.launching = Some((task.turn, format!("Started {} in {folder}", task.kind.display_name())));
        } else if let Some(turn) = self.model.assistant.turns.iter_mut().find(|t| t.id == task.turn) {
            turn.note = Some("Another session is still starting. Try again in a moment.".into());
        }
    }

    /// The task's launch finished (from `launch_result`).
    pub(super) fn assistant_launch_finished(&mut self, error: Option<&str>) {
        let Some((turn, started)) = self.asking.launching.take() else { return };
        if let Some(t) = self.model.assistant.turns.iter_mut().find(|t| t.id == turn) {
            match error {
                None => t.started = Some(started),
                Some(error) => t.note = Some(format!("Couldn't start it: {error}")),
            }
        }
    }

    /// New question: stops the answer and any pending task, and clears the conversation.
    pub(super) fn clear_assistant(&mut self) {
        self.asking.generation.fetch_add(1, Ordering::SeqCst);
        self.cancel_assistant_task();
        self.model.assistant.turns.clear();
        self.model.assistant.asking = false;
    }

    /// The agent new tasks start with (Swift `defaultAgentKind`): the preference if it can start
    /// (October counts once October Harness is ready, or will be installed because there's no
    /// Claude Code or Codex), else Claude Code, else Codex, else the first installed agent.
    pub(super) fn default_agent_kind(&self) -> Option<AgentKind> {
        let installed = self.model.new_session.installed.clone().unwrap_or_default();
        let harness = self.model.harness.as_ref().map(|h| h.status);
        let usable = |kind: &AgentKind| {
            installed.contains(kind)
                || kind.as_str() == AgentKind::OCTOBER
                    && (harness == Some(HarnessState::Ready) || self.needs_harness() && harness != Some(HarnessState::Failed))
        };
        [AgentKind(self.model.prefs.default_task_agent.clone()), AgentKind::new(AgentKind::CLAUDE), AgentKind::new(AgentKind::CODEX)]
            .into_iter()
            .find(usable)
            .or_else(|| installed.first().cloned())
    }

    /// Neither Claude Code nor Codex is installed: October Harness is what this computer can run.
    pub(super) fn needs_harness(&self) -> bool {
        self.model
            .new_session
            .installed
            .as_ref()
            .is_some_and(|kinds| !kinds.iter().any(|k| [AgentKind::CLAUDE, AgentKind::CODEX].contains(&k.as_str())))
    }

    /// The project the person named (matched against recent folders, then agents' folders, newest
    /// first); otherwise the home folder, where the agent can look anywhere.
    fn task_folder(&self, project: Option<&str>) -> String {
        let home = crate::settings::home().to_string_lossy().into_owned();
        let mut agents: Vec<_> = self.model.agents.iter().collect();
        agents.sort_by(|a, b| b.state_since.unwrap_or(0.0).total_cmp(&a.state_since.unwrap_or(0.0)));
        let candidates: Vec<String> =
            self.model.new_session.folders.iter().cloned().chain(agents.into_iter().filter_map(|agent| agent.cwd.clone())).collect();
        let name =
            |folder: &String| std::path::Path::new(folder).file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
        match project.map(str::to_lowercase).filter(|project| !project.is_empty()) {
            Some(project) => candidates
                .iter()
                .find(|folder| name(folder) == project)
                .or_else(|| candidates.iter().find(|folder| name(folder).contains(&project)))
                .cloned()
                .unwrap_or(home),
            None => home,
        }
    }
}

/// What the agents are doing, for the model: one line each, most urgent first (Swift `agentSummary`).
pub fn agent_summary(agents: &[crate::model::Agent], platform: Platform) -> String {
    let computer = match platform {
        Platform::Windows => "this Windows PC",
        Platform::Linux => "this Linux computer",
    };
    let rank = |state: AgentState| {
        if state.wants_you() {
            0
        } else if state == AgentState::Working {
            1
        } else {
            2
        }
    };
    let mut live: Vec<_> = agents.iter().filter(|agent| agent.is_live()).collect();
    live.sort_by_key(|agent| rank(agent.state));
    if live.is_empty() {
        return format!("Lantern's assistant chat (no screenshot). No coding agents are running on {computer}.");
    }
    let mut lines = vec![format!("Lantern's assistant chat (no screenshot). The user's coding agents on {computer} right now:")];
    for agent in live.into_iter().take(20) {
        let mut line = format!(
            "- @{} ({}) in {}: {}",
            agent.handle,
            agent.kind.display_name(),
            agent.project.as_deref().unwrap_or("?"),
            agent.state.label()
        );
        if let Some(title) = &agent.title {
            line.push_str(&format!("; working on \"{}\"", title.chars().take(80).collect::<String>()));
        }
        if let Some(question) = agent.question.as_ref().filter(|_| agent.state.wants_you()) {
            line.push_str(&format!("; asking: {}", question.chars().take(160).collect::<String>()));
        }
        if let Some(message) = &agent.last_message {
            line.push_str(&format!("; last said: {}", message.replace('\n', " ").chars().take(200).collect::<String>()));
        }
        lines.push(line);
    }
    lines.join("\n").chars().take(6000).collect()
}

/// What the agent is asked: your own words when you asked for it outright, not October's
/// rewording ("find the user's thesis"). A follow-up ("yes, do that") only makes sense with the
/// conversation, so then October's self-contained version is used (Swift `Assistant.taskPrompt`).
fn task_prompt(question: &str, suggested: String, follow_up: bool) -> String {
    let mine = question.trim();
    if follow_up || mine.is_empty() { suggested } else { mine.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Agent, HarnessStatus};

    fn lantern(installed: &[&str]) -> Lantern {
        let (engine, _) = crate::sidecar::test_sender();
        let mut lantern = Lantern::new(engine, None);
        lantern.connected = true;
        lantern.model.new_session.installed = Some(installed.iter().map(|k| AgentKind::new(k)).collect());
        lantern.model.new_session.in_lantern_available = true;
        lantern
    }

    /// A question being answered: its generation and a fresh turn id.
    fn with_turn(lantern: &mut Lantern) -> (u64, u64) {
        lantern.asking.next_id += 1;
        let id = lantern.asking.next_id;
        lantern.model.assistant.turns.push(AssistantTurn { id, question: "q".into(), ..Default::default() });
        lantern.model.assistant.asking = true;
        (lantern.asking.generation.fetch_add(1, Ordering::SeqCst) + 1, id)
    }

    #[test]
    fn default_agent_prefers_the_setting_then_claude_codex_and_october_when_it_must() {
        let mut both = lantern(&["codex", "claude", "gemini"]);
        both.model.prefs.default_task_agent = "october".into();
        assert_eq!(both.default_agent_kind(), Some(AgentKind::new("claude")), "October isn't installed and isn't needed");
        let mut only_gemini = lantern(&["gemini"]);
        only_gemini.model.prefs.default_task_agent = "october".into();
        assert_eq!(only_gemini.default_agent_kind(), Some(AgentKind::new("october")), "no Claude or Codex: the harness will install");
        only_gemini.model.harness = Some(HarnessStatus { status: HarnessState::Failed, ..Default::default() });
        assert_eq!(only_gemini.default_agent_kind(), Some(AgentKind::new("gemini")), "a failed harness install falls back");
    }

    #[test]
    fn answers_stream_into_their_turn_and_stale_ones_are_ignored() {
        let mut lantern = lantern(&["claude"]);
        let (generation, turn) = with_turn(&mut lantern);
        lantern.assistant_piece(generation, turn, Piece::Text("Hello ".into()));
        lantern.assistant_piece(generation - 1, turn, Piece::Text("stale".into()));
        lantern.assistant_piece(generation, turn, Piece::Text("there".into()));
        lantern.assistant_piece(generation, turn, Piece::Note("2 free questions left today.".into()));
        lantern.assistant_done(generation, turn, None);
        let t = &lantern.model.assistant.turns[0];
        assert_eq!(t.answer, "Hello there");
        assert_eq!(t.note.as_deref(), Some("2 free questions left today."));
        assert!(!lantern.model.assistant.asking);
    }

    #[test]
    fn an_empty_or_failed_answer_says_so() {
        let mut lantern = lantern(&["claude"]);
        let (generation, turn) = with_turn(&mut lantern);
        lantern.assistant_done(generation, turn, None);
        assert_eq!(lantern.model.assistant.turns[0].answer, "No answer came back. Try again.");
        let (generation, turn) = with_turn(&mut lantern);
        lantern.assistant_done(generation, turn, Some("Sign in to October to ask.".into()));
        assert!(lantern.model.assistant.turns.last().unwrap().failed);
    }

    #[test]
    fn a_task_goes_to_the_agent_in_your_words_unless_it_follows_up() {
        assert_eq!(super::task_prompt(" Find me my thesis ", "Find the user's thesis".into(), false), "Find me my thesis");
        assert_eq!(super::task_prompt("yes, do that", "Fix the Wi-Fi".into(), true), "Fix the Wi-Fi");
        let mut lantern = lantern(&["claude"]);
        let (generation, turn) = with_turn(&mut lantern);
        lantern.assistant_piece(generation, turn, Piece::Task { prompt: "Find the user's q".into(), agent: None, project: None });
        assert_eq!(lantern.model.assistant.pending_task.clone().unwrap().prompt, "q");
    }

    #[test]
    fn a_task_waits_three_seconds_then_starts_and_says_so_only_when_it_launched() {
        let mut lantern = lantern(&["claude"]);
        lantern.model.new_session.folders = vec!["/work/october-lantern".into()];
        let (generation, turn) = with_turn(&mut lantern);
        let task = Piece::Task { prompt: "fix the tests".into(), agent: None, project: Some("October-Lantern".into()) };
        lantern.assistant_piece(generation, turn, task);
        let pending = lantern.model.assistant.pending_task.clone().unwrap();
        assert_eq!((pending.kind.as_str(), pending.folder.as_str(), pending.seconds_left), ("claude", "/work/october-lantern", 3));
        lantern.start_pending_task();
        assert!(lantern.model.assistant.pending_task.is_none());
        assert_eq!(lantern.model.assistant.turns[0].started, None, "not before the launch succeeded");
        let id = lantern.pending.iter().find(|(_, p)| matches!(p, super::super::Pending::Launch)).map(|(id, _)| id.clone()).unwrap();
        lantern.engine_event(serde_json::json!({"type": "launchResult", "requestId": id, "ok": true, "session": "claude-x-1"}));
        assert_eq!(lantern.model.assistant.turns[0].started.as_deref(), Some("Started Claude Code in october-lantern"));
    }

    #[test]
    fn a_failed_task_launch_shows_its_error_not_started() {
        let mut lantern = lantern(&["claude"]);
        let (generation, turn) = with_turn(&mut lantern);
        lantern.assistant_piece(generation, turn, Piece::Task { prompt: "x".into(), agent: Some("claude".into()), project: None });
        lantern.start_pending_task();
        let id = lantern.pending.iter().find(|(_, p)| matches!(p, super::super::Pending::Launch)).map(|(id, _)| id.clone()).unwrap();
        lantern.engine_event(serde_json::json!({"type": "launchResult", "requestId": id, "ok": false, "message": "tmux is missing"}));
        let t = &lantern.model.assistant.turns[0];
        assert_eq!(t.started, None);
        assert_eq!(t.note.as_deref(), Some("Couldn't start it: tmux is missing"));
    }

    #[test]
    fn no_named_project_starts_in_home_and_an_uninstalled_agent_falls_back() {
        let mut lantern = lantern(&["codex"]);
        let (generation, turn) = with_turn(&mut lantern);
        lantern.assistant_piece(
            generation,
            turn,
            Piece::Task { prompt: "find my passport".into(), agent: Some("grok".into()), project: None },
        );
        let pending = lantern.model.assistant.pending_task.clone().unwrap();
        assert_eq!(pending.kind.as_str(), "codex");
        assert_eq!(pending.folder, crate::settings::home().to_string_lossy());
    }

    #[test]
    fn new_question_cancels_the_answer_and_the_pending_task() {
        let mut lantern = lantern(&["claude"]);
        let (generation, turn) = with_turn(&mut lantern);
        lantern.assistant_piece(generation, turn, Piece::Task { prompt: "x".into(), agent: None, project: None });
        lantern.clear_assistant();
        assert!(lantern.model.assistant.turns.is_empty() && lantern.model.assistant.pending_task.is_none());
        lantern.assistant_piece(generation, turn, Piece::Text("late".into()));
        assert!(lantern.model.assistant.turns.is_empty());
    }

    #[test]
    fn summary_lists_waiting_agents_first_with_their_question() {
        let agent = |handle: &str, state, question: Option<&str>| Agent {
            handle: handle.into(),
            kind: AgentKind::new("claude"),
            project: Some("p".into()),
            state,
            question: question.map(str::to_string),
            ..Default::default()
        };
        let summary = agent_summary(
            &[agent("busy", AgentState::Working, Some("hidden")), agent("asks", AgentState::NeedsInput, Some("Allow rm?"))],
            Platform::Linux,
        );
        let lines: Vec<&str> = summary.lines().collect();
        assert!(lines[0].ends_with("on this Linux computer right now:"));
        assert_eq!(lines[1], "- @asks (Claude Code) in p: Needs you; asking: Allow rm?");
        assert_eq!(lines[2], "- @busy (Claude Code) in p: Working");
        assert!(agent_summary(&[], Platform::Windows).ends_with("No coding agents are running on this Windows PC."));
    }
}
