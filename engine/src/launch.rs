//! Starting new agent sessions.
//!
//! With tmux installed, a new session runs in a tmux session on Lantern's own server
//! (`tmux -L lantern`), so Lantern can type replies into it. "Terminal" mode then opens a
//! Terminal window attached to it; "background" mode leaves it detached until you open it.
//! Without tmux, the agent runs directly in a new Terminal window.
//!
//! Agents are started through the user's login shell (`$SHELL -lic`), so they get the same PATH
//! and API keys as in a terminal they opened themselves.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::hooks::{now_ms, support_dir};
use crate::model::{Kind, TmuxPane};
use crate::tmux;

pub const LANTERN_SOCKET: &str = "lantern";

const ALL: [Kind; 17] = [
    Kind::Claude,
    Kind::Codex,
    Kind::Opencode,
    Kind::Pi,
    Kind::Gemini,
    Kind::Grok,
    Kind::Cursor,
    Kind::Qwen,
    Kind::Goose,
    Kind::Aider,
    Kind::Amp,
    Kind::Copilot,
    Kind::Kimi,
    Kind::Droid,
    Kind::Crush,
    Kind::Auggie,
    Kind::October,
];

pub fn program(kind: Kind) -> &'static str {
    match kind {
        Kind::Cursor => "cursor-agent",
        other => other.as_str(),
    }
}

fn shell() -> String {
    std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into())
}

/// Single-quote for POSIX shells.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub kinds: Vec<Kind>,
    pub tmux: bool,
}

/// Which agents are installed, according to the user's login shell. Cached: it takes a moment.
pub fn installed() -> &'static Installed {
    static CACHE: OnceLock<Installed> = OnceLock::new();
    CACHE.get_or_init(|| {
        let names: Vec<&str> = ALL.iter().map(|k| program(*k)).collect();
        let script = format!("for c in {}; do command -v \"$c\" >/dev/null 2>&1 && echo \"$c\"; done", names.join(" "));
        let found: Vec<String> = Command::new(shell())
            .args(["-lic", &script])
            .stdin(std::process::Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).collect())
            .unwrap_or_default();
        Installed {
            kinds: ALL.iter().copied().filter(|k| found.iter().any(|f| f == program(*k))).collect(),
            tmux: Path::new(tmux::tmux_bin()).is_absolute(),
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Terminal,
    Background,
}

pub struct Launched {
    pub session: Option<String>,
}

/// Harnesses that take the first prompt as a command-line argument.
fn takes_prompt_arg(kind: Kind) -> bool {
    matches!(kind, Kind::Claude | Kind::Codex)
}

fn agent_command(kind: Kind, cwd: &Path, prompt: Option<&str>) -> String {
    let mut cmd = format!("cd {} && exec {}", quote(&cwd.to_string_lossy()), program(kind));
    if let Some(p) = prompt.filter(|_| takes_prompt_arg(kind)) {
        cmd.push(' ');
        cmd.push_str(&quote(p));
    }
    format!("exec {} -lic {}", shell(), quote(&cmd))
}

/// Opens a new Terminal window running `script` (through a `.command` file, which needs no
/// Automation permission).
fn open_in_terminal(name: &str, script: &str) -> Result<()> {
    let dir = support_dir().join("launch");
    std::fs::create_dir_all(&dir)?;
    let path: PathBuf = dir.join(format!("{name}.command"));
    std::fs::write(&path, format!("#!/bin/sh\nrm -f {}\n{script}\n", quote(&path.to_string_lossy())))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    let status = Command::new("/usr/bin/open").args(["-a", "Terminal"]).arg(&path).status()?;
    if !status.success() {
        bail!("couldn't open Terminal");
    }
    Ok(())
}

pub fn launch(kind: Kind, cwd: &Path, prompt: Option<&str>, mode: Mode) -> Result<Launched> {
    if !cwd.is_dir() {
        bail!("{} is not a folder", cwd.display());
    }
    if !installed().kinds.contains(&kind) {
        bail!("{} isn't installed", program(kind));
    }
    let prompt = prompt.map(str::trim).filter(|p| !p.is_empty());

    if !installed().tmux {
        if mode == Mode::Background {
            bail!("background sessions need tmux (brew install tmux)");
        }
        let name = format!("{}-{}", kind.as_str(), now_ms());
        open_in_terminal(&name, &agent_command(kind, cwd, prompt))?;
        return Ok(Launched { session: None });
    }

    let project = cwd.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let safe: String = project.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).take(24).collect();
    let session = format!("{}-{}-{}", kind.as_str(), safe, now_ms() % 100_000);
    let status = Command::new(tmux::tmux_bin())
        .args(["-L", LANTERN_SOCKET, "new-session", "-d", "-s", &session, "-x", "200", "-y", "50", "-c"])
        .arg(cwd)
        .arg(agent_command(kind, cwd, prompt))
        .status()
        .context("starting tmux")?;
    if !status.success() {
        bail!("tmux couldn't start the session");
    }

    // Harnesses without a prompt argument get the first prompt typed in once their UI is up.
    if let Some(p) = prompt.filter(|_| !takes_prompt_arg(kind)) {
        let pane = TmuxPane { socket: Some(LANTERN_SOCKET.into()), target: session.clone(), pane_id: format!("{session}:0.0") };
        let text = p.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(4));
            let _ = tmux::send(&pane, &text);
        });
    }

    if mode == Mode::Terminal {
        let attach = format!("exec {} -L {} attach -t {}", quote(tmux::tmux_bin()), LANTERN_SOCKET, quote(&session));
        open_in_terminal(&session, &attach)?;
    }
    Ok(Launched { session: Some(session) })
}

/// Opens a Terminal window attached to a tmux session nobody is looking at.
pub fn attach_in_terminal(pane: &TmuxPane) -> Result<()> {
    let socket = match &pane.socket {
        Some(s) if s.starts_with('/') => format!("-S {}", quote(s)),
        Some(s) => format!("-L {}", quote(s)),
        None => String::new(),
    };
    let session = pane.target.split(':').next().unwrap_or(&pane.target);
    let attach = format!("exec {} {socket} attach -t {}", quote(tmux::tmux_bin()), quote(session));
    open_in_terminal(&format!("attach-{}", now_ms()), &attach)
}
