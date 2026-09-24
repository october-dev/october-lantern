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

/// How an agent is named on October Bus.
fn display_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Claude => "Claude Code",
        Kind::Codex => "Codex",
        Kind::Opencode => "OpenCode",
        Kind::October => "October",
        other => other.as_str(),
    }
}

pub fn program(kind: Kind) -> &'static str {
    match kind {
        Kind::Cursor => "cursor-agent",
        other => other.as_str(),
    }
}

pub(crate) fn shell() -> String {
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
    /// Why the agent started without October Bus, when it was asked for.
    pub bus_problem: Option<String>,
}

/// How a harness takes its first message on the command line: as the last argument (after
/// `--`), or as an option's value. Harnesses with neither have it typed in once they're up.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PromptArg {
    Positional,
    Option(&'static str),
}

fn prompt_arg(kind: Kind) -> Option<PromptArg> {
    match kind {
        Kind::Claude | Kind::Codex | Kind::Grok | Kind::October | Kind::Pi | Kind::Gemini => Some(PromptArg::Positional),
        Kind::Opencode => Some(PromptArg::Option("--prompt")),
        _ => None,
    }
}

fn takes_prompt_arg(kind: Kind) -> bool {
    prompt_arg(kind).is_some()
}

/// Where the app saves screenshots for new sessions; the only folder `launch` accepts them from.
pub fn screenshots_dir() -> PathBuf {
    support_dir().join("screenshots")
}

/// The first message, with the screenshot's path added so any agent can open it.
/// What goes with the first message, besides your words.
#[derive(Default, Clone, Copy)]
pub struct Extras<'a> {
    /// A screenshot the app saved (in the screenshots folder).
    pub screenshot: Option<&'a Path>,
    /// What you're working in, from the app (a task: the app, its window, its document).
    pub context: Option<&'a str>,
    /// The toolkit list (toolkit.md).
    pub toolkit: Option<&'a str>,
    /// Connect the agent to October Bus with the other agents Lantern started.
    pub bus: bool,
    /// What the first message says about October Bus (set by `launch`).
    pub bus_note: Option<&'a str>,
}

/// The first message: your words, then the context, the screenshot's path and the toolkit list.
/// Nothing is sent when there are no words and no screenshot (an agent would act on context
/// alone).
pub fn first_message(prompt: Option<&str>, extras: Extras) -> Option<String> {
    let mut out = match (prompt, extras.screenshot) {
        (Some(p), _) => p.to_string(),
        (None, Some(_)) => "Take a look at the screenshot below, then wait for my instructions.".into(),
        (None, None) => return None,
    };
    if let Some(c) = extras.context.map(str::trim).filter(|c| !c.is_empty()) {
        out.push_str(&format!("\n\n{c}"));
    }
    if let Some(s) = extras.screenshot {
        out.push_str(&format!("\n\nFor context, a screenshot of my screen when I started this session: {}", s.display()));
    }
    if let Some(n) = extras.bus_note {
        out.push_str(&format!("\n\n{n}"));
    }
    if let Some(t) = extras.toolkit.map(str::trim).filter(|t| !t.is_empty()) {
        out.push_str(&format!("\n\nWhat this Mac already has (from Lantern's toolkit list; prefer these over installing new tools):\n{t}"));
    }
    Some(out)
}

pub(crate) fn agent_command(
    kind: Kind,
    cwd: &Path,
    prompt: Option<&str>,
    screenshot: Option<&Path>,
    model: Option<&str>,
    bus: Option<&crate::bus::Attach>,
) -> String {
    // On October Bus: its environment, a wrapper command (the October harness), or arguments.
    let env: String = bus.map(|b| b.env.iter().map(|(k, v)| format!("{k}={} ", quote(v))).collect()).unwrap_or_default();
    let wrap: String = bus.map(|b| b.wrap.iter().map(|w| format!("{w} ")).collect()).unwrap_or_default();
    let mut cmd = format!("cd {} && {env}exec {wrap}{}", quote(&cwd.to_string_lossy()), program(kind));
    for arg in bus.map(|b| b.args.as_slice()).unwrap_or_default() {
        cmd.push(' ');
        cmd.push_str(arg);
    }
    if let (Some(m), Some(flag)) = (model, crate::models::flag(kind)) {
        cmd.push_str(&format!(" {flag} {}", quote(m)));
    }
    // Let the agent read the screenshot without asking (it's outside the project), or attach it.
    if let Some(s) = screenshot {
        let dir = quote(&s.parent().unwrap_or(s).to_string_lossy());
        match kind {
            Kind::Claude => cmd.push_str(&format!(" --add-dir {dir}")),
            Kind::Codex => cmd.push_str(&format!(" --image {}", quote(&s.to_string_lossy()))),
            Kind::Gemini => cmd.push_str(&format!(" --include-directories {dir}")),
            _ => {}
        }
    }
    // Claude Code gets a session id of Lantern's choosing, so its conversation is matched
    // exactly from the start (see scanner::transcript_status).
    if kind == Kind::Claude {
        cmd.push_str(&format!(" --session-id {}", uuid::Uuid::new_v4().hyphenated()));
    }
    match (prompt, prompt_arg(kind)) {
        // `--` so a message starting with `-` isn't read as an option.
        (Some(p), Some(PromptArg::Positional)) => cmd.push_str(&format!(" -- {}", quote(p))),
        (Some(p), Some(PromptArg::Option(flag))) => cmd.push_str(&format!(" {flag} {}", quote(p))),
        _ => {}
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

pub fn launch(kind: Kind, cwd: &Path, prompt: Option<&str>, extras: Extras, model: Option<&str>, mode: Mode) -> Result<Launched> {
    let screenshot = extras.screenshot;
    if !cwd.is_dir() {
        bail!("{} is not a folder", cwd.display());
    }
    if !installed().kinds.contains(&kind) {
        bail!("{} isn't installed", program(kind));
    }
    if let Some(s) = screenshot {
        let inside = s.canonicalize().ok().zip(screenshots_dir().canonicalize().ok()).is_some_and(|(s, dir)| s.starts_with(dir));
        if !inside || !s.is_file() {
            bail!("the screenshot wasn't saved; try again");
        }
    }
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    if let Some(m) = model {
        if crate::models::flag(kind).is_none() {
            bail!("Lantern doesn't know how to choose a model for {}; leave the model on Default", program(kind));
        }
        if !crate::models::valid(m) {
            bail!("\"{m}\" doesn't look like a model id");
        }
    }
    // October Bus: a new identity for this agent, and the tools to reach the others.
    let mut bus_problem = None;
    let bus = if extras.bus && crate::bus::supported(kind) {
        let project = cwd.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let name = format!("{} · {project}", display_name(kind));
        let id = crate::bus::new_id(kind);
        match crate::bus::ensure_ready().and_then(|_| crate::bus::attach(kind, &id, &name)) {
            Ok(a) => Some(a),
            Err(e) => {
                bus_problem = Some(format!("{e:#}"));
                None
            }
        }
    } else {
        None
    };
    let note = bus.as_ref().map(|b| crate::bus::note(&b.name));
    let extras = Extras { bus_note: note.as_deref(), ..extras };
    let message = first_message(prompt.map(str::trim).filter(|p| !p.is_empty()), extras);
    let prompt = message.as_deref();

    if !installed().tmux {
        if mode == Mode::Background {
            bail!("background sessions need tmux (brew install tmux)");
        }
        if prompt.is_some() && !takes_prompt_arg(kind) {
            bail!("Without tmux, Lantern can't hand {} a first message. Leave the message empty, or install tmux.", program(kind));
        }
        let name = format!("{}-{}", kind.as_str(), now_ms());
        open_in_terminal(&name, &agent_command(kind, cwd, prompt, screenshot, model, bus.as_ref()))?;
        if let Some(b) = &bus {
            crate::bus::link_when_ready(b.id.clone());
        }
        return Ok(Launched { session: None, bus_problem });
    }

    let project = cwd.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let safe: String = project.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).take(24).collect();
    let session = format!("{}-{}-{}", kind.as_str(), safe, now_ms() % 100_000);
    let status = &mut Command::new(tmux::tmux_bin());
    let status = status
        .args(["-L", LANTERN_SOCKET, "new-session", "-d", "-s", &session, "-x", "200", "-y", "50", "-c"])
        .arg(cwd)
        .arg(agent_command(kind, cwd, prompt, screenshot, model, bus.as_ref()));
    let out = crate::run::output(status, Duration::from_secs(10)).context("starting tmux")?;
    if !out.status.success() {
        bail!("tmux couldn't start the session");
    }

    if let Some(b) = &bus {
        crate::bus::link_when_ready(b.id.clone());
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
    Ok(Launched { session: Some(session), bus_problem })
}

/// Waits until something other than a shell runs on the pane's terminal (the agent, even when a
/// wrapper script starts it as a child), and its screen has stopped changing (or it has been up
/// for 5 s, for interfaces that animate), then types. Gives up after 30 s.
fn type_first_prompt(pane: &TmuxPane, kind: Kind, text: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let (mut last, mut steady, mut up_since) = (String::new(), 0, None::<Instant>);
    loop {
        if Instant::now() >= deadline {
            bail!("{} didn't finish starting within 30 s; type your message in its window", program(kind));
        }
        std::thread::sleep(Duration::from_millis(300));
        let out = tmux::output(pane, &["display-message", "-p", "-t", &pane.pane_id, "#{pane_pid}\t#{pane_tty}"])?;
        let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let (pid, tty) = line.split_once('\t').context("tmux didn't describe the pane")?;
        let pid: u32 = pid.parse().context("tmux didn't say which process runs the pane")?;
        if crate::procs::live(pid).is_none() {
            bail!("{} exited", program(kind));
        }
        // The agent itself, or the runtime it's written in (not a shell plugin's helper).
        let running = crate::procs::on_tty(tty.trim_start_matches("/dev/")).iter().flatten().any(|exe| {
            let name = crate::procs::basename(exe);
            exe.contains(program(kind)) || ["node", "bun", "deno", "python", "uv"].iter().any(|r| name.starts_with(r))
        });
        if !running {
            steady = 0;
            up_since = None;
            continue;
        }
        let up = *up_since.get_or_insert_with(Instant::now);
        let screen = String::from_utf8_lossy(&tmux::output(pane, &["capture-pane", "-p", "-t", &pane.pane_id])?.stdout).into_owned();
        if !screen.trim().is_empty() && screen == last {
            steady += 1;
        } else {
            steady = 0;
            last = screen;
        }
        if steady >= 3 || up.elapsed() >= Duration::from_secs(5) {
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
