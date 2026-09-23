//! Types sent to the app. See `protocol/README.md`.

use serde::{Deserialize, Serialize};

/// Every harness Lantern can detect. Only Claude Code and Codex have session-file readers so far;
/// the rest show as running until they get readers or hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Claude,
    Codex,
    Opencode,
    Pi,
    Gemini,
    Grok,
    Cursor,
    Qwen,
    Goose,
    Aider,
    Amp,
    Copilot,
    Kimi,
    Droid,
    Crush,
    Auggie,
    /// The October harness (`october`), a Pi fork.
    October,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Claude => "claude",
            Kind::Codex => "codex",
            Kind::Opencode => "opencode",
            Kind::Pi => "pi",
            Kind::Gemini => "gemini",
            Kind::Grok => "grok",
            Kind::Cursor => "cursor",
            Kind::Qwen => "qwen",
            Kind::Goose => "goose",
            Kind::Aider => "aider",
            Kind::Amp => "amp",
            Kind::Copilot => "copilot",
            Kind::Kimi => "kimi",
            Kind::Droid => "droid",
            Kind::Crush => "crush",
            Kind::Auggie => "auggie",
            Kind::October => "october",
        }
    }

    /// The executable name each harness runs as.
    pub fn from_program(prog: &str) -> Option<Kind> {
        Some(match prog {
            "claude" => Kind::Claude,
            "codex" => Kind::Codex,
            "opencode" => Kind::Opencode,
            "pi" => Kind::Pi,
            "gemini" => Kind::Gemini,
            "grok" => Kind::Grok,
            "cursor-agent" => Kind::Cursor,
            "qwen" => Kind::Qwen,
            "goose" => Kind::Goose,
            "aider" => Kind::Aider,
            "amp" => Kind::Amp,
            "copilot" => Kind::Copilot,
            "kimi" => Kind::Kimi,
            "droid" => Kind::Droid,
            "crush" => Kind::Crush,
            "auggie" => Kind::Auggie,
            "october" => Kind::October,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// The agent is doing something.
    Working,
    /// The agent finished its turn and has something for you.
    Waiting,
    /// The agent is blocked on a question or permission prompt.
    NeedsInput,
    /// The agent is open but nothing has happened yet.
    Idle,
    /// Lantern can't tell.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StateSource {
    Hook,
    Transcript,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostApp {
    pub app: String,
    pub pid: u32,
    pub bundle_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxPane {
    /// `-L` socket name, or `-S` socket path when it starts with `/`. `None` is the default server.
    pub socket: Option<String>,
    /// `session:window.pane`, for display.
    pub target: String,
    pub pane_id: String,
}

/// How Lantern can type into an agent. See `deliver.rs`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "via", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum Route {
    /// Through October Desktop's own safe delivery (Lantern is paired with October).
    October { canvas_id: String, node_id: String },
    Tmux,
    Cmux { workspace: String, surface: String },
    Terminal { tty: String },
    Iterm { tty: String },
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub kind: Kind,
    pub handle: String,
    pub pid: u32,
    pub tty: Option<String>,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub title: Option<String>,
    pub session_id: Option<String>,
    pub state: State,
    pub state_since: Option<u64>,
    pub last_message: Option<String>,
    pub question: Option<String>,
    pub host: Option<HostApp>,
    pub tmux: Option<TmuxPane>,
    pub can_reply: bool,
    pub route: Route,
    pub state_source: StateSource,
}

/// What a transcript reader or hook file says about a session.
#[derive(Debug, Clone, Default)]
pub struct SessionStatus {
    pub state: Option<State>,
    pub since: Option<u64>,
    pub last_message: Option<String>,
    pub question: Option<String>,
    pub title: Option<String>,
    pub session_id: Option<String>,
}

pub fn truncate(s: &str, max_chars: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}
