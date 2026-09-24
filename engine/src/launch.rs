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
use std::time::{Duration, Instant};

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
/// A shell whose startup files take longer than 10 s counts as finding nothing.
pub fn installed() -> &'static Installed {
    static CACHE: OnceLock<Installed> = OnceLock::new();
    CACHE.get_or_init(|| {
        let names: Vec<&str> = ALL.iter().map(|k| program(*k)).collect();
        let script = format!("for c in {}; do command -v \"$c\" >/dev/null 2>&1 && echo \"$c\"; done", names.join(" "));
        let found: Vec<String> =
            crate::deliver::output_within(Command::new(shell()).args(["-lic", &script]), Duration::from_secs(10), false)
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
        // `--` so a message starting with `-` isn't read as an option.
        cmd.push_str(" -- ");
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
        if prompt.is_some() && !takes_prompt_arg(kind) {
            bail!("Without tmux, Lantern can't hand {} a first message. Leave the message empty, or install tmux.", program(kind));
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

    if mode == Mode::Terminal {
        let attach = format!("exec {} -L {} attach -t {}", quote(tmux::tmux_bin()), LANTERN_SOCKET, quote(&session));
        open_in_terminal(&session, &attach)?;
    }

    // Harnesses without a prompt argument get the first prompt typed in once their UI is up.
    if let Some(p) = prompt.filter(|_| !takes_prompt_arg(kind)) {
        let pane = TmuxPane { socket: Some(LANTERN_SOCKET.into()), target: session.clone(), pane_id: format!("{session}:0.0") };
        type_first_prompt(&pane, kind, p).context("the session started, but its first message wasn't sent")?;
    }
    Ok(Launched { session: Some(session) })
}

const SHELLS: [&str; 6] = ["sh", "bash", "zsh", "fish", "dash", "login"];

/// Waits until the pane's process is the agent (not the shell starting it), in the foreground,
/// with a screen that has stopped changing, then types. Gives up after 20 s.
fn type_first_prompt(pane: &TmuxPane, kind: Kind, text: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let (mut last, mut steady) = (String::new(), 0);
    loop {
        if Instant::now() >= deadline {
            bail!("{} didn't finish starting within 20 s; type your message in its window", program(kind));
        }
        std::thread::sleep(Duration::from_millis(300));
        let out = tmux::output(pane, &["display-message", "-p", "-t", &pane.pane_id, "#{pane_pid}"])?;
        let pid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().context("tmux didn't say which process runs the pane")?;
        let Some(live) = crate::procs::live(pid) else { bail!("{} exited", program(kind)) };
        let exe = live.exe.as_deref().map(crate::procs::basename).unwrap_or("");
        if exe.is_empty() || SHELLS.contains(&exe) || live.tpgid != live.pgid {
            steady = 0;
            continue;
        }
        let screen = String::from_utf8_lossy(&tmux::output(pane, &["capture-pane", "-p", "-t", &pane.pane_id])?.stdout).into_owned();
        if !screen.trim().is_empty() && screen == last {
            steady += 1;
        } else {
            steady = 0;
            last = screen;
        }
        if steady >= 3 {
            return tmux::send(pane, text);
        }
    }
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
