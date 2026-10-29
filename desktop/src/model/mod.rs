//! Lantern's domain state, mirroring the Mac app's `AppModel` (macos/Sources/OctoberLantern).
//! `ui/` reads it through `&Model` and never mutates it; every change goes through
//! `app::Lantern::apply`. Engine payloads are typed here (see protocol/README.md).

pub mod cards;
pub mod drafts;
pub mod inbox;
pub mod inputs;
pub mod phone;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

pub use cards::{Card, CardAnswer, CardLaunch};
pub use drafts::Drafts;
pub use inbox::Inbox;
pub use inputs::Inputs;

pub type AgentId = String;

/// The harness an agent runs on. Open-ended, so a newer engine can report one this app doesn't know.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentKind(pub String);

impl AgentKind {
    pub const OCTOBER: &str = "october";
    pub const CLAUDE: &str = "claude";
    pub const CODEX: &str = "codex";
    /// Every agent Lantern supports, in the order New Session lists them.
    pub const SUPPORTED: [&str; 18] = [
        "claude", "codex", "gemini", "opencode", "pi", "october", "grok", "cursor", "copilot", "qwen", "goose", "aider", "amp", "kimi",
        "droid", "crush", "auggie", "muse",
    ];
    pub fn new(kind: &str) -> Self {
        Self(kind.to_string())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn display_name(&self) -> String {
        match self.0.as_str() {
            "claude" => "Claude Code",
            "codex" => "Codex",
            "opencode" => "OpenCode",
            "pi" => "Pi",
            "gemini" => "Gemini CLI",
            "grok" => "Grok",
            "cursor" => "Cursor",
            "qwen" => "Qwen Code",
            "goose" => "Goose",
            "aider" => "Aider",
            "amp" => "Amp",
            "copilot" => "Copilot",
            "kimi" => "Kimi",
            "droid" => "Droid",
            "crush" => "Crush",
            "auggie" => "Auggie",
            "muse" => "Muse",
            "october" => "October",
            other => return capitalized(other),
        }
        .to_string()
    }
    /// Where to get an agent that isn't installed (none for Muse, which is Meta-internal).
    pub fn website(&self) -> Option<&'static str> {
        Some(match self.0.as_str() {
            "claude" => "https://code.claude.com/docs",
            "codex" => "https://github.com/openai/codex",
            "gemini" => "https://github.com/google-gemini/gemini-cli",
            "opencode" => "https://opencode.ai",
            "pi" => "https://github.com/earendil-works/pi",
            "october" => "https://github.com/october-dev/october-harness",
            "grok" => "https://x.ai",
            "cursor" => "https://cursor.com/cli",
            "copilot" => "https://github.com/github/copilot-cli",
            "qwen" => "https://github.com/QwenLM/qwen-code",
            "goose" => "https://github.com/block/goose",
            "aider" => "https://aider.chat",
            "amp" => "https://ampcode.com",
            "kimi" => "https://github.com/MoonshotAI/kimi-cli",
            "droid" => "https://factory.ai",
            "crush" => "https://github.com/charmbracelet/crush",
            "auggie" => "https://www.augmentcode.com",
            _ => return None,
        })
    }
}

/// Swift's `String.capitalized` for one word: first letter upper, the rest lower.
fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect()).unwrap_or_default()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Working,
    Waiting,
    Idle,
    NeedsInput,
    /// Also any state a newer engine reports that this app doesn't know.
    #[default]
    #[serde(other)]
    Unknown,
}

impl AgentState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::Waiting => "Your turn",
            Self::NeedsInput => "Needs you",
            Self::Idle => "Idle",
            Self::Unknown => "Running",
        }
    }
    /// Waiting on the person, so it belongs in the inbox.
    pub fn wants_you(self) -> bool {
        matches!(self, Self::Waiting | Self::NeedsInput)
    }
    /// Order in the agent list: needs you, your turn, working, the rest.
    pub fn rank(self) -> u8 {
        match self {
            Self::NeedsInput => 0,
            Self::Waiting => 1,
            Self::Working => 2,
            Self::Idle | Self::Unknown => 3,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HostApp {
    pub app: String,
    pub pid: i64,
    pub bundle_path: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TmuxPane {
    pub socket: Option<String>,
    pub target: String,
    pub pane_id: String,
}

/// How Lantern types into an agent: tmux, cmux, terminal, iterm, october, pty (Lantern's own
/// session on Windows) or none.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Route {
    pub via: String,
    /// The Lantern session name, for `pty`.
    pub session: Option<String>,
}

impl Route {
    /// Single keys (e.g. "1" to allow a permission prompt) can be sent without Enter.
    pub fn sends_keys(&self) -> bool {
        matches!(self.via.as_str(), "tmux" | "cmux" | "iterm" | "pty" | "console")
    }
}

/// What an agent's question offers (engine `Choices`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Choices {
    /// Claude's own questions, in the order the terminal asks them.
    pub questions: Vec<ChoiceQuestion>,
    /// A permission prompt's "don't ask again" option, as the terminal words it.
    pub always: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChoiceQuestion {
    pub question: String,
    pub header: Option<String>,
    pub multi_select: bool,
    pub options: Vec<ChoiceOption>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChoiceOption {
    pub label: String,
    pub description: Option<String>,
}

/// What the person chose for an agent's question; serialized as the engine's `Answer`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AgentAnswer {
    /// Allow once; start on the plan.
    Yes,
    /// The permission prompt's "don't ask again" option.
    Always,
    /// Deny; keep planning; don't answer Claude's question.
    No,
    /// The options picked for each of Claude's questions, in order (0-based).
    Picks { picks: Vec<Vec<usize>> },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Agent {
    /// `<kind>:<pid>:<start time>`; a reused pid is a different agent.
    pub id: AgentId,
    pub kind: AgentKind,
    pub handle: String,
    pub pid: i64,
    pub tty: Option<String>,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub title: Option<String>,
    pub session_id: Option<String>,
    pub state: AgentState,
    /// Milliseconds since the epoch.
    pub state_since: Option<f64>,
    pub last_message: Option<String>,
    pub question: Option<String>,
    /// "permission" at a tool permission prompt (`1` allows once, Escape declines); "choice" for
    /// Claude's own multiple-choice question; "plan" for its plan waiting for approval; "other"
    /// otherwise.
    pub question_kind: Option<String>,
    pub question_detail: Option<String>,
    /// Identifies one prompt; answers carry it so they can't answer a different one.
    pub prompt_id: Option<String>,
    /// The options Lantern offers for the question (Claude's own questions; a permission's "always").
    pub choices: Option<Choices>,
    /// Its October Bus id, for an agent Lantern started on the Bus (Ask Lantern's messages go there).
    pub bus_id: Option<String>,
    /// "exact", "guessed", "ambiguous" (state and history hidden) or "none".
    pub session_match: Option<String>,
    /// The app a session belongs to when it isn't a terminal ("Claude Desktop", "Codex app").
    pub source: Option<String>,
    pub live: Option<bool>,
    pub host: Option<HostApp>,
    pub tmux: Option<TmuxPane>,
    pub can_reply: bool,
    pub route: Option<Route>,
    pub state_source: String,
}

impl Agent {
    /// A question Lantern can answer from its card, by pressing the terminal's keys.
    pub fn is_answerable(&self) -> bool {
        let kind_ok = match self.question_kind.as_deref() {
            Some("permission" | "plan") => true,
            Some("choice") => self.choices.as_ref().is_some_and(|c| !c.questions.is_empty()),
            _ => false,
        };
        kind_ok && self.state == AgentState::NeedsInput && self.prompt_id.is_some() && self.route.as_ref().is_some_and(Route::sends_keys)
    }
    pub fn is_live(&self) -> bool {
        self.live.unwrap_or(true)
    }
    pub fn session_ambiguous(&self) -> bool {
        self.session_match.as_deref() == Some("ambiguous")
    }
    /// Identifies one turn, so dismissing it doesn't hide the next one: `id@stateSince`.
    pub fn turn_key(&self) -> String {
        format!("{}@{}", self.id, self.state_since.unwrap_or(0.0) as i64)
    }
    /// `cwd` with the home folder shown as `~`.
    pub fn short_cwd(&self, home: &str) -> Option<String> {
        let cwd = self.cwd.as_deref()?;
        Some(match cwd.strip_prefix(home) {
            Some(rest) if !home.is_empty() => format!("~{rest}"),
            _ => cwd.to_string(),
        })
    }
    /// Where the agent lives, for display: "cmux", "tmux t1:0.0", "Windows Terminal", …
    pub fn location(&self) -> Option<String> {
        if let Some(source) = &self.source {
            return Some(source.clone());
        }
        if let Some(tmux) = &self.tmux {
            return Some(format!("tmux {}", tmux.target));
        }
        self.host.as_ref().map(|host| host.app.clone())
    }
    /// The Lantern session this agent runs in, if Lantern started it: a tmux session on Linux
    /// (`session:window.pane`) or a ConPTY session on Windows.
    pub fn in_session(&self, session: &str) -> bool {
        self.tmux.as_ref().is_some_and(|tmux| tmux.target.starts_with(&format!("{session}:")))
            || self.route.as_ref().is_some_and(|route| route.via == "pty" && route.session.as_deref() == Some(session))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    #[default]
    Agent,
    Tool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatMessage {
    pub role: Role,
    pub text: String,
    pub at: Option<String>,
}

/// The open conversation (list modes only).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chat {
    pub agent_id: AgentId,
    pub messages: Vec<ChatMessage>,
    pub supported: bool,
    pub loading: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PanelMode {
    Inbox,
    Agents,
    NewSession,
    October,
    Assistant,
    Routines,
    ShowMe,
}

impl PanelMode {
    /// The inbox and agent list share tabs and chats; the others are standalone.
    pub fn is_list(self) -> bool {
        matches!(self, Self::Inbox | Self::Agents)
    }
}

/// A session starting in Lantern, shown as its conversation right away.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchPreview {
    pub kind: AgentKind,
    pub prompt: String,
    pub folder: String,
    /// Why it couldn't start.
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub group: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelList {
    pub choosable: bool,
    pub models: Vec<ModelOption>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HarnessState {
    #[default]
    Missing,
    Installing,
    Ready,
    Failed,
}

/// Cua Driver, which lets agents use this computer's apps (engine/src/computer.rs): "ready",
/// "installing", "failed" (with `message`) or "off".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ComputerStatus {
    pub status: String,
    pub message: Option<String>,
    pub version: Option<String>,
}

/// October Harness (engine/src/harness.rs).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HarnessStatus {
    pub status: HarnessState,
    /// "path" (the user's own `october`) or "bundled".
    pub source: Option<String>,
    pub version: Option<String>,
    pub message: Option<String>,
}

/// The app the person was in when they opened New Session or Point & Ask.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppContext {
    /// Display name ("DaVinci Resolve").
    pub name: String,
    /// Bundle id / executable name, for per-app task folders.
    pub id: String,
    pub is_terminal: bool,
    pub window: Option<String>,
    pub document: Option<PathBuf>,
    pub url: Option<String>,
    pub selected_text: Option<String>,
    /// Pid of a foreground terminal (Point & Ask recipient guess).
    pub pid: Option<i64>,
    /// The app's icon as a PNG Lantern saved (the "Work on <app>" row), when the app provides one.
    pub icon: Option<PathBuf>,
}

impl AppContext {
    /// The text added to the agent's first message for a Task (Swift `AppContext.Target.summary`),
    /// naming this computer the way the platform does.
    pub fn summary(&self, platform: Platform) -> String {
        let computer = match platform {
            Platform::Windows => "this Windows PC",
            Platform::Linux => "this Linux computer",
        };
        let mut lines = vec![format!("I'm working in {} on {computer}.", self.name)];
        if let Some(window) = self.window.as_deref().filter(|window| !window.is_empty()) {
            lines.push(format!("Its front window is \"{window}\"."));
        }
        if let Some(document) = &self.document {
            lines.push(format!("The open document is {}.", document.display()));
        }
        lines.join(" ")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Capture {
    #[default]
    None,
    Capturing,
    Ready {
        path: PathBuf,
    },
    Failed {
        message: String,
    },
    /// The system needs the person to allow screen capture first.
    PermissionNeeded,
}

/// New Session: what the engine and the system told us. The form itself is `Inputs::new_session`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NewSessionModel {
    /// Installed agents, most recently used first; `None` until the engine has checked.
    pub installed: Option<Vec<AgentKind>>,
    /// Installed agents signed in to the user's own plan, best first (engine `subscriptions`).
    pub signed_in: Vec<AgentKind>,
    /// Installed Claude Code or Codex never signed in: started, it would only ask you to sign in.
    pub never_set_up: Vec<AgentKind>,
    /// "In Lantern" sessions are possible (tmux on Linux, ConPTY sessions on Windows).
    pub in_lantern_available: bool,
    /// Models by kind, once listed.
    pub models: HashMap<String, ModelList>,
    /// Folders to offer: recent and running ones (deduplicated, home omitted).
    pub folders: Vec<String>,
    /// The app a Task would be for (captured before the panel took focus).
    pub task_app: Option<AppContext>,
    pub screenshot: Capture,
    pub launching: bool,
    pub launch_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistantTurn {
    pub id: u64,
    pub question: String,
    pub answer: String,
    pub failed: bool,
    pub note: Option<String>,
    /// "Started Claude Code in october-lantern", once a task from this answer launched.
    pub started: Option<String>,
    /// The agent this answer started, shown in the conversation.
    pub task: Option<TaskLink>,
    /// Why no agent could be started for this answer's task.
    pub task_problem: Option<String>,
    /// What the failed answer offers: upgrading, fixing your own key, or asking October's AI.
    pub fix: Option<Fix>,
    /// Your own key's provider answered ("OpenAI").
    pub via: Option<String>,
    /// Images you added to this message (paths in the screenshots folder).
    pub images: Vec<String>,
}

/// What became of an answer to an agent's question: on its way, typed in, or not sure it went in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerDelivery {
    Sending,
    Sent,
    Unconfirmed,
}

/// An agent Ask Lantern started (Swift `Assistant.TaskLink`): found by its Lantern session (tmux
/// on Linux, ConPTY on Windows), then by its id. Its last messages are kept, so the conversation
/// still shows what it did after it ends.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskLink {
    pub kind: AgentKind,
    pub folder: String,
    pub prompt: String,
    pub session: Option<String>,
    pub agent_id: Option<AgentId>,
    pub error: Option<String>,
    /// Started without a Lantern session (a Terminal window): nothing to show here.
    pub in_terminal: bool,
    /// Started in a terminal window because the agent has never been signed in.
    pub sign_in: bool,
    /// The images it was given, so Try again gives them again.
    pub images: Vec<String>,
    /// Its last 60 messages, as last seen.
    pub transcript: Vec<ChatMessage>,
    /// Sent to an agent that was already running, rather than started for this.
    pub sent: bool,
    /// How it reached that agent ("Sent through October Bus"), once the engine says.
    pub delivery: Option<String>,
    /// This send's own id, the same when it's tried again (October Bus keeps one copy), and when
    /// it was first sent (RFC 3339): what the agent said before then isn't its answer.
    pub send_key: Option<String>,
    pub sent_at: Option<String>,
    /// When it was started, and when Lantern first saw its agent (RFC 3339): for Send Logs.
    pub started_at: Option<String>,
    pub seen_at: Option<String>,
    /// The app log already says its agent wasn't seen in time.
    #[serde(skip)]
    pub unseen_noted: bool,
}

/// A saved Ask Lantern conversation (Swift `Assistant.Conversation`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    /// ISO 8601 (`2026-10-13T18:53:28Z`).
    pub updated: String,
    pub turns: Vec<AssistantTurn>,
}

/// A task the answer asked Lantern to start, waiting out its cancel window.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PendingTask {
    pub id: u64,
    pub turn: u64,
    pub prompt: String,
    pub kind: AgentKind,
    pub folder: String,
    /// From Point & Ask: the selection, which the agent gets, and where the user was.
    pub screenshot: Option<std::path::PathBuf>,
    pub context: Option<AppContext>,
    /// Images you added in the conversation (pasted, dropped or picked).
    pub images: Vec<std::path::PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistantModel {
    pub turns: Vec<AssistantTurn>,
    pub asking: bool,
    pub pending_task: Option<PendingTask>,
    /// Earlier conversations, newest first (the current one included once it has a question).
    pub conversations: Vec<Conversation>,
    /// With an agent at work in the conversation, the box talks to it; this sends the next
    /// message to Lantern instead.
    pub ask_lantern_instead: bool,
    /// The latest task's agent, read every two seconds like an open chat.
    pub watched: Option<Chat>,
    /// Images waiting to go with the next message (paths in the screenshots folder).
    pub attachments: Vec<String>,
    /// Why the last image couldn't be added (too many, unreadable).
    pub attach_problem: Option<String>,
}

impl AssistantModel {
    /// The latest task in the conversation, with its turn.
    pub fn latest_task(&self) -> Option<(u64, &TaskLink)> {
        self.turns.iter().rev().find_map(|turn| turn.task.as_ref().map(|link| (turn.id, link)))
    }
}

/// A conversation's title: its first question's first line, at most 60 characters (Swift
/// `Assistant.title`).
pub fn conversation_title(question: &str) -> String {
    let line = question.split(['\n', '\r']).find(|line| !line.is_empty()).unwrap_or(question);
    if line.chars().count() > 60 { format!("{}…", line.chars().take(59).collect::<String>()) } else { line.to_string() }
}

/// Milliseconds since the epoch as ISO 8601 in UTC, to the second.
pub fn iso8601(ms: f64) -> String {
    let seconds = (ms / 1000.0).floor() as i64;
    let (days, time) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", time / 3600, time % 3600 / 60, time % 60)
}

/// ISO 8601 (`2026-10-13T18:53:28Z`, fractions and offsets allowed) as milliseconds since the epoch.
pub fn parse_iso8601(text: &str) -> Option<f64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut rest = &text[19..];
    let mut fraction = 0.0_f64;
    if let Some(digits) = rest.strip_prefix('.') {
        let end = digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len());
        fraction = format!("0.{}", &digits[..end]).parse::<f64>().unwrap_or(0.0);
        rest = &digits[end..];
    }
    let offset = match rest.as_bytes().first() {
        Some(b'+' | b'-') if rest.len() >= 6 => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            sign * (rest.get(1..3)?.parse::<i64>().ok()? * 3600 + rest.get(4..6)?.parse::<i64>().ok()? * 60)
        }
        _ => 0,
    };
    let seconds = days_from_civil(year, month as u32, day as u32) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    Some((seconds as f64 + fraction) * 1000.0)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * ((month as i64 + 9) % 12) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AskTurn {
    pub question: String,
    pub answer: String,
    pub failed: bool,
    pub note: Option<String>,
    /// What the failed answer offers (see `AssistantTurn::fix`).
    pub fix: Option<Fix>,
    /// Your own key's provider answered ("OpenAI").
    pub via: Option<String>,
}

/// What a failed answer offers, beside its message (Swift `AnswerFix`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Fix {
    /// Out of free questions or credit, or your own key needs a plan: the trial and Lifetime
    /// offers, from this October page.
    Upgrade(String),
    /// Your provider didn't take your key: Settings, to change it.
    EditKey,
    /// Your provider couldn't use the model: Settings.
    OpenSettings,
    /// Your provider refused or failed before answering: ask again with October's AI (never
    /// automatic). Only while Lantern still has the question in memory.
    UseOctober,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PointAskPhase {
    #[default]
    Closed,
    /// The selection overlay is up.
    Selecting,
    /// The card is showing.
    Card,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PointAskModel {
    pub phase: PointAskPhase,
    pub screenshot: Capture,
    pub context: Option<AppContext>,
    /// The agent "Send to" goes to.
    pub recipient: Option<AgentId>,
    pub turns: Vec<AskTurn>,
    pub asking: bool,
    pub sending: bool,
    pub problem: Option<String>,
    pub listening: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub email: String,
    pub plan: Option<String>,
    /// What the plan gives in Lantern; `None` when October's plan didn't say.
    pub lantern: Option<LanternRights>,
    pub mobile_allowed: Option<bool>,
}

/// `lantern` in October's `/api/plan` (SQL `lantern_rights`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LanternRights {
    pub lifetime: bool,
    /// When the Pro trial ends (milliseconds since the epoch).
    pub trial_ends_ms: Option<f64>,
    /// Your own AI key may answer.
    pub byok: bool,
    /// October's AI questions a day (0: not known).
    pub daily_questions: u64,
}

impl LanternRights {
    pub fn from_plan(plan: &serde_json::Value) -> Option<Self> {
        let lantern = plan.get("lantern").filter(|value| value.is_object())?;
        Some(Self {
            lifetime: lantern["lifetime"] == true,
            trial_ends_ms: lantern["trialEndsAt"].as_str().and_then(parse_iso8601),
            byok: lantern["byok"] == true,
            daily_questions: lantern["dailyQuestions"].as_u64().unwrap_or(0),
        })
    }

    pub fn trial_running(&self, now_ms: f64) -> bool {
        self.trial_ends_ms.is_some_and(|ends| ends > now_ms)
    }
}

/// Your own AI key, as the views may know it: never the key itself (see `own_key`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum KeyStatus {
    /// Still being read from the keyring.
    #[default]
    Loading,
    Missing,
    Saved {
        provider: crate::own_key::AiProvider,
        /// "…abcd"
        ending: String,
        model: String,
    },
    /// The keyring couldn't be used (locked, or no Secret Service): nothing is saved elsewhere.
    Unavailable,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OwnKeyState {
    pub status: KeyStatus,
    /// Saving or removing.
    pub busy: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountState {
    pub signed_in: Option<Account>,
    /// A browser or email sign-in is in progress.
    pub pending: bool,
    /// The plan is being fetched again (Refresh).
    pub refreshing: bool,
    pub error: Option<String>,
}

/// The connection to October Desktop (engine/src/october_link.rs).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OctoberLink {
    /// notInstalled | notRunning | readOnly | connected | pairing | error
    pub status: String,
    pub core_version: Option<String>,
    pub paired: bool,
    pub pairing_code: Option<String>,
    pub message: Option<String>,
    pub agent_count: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", content = "message", rename_all = "camelCase")]
pub enum EngineHealth {
    #[default]
    Starting,
    Ready,
    Restarting,
    /// Stopped for good, with the reason.
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Toast {
    pub id: u64,
    pub text: String,
}

/// Dictation's first use asks before downloading anything: Download or Not now. Shown in the
/// Point & Ask card when that's where the mic was pressed, otherwise in the panel.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DictationOffer {
    pub text: String,
    pub point_ask: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    #[default]
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookState {
    #[default]
    Off,
    On,
    NeedsUpdate,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateInfo {
    pub version: String,
    pub newer: bool,
}

/// What Settings shows besides the preferences themselves.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemState {
    /// Claude Code and Codex hooks.
    pub hooks: HashMap<String, HookState>,
    pub hooks_error: Option<String>,
    /// Installing or removing the hooks.
    pub hooks_busy: bool,
    /// `None` while unknown.
    pub notifications_allowed: Option<bool>,
    pub update: Option<UpdateInfo>,
    pub checking_updates: bool,
    pub version: String,
    pub uninstall: Uninstall,
    /// Send Logs: the last report's outcome.
    pub logs: Option<LogsReport>,
}

/// Send Logs (Settings › About): sending, sent with its code, or saved to the Desktop instead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum LogsReport {
    Sending,
    Sent { code: String },
    Saved { path: String, error: String },
    Failed { error: String },
}

impl LogsReport {
    pub fn summary(&self) -> String {
        match self {
            Self::Sending => "Sending logs…".into(),
            Self::Sent { code } => format!("Logs sent: {code} (copied). Quote it to hey@october.dev."),
            Self::Saved { path, .. } => format!(
                "Couldn't send the logs. They're saved as {}: email that file to hey@october.dev.",
                std::path::Path::new(path).file_name().map_or(path.clone(), |name| name.to_string_lossy().into_owned())
            ),
            Self::Failed { error } => format!("Couldn't send the logs: {error}"),
        }
    }
}

/// Uninstalling Lantern (Settings › About, after the person confirmed it; Swift `Support.uninstall`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Uninstall {
    #[default]
    Idle,
    /// Signing out and removing hooks, support files, settings and desktop integrations.
    Removing,
    /// Some of that couldn't be removed; each entry says what, and how to remove it by hand. The
    /// app is still in place: the person chooses to remove it anyway or keep it.
    Partial { failures: Vec<String> },
    /// The app itself couldn't be removed (or this copy isn't an installed one).
    AppFailed { message: String },
}

/// Persisted preferences (Swift `Preferences` + window placement).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub edge: Edge,
    /// Top of the pill as a fraction of the work area's height, measured from the bottom.
    pub pill_top: f32,
    pub open_in_terminal: bool,
    /// The agent Ask Lantern starts tasks with (no picker; routing only), or empty to choose: your
    /// own signed-in plan first, then October. (It replaced `default_task_agent`, which was always
    /// "october".)
    pub task_agent: String,
    pub screenshot_new_sessions: bool,
    pub bus_for_sessions: bool,
    /// Let agents Lantern starts use this computer's apps through Cua Driver (engine `computer`).
    pub computer_use: bool,
    pub show_me: bool,
    /// Agents Lantern starts ask before each action (engine `askFirst`); off by default.
    pub ask_first: bool,
    pub toolkit_in_sessions: bool,
    pub notifications: bool,
    pub notify_finished: bool,
    /// Harnesses that don't count in Waiting, the badge or notifications.
    pub muted: Vec<String>,
    pub open_at_login: bool,
    pub message_hotkey: String,
    pub point_ask_hotkey: String,
    pub automatic_updates: bool,
    pub welcomed: bool,
    /// The welcome's page while it isn't finished, so the next launch picks up there.
    pub welcome_step: usize,
    /// Last model chosen per agent kind.
    pub model_by_kind: HashMap<String, String>,
    /// Last task folder per app id.
    pub task_folder_by_app: HashMap<String, String>,
    pub recent_folders: Vec<String>,
    /// Agent kinds, most recently used first.
    pub recent_kinds: Vec<String>,
    /// Where Lantern was installed (the folder its app came from), so uninstalling can remove it
    /// even when an update now runs from Lantern's data folder. Linux only; Windows asks its installer.
    pub bundle: Option<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            edge: Edge::Right,
            pill_top: 0.72,
            open_in_terminal: false,
            task_agent: String::new(),
            screenshot_new_sessions: false,
            bus_for_sessions: true,
            computer_use: true,
            show_me: true,
            ask_first: false,
            // As Preferences.swift registers them: finishing notifies, the toolkit list is opt-in.
            toolkit_in_sessions: false,
            notifications: true,
            notify_finished: true,
            muted: Vec::new(),
            open_at_login: false,
            message_hotkey: "ctrl+alt+space".into(),
            point_ask_hotkey: "ctrl+alt+p".into(),
            automatic_updates: true,
            welcomed: false,
            welcome_step: 0,
            model_by_kind: HashMap::new(),
            task_folder_by_app: HashMap::new(),
            recent_folders: Vec::new(),
            recent_kinds: Vec::new(),
            bundle: None,
        }
    }
}

/// Folders to offer for a new session (Swift `recentFolders`): ones you've used, then where agents
/// are running (in rank order); no repeats, never the home folder, at most 8.
pub fn offered_folders(prefs: &Prefs, agents: &[Agent], home: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    prefs
        .recent_folders
        .iter()
        .cloned()
        .chain(agents.iter().filter_map(|agent| agent.cwd.clone()))
        .filter(|folder| folder != home && seen.insert(folder.clone()))
        .take(8)
        .collect()
}

/// Shortcuts Settings offers to message an agent (Swift `HotKeyPreset`): ⌃⌥Space, ⇧⌘Space and ⌃⌥L
/// as Ctrl+Alt+Space, Ctrl+Shift+Space and Ctrl+Alt+L. The Mac's ⌥Space is left out: Alt+Space
/// opens the window menu on GNOME and Windows (CONTRACT §8).
pub const MESSAGE_HOTKEYS: [&str; 4] = ["ctrl+alt+space", "ctrl+shift+space", "ctrl+alt+l", "none"];
/// Point & Ask shortcuts (Swift `PointAskPreset`: ⌃⌥P, ⌃⌥K, None).
pub const POINT_ASK_HOTKEYS: [&str; 3] = ["ctrl+alt+p", "ctrl+alt+k", "none"];

/// "ctrl+alt+space" → "Ctrl+Alt+Space" (menus, Welcome and Settings show shortcuts this way).
pub fn hotkey_label(keys: &str) -> String {
    if keys == "none" {
        return "None".into();
    }
    keys.split('+')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or(String::new(), |first| first.to_uppercase().chain(chars).collect())
        })
        .collect::<Vec<_>>()
        .join("+")
}

impl Prefs {
    /// Whether a harness counts in Waiting, the badge and notifications.
    pub fn counts(&self, kind: &AgentKind) -> bool {
        !self.muted.iter().any(|muted| muted == kind.as_str())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    #[default]
    Linux,
    Windows,
}

/// What this platform can do (CONTRACT §8), so the UI shows the right controls and copy.
/// Everything not proven on a real machine stays `false` until it is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Caps {
    pub platform: Platform,
    /// The edge pill (Linux: needs an X server).
    pub edge_pill: bool,
    pub backdrop_blur: bool,
    /// Glass is filled denser than the Mac's so a sharp wallpaper doesn't show through: GNOME,
    /// which has no blur. Windows uses the Mac's tint.
    pub dense_glass: bool,
    /// The Mac's 6pt hover halo around the pill.
    pub hover_halo: bool,
    /// Point & Ask can pick a window under the pointer.
    pub window_hit_test: bool,
    pub foreground_app: bool,
    pub document_context: bool,
    pub url_context: bool,
    pub selected_text: bool,
    pub dictation: bool,
    pub global_hotkeys: bool,
    /// A tray host shows Lantern's status menu (GNOME needs the AppIndicator extension). Live.
    pub tray: bool,
}

/// Everything the UI reads. Field names are frozen (CONTRACT §3); changes go through the bus.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Model {
    pub control_held: bool,
    pub control_sending: bool,
    pub control_message: Option<String>,
    pub guide: Option<serde_json::Value>,
    pub routines: Vec<serde_json::Value>,
    pub routine_error: Option<String>,
    pub routine_pending: Option<(String, bool)>,
    pub panel: Option<PanelMode>,
    /// Agents in rank order (needs you, your turn, working, the rest; not-live last).
    pub agents: Vec<Agent>,
    pub inbox: Inbox,
    pub chat: Option<Chat>,
    pub launch_preview: Option<LaunchPreview>,
    /// A reply to the composer's recipient (`Inputs::drafts`) is on its way.
    pub sending: bool,
    /// What was sent for each question (by prompt id), shown as "Sent" on its card until the
    /// agent moves on.
    pub answered: std::collections::HashMap<String, AgentAnswer>,
    /// What became of each answer, by prompt id (Swift `AppModel.answerDelivery`).
    #[serde(skip)]
    pub answer_delivery: std::collections::HashMap<String, AnswerDelivery>,
    pub assistant: AssistantModel,
    pub new_session: NewSessionModel,
    pub point_ask: PointAskModel,
    pub account: AccountState,
    #[serde(skip_serializing)]
    pub phone: Option<phone::Phone>,
    pub own_key: OwnKeyState,
    pub october: Option<OctoberLink>,
    pub harness: Option<HarnessStatus>,
    /// Cua Driver's state, once the engine has reported it.
    pub computer: Option<ComputerStatus>,
    pub engine: EngineHealth,
    /// Sessions Lantern started with its card tools, newest first (engine `cards`).
    pub launches: Vec<CardLaunch>,
    /// Cards whose answer is on its way to the engine, and why one was refused.
    #[serde(skip)]
    pub card_sending: std::collections::HashSet<String>,
    #[serde(skip)]
    pub card_errors: HashMap<String, String>,
    pub toast: Option<Toast>,
    pub dictation_offer: Option<DictationOffer>,
    /// Bumped to move keyboard focus into the composer or the Ask box.
    pub compose_focus: u64,
    pub dictating: bool,
    pub pill_expanded: bool,
    pub system: SystemState,
    pub prefs: Prefs,
    pub caps: Caps,
}

impl Model {
    /// What October's plan gives in Lantern, while signed in and once it has said.
    pub fn lantern_rights(&self) -> Option<&LanternRights> {
        self.account.signed_in.as_ref()?.lantern.as_ref()
    }

    pub fn agent(&self, id: &str) -> Option<&Agent> {
        self.agents.iter().find(|agent| agent.id == id)
    }
    /// The session Lantern started for this agent, with its cards.
    pub fn launch_for(&self, agent_id: &str) -> Option<&CardLaunch> {
        self.launches.iter().find(|launch| launch.agent_id.as_deref() == Some(agent_id))
    }
    /// Agents with a card waiting for you.
    pub fn carded(&self) -> std::collections::HashSet<String> {
        self.launches.iter().filter(|launch| launch.waiting().next().is_some()).filter_map(|launch| launch.agent_id.clone()).collect()
    }
    /// Sessions that have ended, with something to look back at.
    pub fn ended_launches(&self) -> impl Iterator<Item = &CardLaunch> {
        self.launches.iter().filter(|launch| launch.ended_at.is_some() && !launch.cards.is_empty())
    }
    pub fn chat_agent(&self) -> Option<&Agent> {
        self.chat.as_ref().and_then(|chat| self.agent(&chat.agent_id))
    }
    /// The agent a task started, once it's up: by id once known, else by its Lantern session.
    pub fn task_agent(&self, link: &TaskLink) -> Option<&Agent> {
        if let Some(id) = &link.agent_id {
            return self.agent(id);
        }
        let session = link.session.as_deref()?;
        self.agents.iter().find(|agent| agent.in_session(session))
    }
    /// The latest tasks, for Send Logs (Swift `Assistant.reportLines`): what each shows and what
    /// Lantern knows about it. Never the message or the folder.
    pub fn task_report(&self) -> String {
        let current = &self.assistant.turns;
        let earlier = self.assistant.conversations.iter().flat_map(|c| &c.turns).filter(|t| !current.iter().any(|c| c.id == t.id));
        let mut tasks: Vec<&TaskLink> = current.iter().chain(earlier).filter_map(|t| t.task.as_ref()).filter(|l| !l.sent).collect();
        tasks.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        if tasks.is_empty() {
            return "none".into();
        }
        let lines: Vec<String> = tasks
            .iter()
            .take(5)
            .map(|link| {
                let mut parts = vec![
                    link.kind.as_str().to_string(),
                    format!("started {}", link.started_at.as_deref().unwrap_or("?")),
                    format!("session {}", link.session.as_deref().unwrap_or("none")),
                ];
                if let Some(id) = &link.agent_id {
                    parts.push(format!("agent {id}"));
                }
                if let Some(seen) = &link.seen_at {
                    parts.push(format!("seen {seen}"));
                }
                parts.push(format!("shows: {}", self.task_shown(link)));
                format!("- {}", parts.join(", "))
            })
            .collect();
        lines.join("\n")
    }

    /// What the conversation shows for a task, in words.
    fn task_shown(&self, link: &TaskLink) -> String {
        let agent = self.task_agent(link);
        if let Some(error) = &link.error {
            return format!("couldn't start it: {error}");
        }
        if link.sign_in && agent.is_none_or(|a| a.state != AgentState::Working) {
            return "needs sign-in in a terminal window".into();
        }
        if let Some(agent) = agent {
            return format!("agent {:?}", agent.state);
        }
        if link.in_terminal {
            return "running in a terminal window".into();
        }
        if link.agent_id.is_some() || !link.transcript.is_empty() {
            return "closed".into();
        }
        "Getting started (no agent seen yet)".into()
    }

    /// The running agent from the latest task in Ask Lantern's conversation: the box replies to it.
    pub fn assistant_agent(&self) -> Option<&Agent> {
        let (_, link) = self.assistant.latest_task()?;
        self.task_agent(link).filter(|agent| agent.is_live())
    }
    pub fn badge(&self) -> usize {
        self.inbox.new.len()
    }
    pub fn any_working(&self) -> bool {
        self.agents.iter().any(|agent| agent.state == AgentState::Working)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_an_engine_agent() {
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "id": "claude:42:1790180000", "kind": "claude", "handle": "claude-3", "pid": 42, "state": "needs_input",
            "stateSince": 1790180000123u64, "questionKind": "permission", "promptId": "9f2c", "canReply": true,
            "route": {"via": "pty", "session": "claude-proj-1"}, "stateSource": "hook", "exe": "ignored"
        }))
        .unwrap();
        assert_eq!(agent.state, AgentState::NeedsInput);
        assert_eq!(agent.turn_key(), "claude:42:1790180000@1790180000123");
        assert!(agent.is_answerable());
        assert!(agent.in_session("claude-proj-1") && !agent.in_session("claude-proj"));
        assert!(agent.is_live());
    }

    #[test]
    fn unknown_states_and_kinds_are_kept_open() {
        let agent: Agent = serde_json::from_value(serde_json::json!({"id": "x", "kind": "newharness", "state": "thinking"})).unwrap();
        assert_eq!(agent.state, AgentState::Unknown);
        assert_eq!(agent.kind.display_name(), "Newharness");
        assert_eq!(AgentState::Unknown.label(), "Running");
    }

    #[test]
    fn task_context_names_the_app_window_and_document_per_platform() {
        let context = AppContext {
            name: "DaVinci Resolve".into(),
            window: Some("video-3".into()),
            document: Some("/home/me/video-3.drp".into()),
            ..Default::default()
        };
        assert_eq!(
            context.summary(Platform::Linux),
            "I'm working in DaVinci Resolve on this Linux computer. Its front window is \"video-3\". The open document is /home/me/video-3.drp."
        );
        assert!(context.summary(Platform::Windows).contains("on this Windows PC."));
    }

    #[test]
    fn offered_folders_put_used_ones_first_and_skip_home_and_repeats() {
        let prefs = Prefs { recent_folders: vec!["/p/a".into(), "/home/me".into()], ..Default::default() };
        let agent = |cwd: &str| Agent { cwd: Some(cwd.into()), ..Default::default() };
        let agents: Vec<Agent> = ["/p/b", "/p/a", "/home/me"].into_iter().chain((0..10).map(|_| "/p/c")).map(agent).collect();
        assert_eq!(offered_folders(&prefs, &agents, "/home/me"), ["/p/a", "/p/b", "/p/c"]);
        let many: Vec<Agent> = (0..12).map(|n| agent(&format!("/p/{n}"))).collect();
        assert_eq!(offered_folders(&Prefs::default(), &many, "/home/me").len(), 8);
    }

    #[test]
    fn titles_are_the_first_line_cut_at_60_characters() {
        assert_eq!(conversation_title("Find me my thesis\nIt's a PDF"), "Find me my thesis");
        assert_eq!(conversation_title("\nSecond line first"), "Second line first");
        let long = "é".repeat(61);
        assert_eq!(conversation_title(&long), format!("{}…", "é".repeat(59)));
        assert_eq!(conversation_title(&"a".repeat(60)), "a".repeat(60));
    }

    #[test]
    fn iso_dates_round_trip() {
        assert_eq!(iso8601(1_790_000_000_000.0), "2026-09-21T14:13:20Z");
        assert_eq!(parse_iso8601("2026-09-21T14:13:20Z"), Some(1_790_000_000_000.0));
        assert_eq!(parse_iso8601("2026-09-21T16:13:20.5+02:00"), Some(1_790_000_000_500.0));
        assert_eq!(iso8601(951_782_400_000.0), "2000-02-29T00:00:00Z");
        assert_eq!(parse_iso8601("yesterday"), None);
    }

    #[test]
    fn tmux_sessions_match_by_target_prefix() {
        let agent = Agent { tmux: Some(TmuxPane { target: "codex-proj-1:0.0".into(), ..Default::default() }), ..Default::default() };
        assert!(agent.in_session("codex-proj-1"));
        assert!(!agent.in_session("codex-proj"));
    }
}
