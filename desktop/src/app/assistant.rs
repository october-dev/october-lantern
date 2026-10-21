//! Ask Lantern (Swift `Assistant.swift` and `AppModel.defaultAgentKind`): a chat with October's AI
//! that knows what your agents are doing, and starts a task when the answer asks for one, after a
//! three-second window to cancel. The task's agent runs in the background and stays in the
//! conversation: its replies show in the thread and what you type next goes to it. Conversations
//! are saved on this computer (the most recent 50) and listed on Ask Lantern's home.
//!
//! One deliberate difference from the Mac (CONTRACT §9.2): "Started <agent> in <folder>" is written
//! only once the launch succeeded; a failed one shows its error instead.
use super::{Lantern, Pending};
use crate::model::{
    AgentKind, AgentState, AppContext, AssistantTurn, Chat, ChatMessage, Conversation, HarnessState, PanelMode, PendingTask, Platform,
    TaskLink,
};
use crate::october_ai::{self, Failure, Piece};
use crate::os::Event;
use crate::ui::SessionRequest;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Seconds a proposed task waits before it starts.
/// Saved conversations kept, newest first.
const KEPT_CONVERSATIONS: usize = 50;
/// A task's messages kept with the conversation.
const KEPT_MESSAGES: usize = 60;

#[derive(Default)]
pub(super) struct Asking {
    /// The question being answered; a newer one (or New chat) makes older answers stale.
    generation: Arc<AtomicU64>,
    next_id: u64,
    /// The conversation being had: its id in the saved list.
    conversation: String,
    /// The turns as last seen, and when they last changed: saved a second after changes settle.
    seen: Vec<AssistantTurn>,
    changed_at: Option<Instant>,
    /// Where conversations are saved (none in tests).
    file: Option<PathBuf>,
    /// The history request for the watched agent; older answers are stale.
    pub watch_request: Option<String>,
    watch_at: Option<Instant>,
    /// A task waiting for the engine to say which agents are installed (just after it starts),
    /// and since when: given up after 20 seconds.
    awaiting_agents: Option<(Instant, AwaitingTask)>,
}

/// What `propose_with` was asked to start, kept until the installed agents are known.
#[derive(Debug)]
struct AwaitingTask {
    prompt: String,
    agent: Option<String>,
    project: Option<String>,
    turn: u64,
    screenshot: Option<std::path::PathBuf>,
    context: Option<AppContext>,
}

impl Asking {
    pub(super) fn saving_to(file: Option<PathBuf>) -> Self {
        Self { file, ..Default::default() }
    }
}

/// How Ask Lantern's launch went (Swift `LaunchOutcome`): started (in a Lantern session when it
/// has one), or why not.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum LaunchOutcome {
    Started { session: Option<String> },
    Failed(String),
}

/// Where conversations are saved: `ask-lantern.json` in Lantern's data folder.
pub(super) fn conversations_path() -> PathBuf {
    crate::settings::data_dir().join("ask-lantern.json")
}

pub(super) fn load_conversations(path: &Path) -> Vec<Conversation> {
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Puts `conversation` first (replacing its earlier copy), keeping the most recent 50.
fn remember(conversations: &mut Vec<Conversation>, conversation: Conversation) {
    conversations.retain(|c| c.id != conversation.id);
    conversations.insert(0, conversation);
    conversations.truncate(KEPT_CONVERSATIONS);
}

fn folder_name(folder: &str) -> String {
    Path::new(folder).file_name().map_or(folder.to_string(), |f| f.to_string_lossy().into_owned())
}

impl Lantern {
    /// Asks Lantern; with the latest task's agent still running, replies to it instead (unless
    /// "Ask Lantern instead" is on).
    pub(super) fn submit_assistant(&mut self) {
        if !self.model.assistant.ask_lantern_instead
            && let Some(agent_id) = self.model.assistant_agent().map(|agent| agent.id.clone())
        {
            return self.reply_to_task(&agent_id);
        }
        self.ask_assistant();
        self.model.assistant.ask_lantern_instead = false;
    }

    /// What you type goes to the agent (Point & Ask's direct send: typed in where Lantern can,
    /// otherwise copied with its terminal brought forward). Works while Lantern is answering.
    fn reply_to_task(&mut self, agent_id: &str) {
        let message = self.inputs.assistant.trim().to_string();
        if message.is_empty() {
            return;
        }
        self.inputs.assistant.clear();
        self.send_direct(agent_id, message);
    }

    fn ask_assistant(&mut self) {
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
        crate::analytics::capture("assistant_asked", json!({"follow_up": self.model.assistant.turns.len() > 1}));
        let generation = self.asking.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let current = self.asking.generation.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let pieces = tx.clone();
            let result = october_ai::stream(&request, &token, &mut |piece| {
                current.load(Ordering::SeqCst) == generation && pieces.send(Event::Assistant { generation, turn, piece }).is_ok()
            });
            let error = result.err().filter(|failure| *failure != Failure::Cancelled);
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

    pub(super) fn assistant_done(&mut self, generation: u64, turn: u64, error: Option<Failure>) {
        if generation != self.asking.generation.load(Ordering::SeqCst) {
            return;
        }
        if let Some(t) = self.model.assistant.turns.iter_mut().find(|t| t.id == turn) {
            match error {
                Some(error) => {
                    t.answer = error.to_string();
                    t.failed = true;
                    t.upgrade = error.upgrade().map(str::to_string);
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
        self.propose_with(prompt, agent, project, turn, None, None);
    }

    /// Point & Ask decided its question is work (Swift `Assistant.adopt`): a new conversation here
    /// with that question and answer, starting the task with the selection's screenshot.
    /// October's prompt is used as it is: it says what the screenshot shows, which the user's own
    /// words ("move these") don't.
    pub(super) fn adopt_point_task(
        &mut self,
        question: String,
        answer: String,
        work: (String, Option<String>, Option<String>),
        screenshot: Option<std::path::PathBuf>,
        context: Option<AppContext>,
    ) {
        self.clear_assistant();
        self.asking.next_id += 1;
        let turn = self.asking.next_id;
        self.model.assistant.turns.push(AssistantTurn { id: turn, question, answer, ..Default::default() });
        let (prompt, agent, project) = work;
        self.propose_with(prompt, agent, project, turn, screenshot, context);
    }

    fn propose_with(
        &mut self,
        prompt: String,
        agent: Option<String>,
        project: Option<String>,
        turn: u64,
        screenshot: Option<std::path::PathBuf>,
        context: Option<AppContext>,
    ) {
        let Some(installed) = self.model.new_session.installed.clone() else {
            // The engine hasn't said which agents are installed yet: start once it has.
            self.refresh_installed();
            let task = AwaitingTask { prompt, agent, project, turn, screenshot, context };
            self.asking.awaiting_agents = Some((Instant::now(), task));
            return;
        };
        let named = agent.map(AgentKind).filter(|kind| installed.contains(kind));
        let Some(kind) = named.or_else(|| self.default_agent_kind()) else {
            self.refresh_installed();
            return self.set_task_problem(turn, "Lantern has no agent to do this with. Set one up in New Session, then try again.");
        };
        let folder = self.task_folder(project.as_deref());
        let turns = &self.model.assistant.turns;
        let prompt = match turns.iter().position(|t| t.id == turn) {
            Some(i) if screenshot.is_none() => task_prompt(&turns[i].question, prompt, i > 0),
            _ => prompt,
        };
        self.asking.next_id += 1;
        self.model.assistant.pending_task = Some(PendingTask { id: self.asking.next_id, turn, prompt, kind, folder, screenshot, context });
    }

    /// Starts a task the answer asked for (on the next tick, so it never waits), reads the task's
    /// agent, and saves the conversation once changes settle.
    pub(super) fn assistant_tick(&mut self) {
        if let Some((since, _)) = &self.asking.awaiting_agents {
            if self.model.new_session.installed.is_some() {
                let (_, t) = self.asking.awaiting_agents.take().expect("checked");
                self.propose_with(t.prompt, t.agent, t.project, t.turn, t.screenshot, t.context);
            } else if since.elapsed() >= Duration::from_secs(20) {
                let (_, t) = self.asking.awaiting_agents.take().expect("checked");
                self.set_task_problem(
                    t.turn,
                    "Lantern couldn't check which agents this computer has, so nothing was started. Try again in a moment.",
                );
            }
        }
        if self.model.assistant.pending_task.is_some() {
            self.start_pending_task();
        }
        self.watch_tick();
        if self.model.assistant.turns != self.asking.seen {
            self.asking.seen = self.model.assistant.turns.clone();
            self.asking.changed_at = Some(Instant::now());
        }
        if self.asking.changed_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(1)) {
            self.save_conversation();
        }
    }

    pub(super) fn cancel_assistant_task(&mut self) {
        self.model.assistant.pending_task = None;
    }

    fn set_task_problem(&mut self, turn: u64, problem: &str) {
        if let Some(t) = self.model.assistant.turns.iter_mut().find(|t| t.id == turn) {
            t.task_problem = Some(problem.into());
        }
    }

    /// Looks for installed agents again, so one installed since Lantern started shows up.
    pub(super) fn refresh_installed(&mut self) {
        self.request(serde_json::json!({"type": "installed.refresh"}));
    }

    /// Starts a task that couldn't start again, with the same agent, folder and message (Swift
    /// `Assistant.retry`).
    pub(super) fn retry_assistant_task(&mut self, turn: u64) {
        let Some(t) = self.model.assistant.turns.iter_mut().find(|t| t.id == turn) else { return };
        let Some(link) = t.task.take() else { return };
        self.asking.next_id += 1;
        self.model.assistant.pending_task = Some(PendingTask {
            id: self.asking.next_id,
            turn,
            prompt: link.prompt,
            kind: link.kind,
            folder: link.folder,
            screenshot: None,
            context: None,
        });
        self.start_pending_task();
    }

    /// Starts the task in the background (a Lantern session: tmux on Linux, ConPTY on Windows)
    /// when possible, shown right here in the conversation; Open shows its terminal. Otherwise a
    /// Terminal window. October waits for October Harness to finish installing.
    pub(super) fn start_pending_task(&mut self) {
        let Some(task) = self.model.assistant.pending_task.take() else { return };
        crate::analytics::capture("assistant_task", json!({"kind": task.kind.as_str()}));
        // An agent never signed in gets a window, where it asks you to sign in.
        let sign_in = self.model.new_session.never_set_up.contains(&task.kind);
        let in_lantern = self.model.new_session.in_lantern_available;
        let background = in_lantern && !sign_in;
        let Some(turn) = self.model.assistant.turns.iter_mut().find(|t| t.id == task.turn) else { return };
        turn.task_problem = None;
        turn.task = Some(TaskLink {
            kind: task.kind.clone(),
            folder: task.folder.clone(),
            prompt: task.prompt.clone(),
            in_terminal: !in_lantern,
            sign_in,
            ..Default::default()
        });
        let request = SessionRequest {
            kind: task.kind,
            folder: task.folder,
            prompt: task.prompt,
            screenshot: task.screenshot,
            context: task.context,
            toolkit: true,
            bus: self.model.prefs.bus_for_sessions,
            background,
            for_turn: Some(task.turn),
            ..Default::default()
        };
        if !self.launch(request)
            && let Some(link) = self.task_link(task.turn)
        {
            link.error = Some("Another session is still starting. Try again in a moment.".into());
        }
    }

    fn task_link(&mut self, turn: u64) -> Option<&mut TaskLink> {
        self.model.assistant.turns.iter_mut().find(|t| t.id == turn)?.task.as_mut()
    }

    /// How the task's launch went (from `launch_result`).
    pub(super) fn assistant_launched(&mut self, turn: u64, outcome: LaunchOutcome) {
        let Some(t) = self.model.assistant.turns.iter_mut().find(|t| t.id == turn) else { return };
        let Some(link) = t.task.as_mut() else { return };
        match outcome {
            LaunchOutcome::Started { session } => {
                if session.is_some() {
                    link.session = session;
                }
                let started = format!("Started {} in {}", link.kind.display_name(), folder_name(&link.folder));
                t.started = Some(started);
            }
            LaunchOutcome::Failed(message) => link.error = Some(message),
        }
        self.track_tasks();
    }

    /// Remembers which agent each task became (found by its session, then kept by id), and what
    /// the latest one has said (Swift `Assistant.track`).
    pub(super) fn track_tasks(&mut self) {
        let model = &self.model;
        let latest = model.assistant.latest_task().map(|(turn, _)| turn);
        let updates: Vec<(usize, TaskLink)> = model
            .assistant
            .turns
            .iter()
            .enumerate()
            .filter_map(|(i, turn)| {
                let link = turn.task.as_ref()?;
                let agent = model.task_agent(link).map(|agent| agent.id.as_str());
                let mut updated = link.clone();
                if updated.agent_id.is_none() {
                    updated.agent_id = agent.map(str::to_string);
                }
                if Some(turn.id) == latest
                    && let Some(watched) = &model.assistant.watched
                    && agent == Some(watched.agent_id.as_str())
                    && !watched.messages.is_empty()
                {
                    updated.transcript = watched.messages[watched.messages.len().saturating_sub(KEPT_MESSAGES)..].to_vec();
                }
                (updated != *link).then_some((i, updated))
            })
            .collect();
        for (i, link) in updates {
            self.model.assistant.turns[i].task = Some(link);
        }
    }

    /// Only the latest task's agent is read live, every two seconds like an open chat (and
    /// without disturbing it), while Ask Lantern's conversation shows (Swift `AppModel.watch`).
    fn watch_tick(&mut self) {
        let showing = self.model.panel == Some(PanelMode::Assistant) && !self.model.assistant.turns.is_empty();
        let wanted = if showing { self.model.assistant_agent().map(|agent| agent.id.clone()) } else { None };
        if wanted != self.model.assistant.watched.as_ref().map(|watched| watched.agent_id.clone()) {
            self.stop_watching();
            self.model.assistant.watched =
                wanted.clone().map(|agent_id| Chat { agent_id, supported: true, loading: true, ..Default::default() });
        }
        let Some(agent_id) = wanted else { return };
        if self.asking.watch_at.is_some_and(|at| at.elapsed() < Duration::from_secs(2)) {
            return;
        }
        self.asking.watch_at = Some(Instant::now());
        if let Some(old) = self.asking.watch_request.take() {
            self.pending.remove(&old);
        }
        if let Some(id) = self.request(json!({"type": "history", "agentId": agent_id})) {
            self.pending.insert(id.clone(), Pending::History);
            self.asking.watch_request = Some(id);
        }
    }

    fn stop_watching(&mut self) {
        if let Some(old) = self.asking.watch_request.take() {
            self.pending.remove(&old);
        }
        self.asking.watch_at = None;
        self.model.assistant.watched = None;
    }

    /// The watched agent's conversation arrived (from `history_result`).
    pub(super) fn watch_result(&mut self, event: &Value) {
        let Some(watched) = self.model.assistant.watched.as_mut() else { return };
        if event["agentId"].as_str() != Some(watched.agent_id.as_str()) || event["busy"] == true {
            return;
        }
        let messages: Vec<ChatMessage> = serde_json::from_value(event["messages"].clone()).unwrap_or_default();
        watched.loading = false;
        watched.supported = event["supported"] != false;
        if watched.messages != messages {
            watched.messages = messages;
        }
        self.track_tasks();
    }

    /// New chat: stops the answer and any pending task; the conversation stays in Recent.
    pub(super) fn clear_assistant(&mut self) {
        self.asking.generation.fetch_add(1, Ordering::SeqCst);
        self.cancel_assistant_task();
        self.save_conversation();
        self.reset_conversation();
    }

    fn reset_conversation(&mut self) {
        self.model.assistant.turns.clear();
        self.model.assistant.asking = false;
        self.model.assistant.ask_lantern_instead = false;
        self.stop_watching();
        self.asking.conversation = uuid::Uuid::new_v4().to_string();
        self.asking.seen.clear();
        self.asking.changed_at = None;
    }

    /// Reopens a saved conversation; an agent from it that's still running is live again.
    pub(super) fn open_conversation(&mut self, id: &str) {
        let Some(conversation) = self.model.assistant.conversations.iter().find(|c| c.id == id).cloned() else { return };
        self.clear_assistant();
        // Turn ids stay unique across the reopened turns and new ones.
        self.asking.next_id = self.asking.next_id.max(conversation.turns.iter().map(|t| t.id).max().unwrap_or(0));
        self.asking.seen = conversation.turns.clone();
        self.model.assistant.turns = conversation.turns;
        self.asking.conversation = conversation.id;
        self.track_tasks();
    }

    pub(super) fn delete_conversation(&mut self, id: &str) {
        self.model.assistant.conversations.retain(|c| c.id != id);
        if self.asking.conversation == id {
            self.asking.generation.fetch_add(1, Ordering::SeqCst);
            self.cancel_assistant_task();
            self.reset_conversation();
        }
        self.store_conversations();
    }

    /// Puts the current conversation first in the saved list (once it has a question).
    fn save_conversation(&mut self) {
        self.asking.changed_at = None;
        let kept: Vec<AssistantTurn> = self.model.assistant.turns.iter().filter(|t| !t.question.is_empty()).cloned().collect();
        let Some(first) = kept.first() else { return };
        if self.asking.conversation.is_empty() {
            self.asking.conversation = uuid::Uuid::new_v4().to_string();
        }
        let conversation = Conversation {
            id: self.asking.conversation.clone(),
            title: crate::model::conversation_title(&first.question),
            updated: crate::model::iso8601(Self::now_ms()),
            turns: kept,
        };
        remember(&mut self.model.assistant.conversations, conversation);
        self.store_conversations();
    }

    fn store_conversations(&self) {
        let Some(file) = &self.asking.file else { return };
        let written = serde_json::to_vec(&self.model.assistant.conversations)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| crate::settings::private_write(file, &bytes));
        if let Err(error) = written {
            eprintln!("Lantern: couldn't save Ask Lantern's conversations: {error:#}");
        }
    }

    /// The agent new tasks start with (Swift `defaultAgentKind`): the preference if it's set and
    /// can start; else an agent signed in to a plan the user already pays for, so their own
    /// subscription is used before October's; else October (counted once October Harness is ready,
    /// or will be installed because there's no Claude Code or Codex set up; after a failed
    /// install, only when nothing else can start), Claude Code, Codex, then the first installed agent. A Claude Code or
    /// Codex never signed in comes last. `None` until the engine has said what's installed.
    pub(super) fn default_agent_kind(&self) -> Option<AgentKind> {
        let installed = self.model.new_session.installed.clone()?;
        let never = &self.model.new_session.never_set_up;
        let harness = self.model.harness.as_ref().map(|h| h.status);
        // After a failed install, October is tried again only when there's nothing else to start.
        let other = installed.iter().any(|kind| !never.contains(kind));
        let usable = |kind: &AgentKind| {
            installed.contains(kind)
                || kind.as_str() == AgentKind::OCTOBER
                    && (harness == Some(HarnessState::Ready) || self.needs_harness() && (harness != Some(HarnessState::Failed) || !other))
        };
        let chosen = Some(AgentKind(self.model.prefs.task_agent.clone())).filter(|k| !k.as_str().is_empty());
        chosen
            .filter(usable)
            .or_else(|| {
                self.model
                    .new_session
                    .signed_in
                    .iter()
                    .cloned()
                    .chain([AgentKind::OCTOBER, AgentKind::CLAUDE, AgentKind::CODEX].map(AgentKind::new))
                    .find(|kind| usable(kind) && !never.contains(kind))
            })
            .or_else(|| installed.iter().find(|kind| !never.contains(kind)).cloned())
            .or_else(|| installed.first().cloned())
    }

    /// No Claude Code or Codex that's been set up: October Harness is what this computer can run.
    pub(super) fn needs_harness(&self) -> bool {
        let never = &self.model.new_session.never_set_up;
        self.model
            .new_session
            .installed
            .as_ref()
            .is_some_and(|kinds| !kinds.iter().any(|k| [AgentKind::CLAUDE, AgentKind::CODEX].contains(&k.as_str()) && !never.contains(k)))
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
    fn default_agent_prefers_the_setting_then_your_own_plan_then_october() {
        let mut both = lantern(&["codex", "claude", "gemini"]);
        assert_eq!(both.default_agent_kind(), Some(AgentKind::new("claude")), "October isn't installed and isn't needed");
        both.model.new_session.signed_in = vec![AgentKind::new("codex")];
        assert_eq!(both.default_agent_kind(), Some(AgentKind::new("codex")), "a signed-in plan comes first");
        both.model.prefs.task_agent = "gemini".into();
        assert_eq!(both.default_agent_kind(), Some(AgentKind::new("gemini")), "unless another agent was chosen");
        let mut with_october = lantern(&["claude", "october"]);
        assert_eq!(with_october.default_agent_kind(), Some(AgentKind::new("october")), "Claude Code without a plan");
        with_october.model.new_session.signed_in = vec![AgentKind::new("claude")];
        assert_eq!(with_october.default_agent_kind(), Some(AgentKind::new("claude")));
        let mut only_gemini = lantern(&["gemini"]);
        assert_eq!(only_gemini.default_agent_kind(), Some(AgentKind::new("october")), "no Claude or Codex: the harness will install");
        only_gemini.model.harness = Some(HarnessStatus { status: HarnessState::Failed, ..Default::default() });
        assert_eq!(only_gemini.default_agent_kind(), Some(AgentKind::new("gemini")), "a failed harness install falls back");

        let mut nothing = lantern(&[]);
        nothing.model.harness = Some(HarnessStatus { status: HarnessState::Failed, ..Default::default() });
        assert_eq!(nothing.default_agent_kind(), Some(AgentKind::new("october")), "with nothing else, October is tried again");
        assert_eq!(lantern(&[]).default_agent_kind(), Some(AgentKind::new("october")));
        let mut unknown = lantern(&[]);
        unknown.model.new_session.installed = None;
        assert_eq!(unknown.default_agent_kind(), None, "not known yet");
    }

    #[test]
    fn agents_never_signed_in_come_last_and_start_where_you_can_sign_in() {
        let mut fresh = lantern(&["claude", "gemini"]);
        fresh.model.new_session.never_set_up = vec![AgentKind::new("claude")];
        assert!(fresh.needs_harness(), "a Claude Code never signed in doesn't count");
        assert_eq!(fresh.default_agent_kind(), Some(AgentKind::new("october")));
        fresh.model.prefs.task_agent = "claude".into();
        assert_eq!(fresh.default_agent_kind(), Some(AgentKind::new("claude")), "chosen anyway");

        let (_, turn) = with_turn(&mut fresh);
        fresh.propose(String::from("fix it"), Some("claude".into()), None, turn);
        fresh.start_pending_task();
        let link = fresh.model.assistant.turns.iter().find(|t| t.id == turn).and_then(|t| t.task.clone()).expect("started");
        assert!(link.sign_in && !link.in_terminal, "a window to sign in, still in a Lantern session");
    }

    #[test]
    fn a_task_waits_for_the_installed_agents_then_starts() {
        let mut lantern = lantern(&["claude"]);
        lantern.model.new_session.installed = None;
        let (_, turn) = with_turn(&mut lantern);
        lantern.propose(String::from("fix it"), None, None, turn);
        assert!(lantern.model.assistant.pending_task.is_none());
        lantern.model.new_session.installed = Some(vec![AgentKind::new("claude")]);
        lantern.assistant_tick();
        let t = lantern.model.assistant.turns.iter().find(|t| t.id == turn).expect("turn");
        assert_eq!(t.task.as_ref().map(|link| link.kind.as_str()), Some("claude"));
        assert_eq!(t.task_problem, None);
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
        lantern.assistant_done(generation, turn, Some(Failure::SignedOut));
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
    fn a_task_starts_on_the_next_tick_and_says_so_only_when_it_launched() {
        let mut lantern = lantern(&["claude"]);
        lantern.model.new_session.folders = vec!["/work/october-lantern".into()];
        let (generation, turn) = with_turn(&mut lantern);
        let task = Piece::Task { prompt: "fix the tests".into(), agent: None, project: Some("October-Lantern".into()) };
        lantern.assistant_piece(generation, turn, task);
        let pending = lantern.model.assistant.pending_task.clone().unwrap();
        assert_eq!((pending.kind.as_str(), pending.folder.as_str()), ("claude", "/work/october-lantern"));
        lantern.assistant_tick();
        assert!(lantern.model.assistant.pending_task.is_none(), "no countdown: the next tick starts it");
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
        assert_eq!(t.task.as_ref().and_then(|link| link.error.as_deref()), Some("tmux is missing"));
    }

    /// The engine answers the pending launch.
    fn answer_launch(lantern: &mut Lantern, ok: bool, session: Option<&str>) {
        let id = lantern.pending.iter().find(|(_, p)| matches!(p, Pending::Launch)).map(|(id, _)| id.clone()).expect("a launch");
        lantern.engine_event(json!({"type": "launchResult", "requestId": id, "ok": ok, "session": session}));
    }

    fn running(id: &str, session: &str) -> Agent {
        Agent {
            id: id.into(),
            kind: AgentKind::new("claude"),
            handle: "claude-1".into(),
            state: AgentState::Working,
            can_reply: true,
            tmux: Some(crate::model::TmuxPane { target: format!("{session}:0.0"), ..Default::default() }),
            ..Default::default()
        }
    }

    /// Ask Lantern with a task whose agent is up in session `claude-x-1`.
    fn with_running_task(lantern: &mut Lantern) -> u64 {
        lantern.set_panel(Some(PanelMode::Assistant));
        let (generation, turn) = with_turn(lantern);
        lantern.assistant_piece(generation, turn, Piece::Task { prompt: "x".into(), agent: None, project: None });
        lantern.assistant_done(generation, turn, None);
        lantern.start_pending_task();
        answer_launch(lantern, true, Some("claude-x-1"));
        lantern.engine_event(json!({"type": "snapshot", "agents": [running("claude:9:9", "claude-x-1")]}));
        turn
    }

    #[test]
    fn a_task_launches_quietly_and_reports_back_without_switching_panels() {
        let mut lantern = lantern(&["claude"]);
        lantern.set_panel(Some(PanelMode::Assistant));
        let (generation, turn) = with_turn(&mut lantern);
        lantern.assistant_piece(generation, turn, Piece::Task { prompt: "find my thesis".into(), agent: None, project: None });
        lantern.start_pending_task();
        let link = lantern.model.assistant.turns[0].task.clone().expect("the task is in the conversation");
        assert_eq!((link.prompt.as_str(), link.in_terminal, link.session.as_deref()), ("q", false, None));
        assert_eq!(lantern.model.panel, Some(PanelMode::Assistant));
        assert!(lantern.model.launch_preview.is_none() && lantern.model.chat.is_none(), "no Starting… preview, no chat");
        answer_launch(&mut lantern, true, Some("claude-x-1"));
        assert_eq!(lantern.model.panel, Some(PanelMode::Assistant));
        assert!(lantern.model.chat.is_none() && lantern.model.toast.is_none());
        let link = lantern.model.assistant.turns[0].task.clone().unwrap();
        assert_eq!(link.session.as_deref(), Some("claude-x-1"));
        // Found by its session, then remembered by id; its chat still doesn't open.
        lantern.engine_event(json!({"type": "snapshot", "agents": [running("claude:9:9", "claude-x-1")]}));
        assert_eq!(lantern.model.assistant.turns[0].task.as_ref().unwrap().agent_id.as_deref(), Some("claude:9:9"));
        assert!(lantern.model.chat.is_none());
        assert_eq!(lantern.model.assistant_agent().map(|agent| agent.id.as_str()), Some("claude:9:9"));
        // Its conversation is read live, without opening a chat, and kept with the task.
        lantern.assistant_tick();
        assert_eq!(lantern.model.assistant.watched.as_ref().map(|w| w.agent_id.as_str()), Some("claude:9:9"));
        let request = lantern.asking.watch_request.clone().expect("history asked for");
        lantern.engine_event(json!({"type": "historyResult", "requestId": request, "agentId": "claude:9:9", "supported": true,
            "messages": [{"role": "user", "text": "q"}, {"role": "agent", "text": "Found it."}]}));
        let link = lantern.model.assistant.turns[0].task.clone().unwrap();
        assert_eq!(link.transcript.len(), 2);
        assert!(lantern.model.chat.is_none());
    }

    #[test]
    fn a_task_without_a_lantern_session_runs_in_a_terminal_window() {
        let mut lantern = lantern(&["claude"]);
        lantern.model.new_session.in_lantern_available = false;
        let (generation, turn) = with_turn(&mut lantern);
        lantern.assistant_piece(generation, turn, Piece::Task { prompt: "x".into(), agent: None, project: None });
        lantern.start_pending_task();
        answer_launch(&mut lantern, true, None);
        let t = &lantern.model.assistant.turns[0];
        assert!(t.task.as_ref().unwrap().in_terminal);
        assert_eq!(t.started.as_deref().map(|s| s.starts_with("Started Claude Code in")), Some(true));
        assert_ne!(lantern.model.panel, Some(PanelMode::Agents));
    }

    #[test]
    fn the_box_replies_to_the_running_agent_unless_asked_to_ask_lantern() {
        let mut lantern = lantern(&["claude"]);
        with_running_task(&mut lantern);
        let directs = |lantern: &Lantern| {
            lantern.pending.values().filter(|p| matches!(p, Pending::Direct { agent_id } if agent_id == "claude:9:9")).count()
        };
        // While Lantern is answering, too.
        lantern.model.assistant.asking = true;
        lantern.inputs.assistant = " yes, the PDF ".into();
        lantern.submit_assistant();
        assert_eq!(directs(&lantern), 1, "sent to the agent");
        assert_eq!(lantern.inputs.assistant, "");
        assert_eq!(lantern.model.assistant.turns.len(), 1, "not a question for Lantern");
        lantern.model.assistant.asking = false;
        // Ask Lantern instead: not to the agent (signed out here, so nothing is asked), and the
        // box goes back to replying afterwards.
        lantern.apply(crate::ui::UiAction::ToggleAskLanternInstead);
        assert!(lantern.model.assistant.ask_lantern_instead);
        lantern.inputs.assistant = "what else is running?".into();
        lantern.submit_assistant();
        assert_eq!(directs(&lantern), 1);
        assert!(!lantern.model.assistant.ask_lantern_instead);
        // An agent that has stopped takes no replies.
        lantern.engine_event(json!({"type": "snapshot", "agents": []}));
        assert!(lantern.model.assistant_agent().is_none());
        lantern.submit_assistant();
        assert_eq!(directs(&lantern), 1);
    }

    #[test]
    fn conversations_are_saved_newest_first_and_reopen_with_their_agent() {
        let dir = std::env::temp_dir().join(format!("lantern-ask-{}", uuid::Uuid::new_v4().simple()));
        let file = dir.join("ask-lantern.json");
        let mut lantern = lantern(&["claude"]);
        lantern.asking.file = Some(file.clone());
        let turn = with_running_task(&mut lantern);
        lantern.model.assistant.turns[0].question = "Find me my thesis document\nIt's a PDF".into();
        lantern.clear_assistant();
        assert!(lantern.model.assistant.turns.is_empty() && lantern.model.assistant.watched.is_none());
        let saved = load_conversations(&file);
        assert_eq!(saved, lantern.model.assistant.conversations);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].title, "Find me my thesis document");
        assert!(crate::model::parse_iso8601(&saved[0].updated).is_some());
        assert_eq!(saved[0].turns[0].task.as_ref().unwrap().agent_id.as_deref(), Some("claude:9:9"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // A second chat goes first; reopening the first brings its running agent back.
        lantern.inputs.assistant.clear();
        let (_, _) = with_turn(&mut lantern);
        lantern.clear_assistant();
        assert_eq!(lantern.model.assistant.conversations.len(), 2);
        assert_eq!(lantern.model.assistant.conversations[0].title, "q");
        let first = lantern.model.assistant.conversations[1].id.clone();
        lantern.open_conversation(&first);
        assert_eq!(lantern.model.assistant.turns[0].id, turn);
        assert_eq!(lantern.model.assistant_agent().map(|agent| agent.id.as_str()), Some("claude:9:9"));
        assert!(lantern.asking.next_id >= turn);
        lantern.clear_assistant();
        lantern.delete_conversation(&first);
        assert_eq!(load_conversations(&file).len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn at_most_50_conversations_are_kept_newest_first() {
        let mut conversations = Vec::new();
        for n in 0..55 {
            remember(&mut conversations, Conversation { id: n.to_string(), ..Default::default() });
        }
        remember(&mut conversations, Conversation { id: "40".into(), title: "again".into(), ..Default::default() });
        assert_eq!(conversations.len(), 50);
        assert_eq!((conversations[0].id.as_str(), conversations[0].title.as_str()), ("40", "again"));
        assert_eq!(conversations[1].id, "54");
        assert_eq!(conversations.iter().filter(|c| c.id == "40").count(), 1);
        assert!(!conversations.iter().any(|c| c.id == "4"), "the oldest went");
    }

    #[test]
    fn changes_are_saved_a_moment_after_they_settle() {
        let mut lantern = lantern(&["claude"]);
        with_turn(&mut lantern);
        lantern.assistant_tick();
        assert!(lantern.model.assistant.conversations.is_empty(), "not right away");
        lantern.asking.changed_at = Some(Instant::now() - Duration::from_secs(2));
        lantern.assistant_tick();
        assert_eq!(lantern.model.assistant.conversations.len(), 1);
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
