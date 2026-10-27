//! Starting new agent sessions.
//!
//! With tmux installed, a new session runs in a tmux session on Lantern's own server
//! (`tmux -L lantern`), so Lantern can type replies into it. "Terminal" mode then opens a
//! Terminal window attached to it; "background" mode leaves it detached until you open it.
//! Without tmux, the agent runs directly in a new Terminal window.
//!
//! Agents are started through the user's login shell (`$SHELL -lic`), so they get the same PATH
//! and API keys as in a terminal they opened themselves.

#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::hooks::{now_ms, support_dir};
use crate::model::{Kind, TmuxPane};
use crate::tmux;

#[cfg_attr(windows, allow(dead_code))]
pub const LANTERN_SOCKET: &str = "lantern";

const ALL: [Kind; 18] = [
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
    Kind::Muse,
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

/// How an agent is named in messages, as the apps name it.
fn agent_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Gemini => "Gemini CLI",
        Kind::Qwen => "Qwen Code",
        Kind::Pi => "Pi",
        Kind::Grok => "Grok",
        Kind::Cursor => "Cursor",
        Kind::Goose => "Goose",
        Kind::Aider => "Aider",
        Kind::Amp => "Amp",
        Kind::Copilot => "Copilot",
        Kind::Kimi => "Kimi",
        Kind::Droid => "Droid",
        Kind::Crush => "Crush",
        Kind::Auggie => "Auggie",
        Kind::Muse => "Muse",
        other => display_name(other),
    }
}

pub fn program(kind: Kind) -> &'static str {
    match kind {
        Kind::Cursor => "cursor-agent",
        other => other.as_str(),
    }
}

#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| if cfg!(target_os = "macos") { "/bin/zsh".into() } else { "/bin/bash".into() })
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
    /// Installed agents signed in to the user's own plan, in the order Ask Lantern prefers them
    /// (see `subscriptions`). Filled in by `installed`.
    pub signed_in: Vec<Kind>,
    /// Installed agents that have never been set up (signed in), so they're started where the
    /// user can see them. Filled in by `installed`.
    pub never_set_up: Vec<Kind>,
}

/// Which agents are installed, according to the user's login shell. Cached: it takes a moment;
/// `refresh` forgets it, so an agent installed while Lantern runs shows up. A shell whose
/// startup files take longer than 10 s isn't believed: the usual install folders are looked in
/// instead, and the shell is asked again a minute later. On Windows, looked up each time (it's quick).
pub fn on_path() -> std::borrow::Cow<'static, Installed> {
    #[cfg(windows)]
    {
        let kinds: Vec<Kind> = ALL.iter().copied().filter(|k| crate::platform::find_on_path(program(*k)).is_some()).collect();
        static CODEX: std::sync::Once = std::sync::Once::new();
        if kinds.contains(&Kind::Codex) {
            CODEX.call_once(check_codex_early);
        }
        std::borrow::Cow::Owned(Installed { kinds, tmux: false, signed_in: Vec::new(), never_set_up: Vec::new() })
    }
    #[cfg(unix)]
    {
        let mut cache = ON_PATH.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((i, expires)) = cache.as_ref()
            && expires.is_none_or(|t| Instant::now() < t)
        {
            return std::borrow::Cow::Owned(i.clone());
        }
        let names: Vec<&str> = ALL.iter().map(|k| program(*k)).collect();
        let script = format!("for c in {}; do command -v \"$c\" >/dev/null 2>&1 && echo \"$c\"; done", names.join(" "));
        let shell_found: Option<Vec<String>> =
            crate::deliver::output_within(Command::new(shell()).args(["-lic", &script]), Duration::from_secs(10), false)
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).collect());
        let found = shell_found.clone().unwrap_or_else(|| {
            in_install_folders(&names, &crate::transcripts::home(), &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"])
        });
        let i = Installed {
            kinds: ALL.iter().copied().filter(|k| found.iter().any(|f| f == program(*k))).collect(),
            tmux: Path::new(tmux::tmux_bin()).is_absolute(),
            signed_in: Vec::new(),
            never_set_up: Vec::new(),
        };
        // A guess from the folders is kept for a minute, so starting an agent doesn't wait on the
        // slow shell again.
        let expires = shell_found.is_none().then(|| Instant::now() + Duration::from_secs(60));
        *cache = Some((i.clone(), expires));
        std::borrow::Cow::Owned(i)
    }
}

#[cfg(unix)]
static ON_PATH: std::sync::Mutex<Option<(Installed, Option<Instant>)>> = std::sync::Mutex::new(None);

/// Look for agents again on the next `on_path` (the app asks when it might have missed one: a
/// panel opened, or nothing to start a task with).
pub fn refresh() {
    #[cfg(unix)]
    {
        *ON_PATH.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// `names` found in the folders their installers use, for when the login shell is too slow to
/// say what's on PATH.
#[cfg_attr(windows, allow(dead_code))]
fn in_install_folders(names: &[&str], home: &Path, system: &[&str]) -> Vec<String> {
    let mut dirs: Vec<PathBuf> =
        [".local/bin", ".claude/local", ".npm-global/bin", ".bun/bin", ".volta/bin", ".cargo/bin", ".opencode/bin"]
            .iter()
            .map(|d| home.join(d))
            .collect();
    dirs.extend(system.iter().map(PathBuf::from));
    // nvm and fnm keep one bin folder per Node.js version.
    for root in [".nvm/versions/node", ".local/share/fnm/node-versions", "Library/Application Support/fnm/node-versions"] {
        if let Ok(entries) = std::fs::read_dir(home.join(root)) {
            for e in entries.flatten() {
                dirs.push(e.path().join("bin"));
                dirs.push(e.path().join("installation/bin"));
            }
        }
    }
    names.iter().filter(|n| dirs.iter().any(|d| d.join(n).is_file())).map(|n| n.to_string()).collect()
}

/// Which agents can be started: those on the user's PATH, and October when Lantern's own October
/// Harness is installed (see `harness`).
pub fn installed() -> Installed {
    let mut i = with_bundled(&on_path(), crate::harness::binary().is_some());
    i.signed_in = crate::subscriptions::signed_in().into_iter().filter(|k| i.kinds.contains(k)).collect();
    i.never_set_up = crate::subscriptions::never_set_up().into_iter().filter(|k| i.kinds.contains(k)).collect();
    i
}

pub(crate) fn with_bundled(on_path: &Installed, bundled_october: bool) -> Installed {
    let mut i = on_path.clone();
    if bundled_october && !i.kinds.contains(&Kind::October) {
        i.kinds.push(Kind::October);
    }
    i
}

/// What a launch adds besides the agent's own arguments: the program to run in place of the
/// agent's name (a full path), a folder to put first on PATH, and environment.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LaunchEnv {
    pub session: Option<String>,
    pub program: Option<String>,
    pub path_prefix: Option<String>,
    pub env: Vec<(String, String)>,
    /// Without the agent's auto-approve flags (`hands_free_args`).
    pub ask_first: bool,
    pub run_once: bool,
}

/// October: the user's own `october` when it's on PATH, else Lantern's October Harness with its
/// Node.js first on PATH. Either way it gets the October sign-in, when the app has one.
pub(crate) fn october_env(on_path: bool, bundled: Option<&crate::harness::Bundled>, session: Vec<(String, String)>) -> LaunchEnv {
    let bundled = bundled.filter(|_| !on_path);
    LaunchEnv {
        session: None,
        program: bundled.map(|b| b.bin.to_string_lossy().into_owned()),
        path_prefix: bundled.map(|b| b.node_bin.to_string_lossy().into_owned()),
        env: session,
        ask_first: false,
        run_once: false,
    }
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
    /// Its id for cards (see `cards`), when it got the card tools.
    pub launch: Option<String>,
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
        Kind::Claude | Kind::Codex | Kind::Grok | Kind::October | Kind::Pi | Kind::Gemini | Kind::Muse => Some(PromptArg::Positional),
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

/// What goes with the first message, besides your words.
#[derive(Default, Clone, Copy)]
pub struct Extras<'a> {
    /// A screenshot the app saved (in the screenshots folder).
    pub screenshot: Option<&'a Path>,
    /// Images you added to the task (pasted or dropped), saved by the app in the screenshots folder.
    pub images: &'a [PathBuf],
    /// What you're working in, from the app (a task: the app, its window, its document).
    pub context: Option<&'a str>,
    /// The toolkit list (toolkit.md).
    pub toolkit: Option<&'a str>,
    /// Connect the agent to October Bus with the other agents Lantern started.
    pub bus: bool,
    /// What the first message says about October Bus (set by `launch`).
    pub bus_note: Option<&'a str>,
    /// Let the agent use the computer's apps through Cua Driver (see `computer`).
    pub computer: bool,
    /// What the first message says about Cua Driver (set by `launch`).
    pub computer_note: Option<&'a str>,
    /// What the first message says about Lantern's card tools (set by `launch`).
    pub cards_note: Option<&'a str>,
    pub operation_id: Option<&'a str>,
    /// The agent asks before acting, as it does on its own (see `hands_free_args`).
    pub ask_first: bool,
    pub run_once: bool,
}

/// How Lantern asks every agent it starts to answer, after the first message: where it is, and
/// to give full paths (the person reads the answer in Lantern and opens files from it).
pub(crate) fn with_guidance(message: String, cwd: &Path) -> String {
    guidance_for(std::env::consts::OS, message, cwd)
}

/// The note added to every first message, for the computer the agent runs on. It always starts
/// with "\n\n(Started from October Lantern", which the apps hide in the conversation.
pub(crate) fn guidance_for(os: &str, message: String, cwd: &Path) -> String {
    let (system, example, search, admin) = match os {
        "windows" => (
            "Windows",
            r"C:\Users\name\Documents\file.pdf",
            "the computer (for example with Get-ChildItem -Recurse under the user folder)",
            "PowerShell's Start-Process with -Verb RunAs (Windows asks them to allow it)",
        ),
        "linux" => (
            "Linux",
            "/home/name/Documents/file.pdf",
            "the computer (for example with locate, or find under the home folder)",
            "pkexec (the system asks for their password)",
        ),
        _ => (
            "macOS",
            "/Users/name/Documents/file.pdf",
            "the Mac (for example with mdfind or find under the home folder)",
            "osascript's do shell script ... with administrator privileges (macOS asks for their password)",
        ),
    };
    format!(
        "{message}\n\n(Started from October Lantern on {system}. You're in {}. When you mention a file or folder, give its full \
         absolute path (like {example}), not a relative one, so it can be opened directly. If the task is about something outside \
         this folder, look for it across {search}. You don't run as administrator: when a step needs it (installing or removing \
         a program, a system setting), run just that step with {admin}, and tell them to look for that prompt. End with a short \
         summary of what you did or found.)",
        cwd.display()
    )
}

/// The first message: your words, then the context, the screenshot's and images' paths and the
/// toolkit list. Nothing is sent when there are no words and nothing to look at (an agent would
/// act on context alone).
pub fn first_message(prompt: Option<&str>, extras: Extras) -> Option<String> {
    let mut out = match (prompt, extras.screenshot, extras.images.is_empty()) {
        (Some(p), _, _) => p.to_string(),
        (None, Some(_), _) => "Take a look at the screenshot below, then wait for my instructions.".into(),
        (None, None, false) => "Take a look at the images below, then wait for my instructions.".into(),
        (None, None, true) => return None,
    };
    if let Some(c) = extras.context.map(str::trim).filter(|c| !c.is_empty()) {
        out.push_str(&format!("\n\n{c}"));
    }
    if let Some(s) = extras.screenshot {
        out.push_str(&format!("\n\nFor context, a screenshot of my screen when I started this session: {}", s.display()));
    }
    match extras.images {
        [] => {}
        [one] => out.push_str(&format!("\n\nAn image I added (open it to see it): {}", one.display())),
        many => {
            out.push_str("\n\nImages I added (open them to see them):");
            for image in many {
                out.push_str(&format!("\n- {}", image.display()));
            }
        }
    }
    if let Some(n) = extras.bus_note {
        out.push_str(&format!("\n\n{n}"));
    }
    if let Some(n) = extras.computer_note {
        out.push_str(&format!("\n\n{n}"));
    }
    if let Some(n) = extras.cards_note {
        out.push_str(&format!("\n\n{n}"));
    }
    if let Some(t) = extras.toolkit.map(str::trim).filter(|t| !t.is_empty()) {
        out.push_str(&format!(
            "\n\nWhat this computer already has (from Lantern's toolkit list; prefer these over installing new tools):\n{t}"
        ));
    }
    Some(out)
}

#[cfg(test)]
pub(crate) fn agent_command(
    kind: Kind,
    cwd: &Path,
    prompt: Option<&str>,
    screenshot: Option<&Path>,
    model: Option<&str>,
    bus: Option<&crate::bus::Attach>,
) -> String {
    agent_command_with(kind, cwd, prompt, screenshot.as_slice(), model, bus, &LaunchEnv::default())
}

/// Lets the agent see the screenshot and images, which are outside the project: Claude Code and
/// Gemini CLI may read their folders without asking, Codex gets each one attached.
fn image_args(kind: Kind, files: &[&Path]) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    let mut args = Vec::new();
    for file in files {
        let dir = file.parent().unwrap_or(file).to_string_lossy().into_owned();
        match kind {
            Kind::Claude | Kind::Gemini if !dirs.contains(&dir) => dirs.push(dir),
            Kind::Codex => args.extend(["--image".to_string(), file.to_string_lossy().into_owned()]),
            _ => {}
        }
    }
    let flag = if kind == Kind::Claude { "--add-dir" } else { "--include-directories" };
    for dir in dirs {
        args.extend([flag.to_string(), dir]);
    }
    args
}

// tmux launches (Unix) and tests; Windows starts Lantern's own sessions with `agent_argv`.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn agent_command_with(
    kind: Kind,
    cwd: &Path,
    prompt: Option<&str>,
    images: &[&Path],
    model: Option<&str>,
    bus: Option<&crate::bus::Attach>,
    extra: &LaunchEnv,
) -> String {
    // The launch's own PATH folder and environment, then October Bus's: its environment, a
    // wrapper command (the October harness), or arguments.
    let mut env: String = extra.path_prefix.as_ref().map(|p| format!("PATH={}:\"$PATH\" ", quote(p))).unwrap_or_default();
    for (k, v) in extra.env.iter().chain(bus.map(|b| b.env.as_slice()).unwrap_or_default()) {
        env.push_str(&format!("{k}={} ", quote(v)));
    }
    for (k, v) in hands_free_env(kind, extra.ask_first) {
        env.push_str(&format!("{k}={} ", quote(v)));
    }
    let wrap: String = bus.map(|b| b.wrap.iter().map(|w| format!("{w} ")).collect()).unwrap_or_default();
    let program = extra.program.as_deref().map(quote).unwrap_or_else(|| program(kind).to_string());
    let mut cmd = format!("cd {} && {env}exec {wrap}{program}", quote(&cwd.to_string_lossy()));
    for arg in bus.map(|b| b.args.as_slice()).unwrap_or_default() {
        cmd.push(' ');
        cmd.push_str(arg);
    }
    for arg in once_args(kind, extra.run_once) {
        cmd.push(' ');
        cmd.push_str(&quote(arg));
    }
    for arg in hands_free_args(kind, extra.ask_first) {
        cmd.push(' ');
        cmd.push_str(&quote(arg));
    }
    if let (Some(m), Some(flag)) = (model, crate::models::flag(kind)) {
        cmd.push_str(&format!(" {flag} {}", quote(m)));
    }
    for arg in image_args(kind, images) {
        cmd.push(' ');
        cmd.push_str(&quote(&arg));
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
    cmd
}

/// Keep app credentials and the first message out of shell/tmux arguments (and tmux's retained
/// pane_start_command). After login setup, a noninteractive POSIX shell reads the private file
/// and removes it before exec; interactive shell history never reads its credential lines.
#[cfg(unix)]
struct LaunchScript(Option<PathBuf>);

#[cfg(unix)]
impl LaunchScript {
    fn new(command: &str) -> Result<Self> {
        Self::in_dir(&support_dir().join("launch"), command)
    }

    fn in_dir(dir: &Path, command: &str) -> Result<Self> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        crate::bus::create_private_dir(dir)?;
        let path = dir.join(format!("agent-{}.sh", uuid::Uuid::new_v4().simple()));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
        let script = Self(Some(path.clone()));
        writeln!(file, "/bin/rm -f -- {} || exit 1\n{command}", quote(&path.to_string_lossy()))?;
        Ok(script)
    }

    fn command(&self) -> String {
        format!("exec {} -lic {}", quote(&shell()), quote(&self.reader_command()))
    }

    fn reader_command(&self) -> String {
        format!("exec /bin/sh {}", quote(&self.0.as_ref().unwrap().to_string_lossy()))
    }

    /// The terminal now owns consumption; dropping a failed launch still removes its secret file.
    fn handed_off(mut self) {
        self.0 = None;
    }
}

#[cfg(unix)]
impl Drop for LaunchScript {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Opens a new Terminal window running `script` (through a `.command` file, which needs no
/// Automation permission).
#[cfg(target_os = "macos")]
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

#[cfg(target_os = "linux")]
fn open_in_terminal(_name: &str, script: &str) -> Result<()> {
    crate::platform::open_terminal(script)
}

/// tmux sessions don't exist on Windows; Lantern's own sessions are opened by `pty::open_window`.
#[cfg(windows)]
fn open_in_terminal(_name: &str, _script: &str) -> Result<()> {
    bail!("tmux sessions aren't available on Windows")
}

/// The program and arguments that start `kind` on Windows, run without a shell. An npm `.cmd`
/// shim is unwrapped to Node and its script, so `cmd.exe` and its quoting never see the message.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn agent_argv(
    kind: Kind,
    prompt: Option<&str>,
    images: &[&Path],
    model: Option<&str>,
    bus: Option<&crate::bus::Attach>,
    program_override: Option<&Path>,
    // Where a launch's own Node.js is (October Harness's), for npm shims that need it.
    node_dir: Option<&Path>,
    ask_first: bool,
    run_once: bool,
) -> Result<(PathBuf, Vec<String>)> {
    let name = program(kind);
    let found = match program_override {
        Some(p) => p.to_path_buf(),
        None => crate::platform::find_on_path(name)
            .with_context(|| format!("Lantern can't find {} on this computer. Install it, or pick another agent.", agent_name(kind)))?,
    };
    let ext = found.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let executable = found.clone();
    let (program, mut args) = match ext.as_str() {
        "exe" | "com" => (found, Vec::new()),
        "cmd" | "bat" => {
            let shim = std::fs::read_to_string(&found).unwrap_or_default();
            let script = crate::pty::shim_script(&shim)
                .map(|rel| found.parent().unwrap_or(Path::new(".")).join(rel))
                .filter(|p| p.is_file())
                .with_context(|| {
                    format!(
                        "Lantern can't start {} the way it's installed ({}). Install it with its own installer, or pick another agent.",
                        agent_name(kind),
                        found.display()
                    )
                })?;
            let beside = found.with_file_name("node.exe");
            let own = node_dir.map(|dir| dir.join("node.exe")).filter(|node| node.is_file());
            let node = if beside.is_file() {
                beside
            } else if let Some(own) = own {
                own
            } else {
                crate::platform::find_on_path("node").with_context(|| {
                    format!("{} needs Node.js, which isn't installed. Install it from nodejs.org, or pick another agent.", agent_name(kind))
                })?
            };
            (node, vec![script.to_string_lossy().into_owned()])
        }
        _ => bail!("Lantern can't start {} from {}.", agent_name(kind), found.display()),
    };
    // Asked before the Bus's arguments are added: whether Codex takes it depends only on Codex.
    let no_daemon = !run_once && kind == Kind::Codex && codex_has_no_daemon(&executable, &program, &args);
    if let Some(b) = bus {
        args.extend(b.args.iter().cloned());
    }
    if no_daemon {
        args.push("--no-daemon".into());
    }
    args.extend(once_args(kind, run_once).iter().map(|a| a.to_string()));
    args.extend(hands_free_args(kind, ask_first).iter().map(|a| a.to_string()));
    if let (Some(m), Some(flag)) = (model, crate::models::flag(kind)) {
        args.extend([flag.to_string(), m.to_string()]);
    }
    args.extend(image_args(kind, images));
    if kind == Kind::Claude {
        args.extend(["--session-id".into(), uuid::Uuid::new_v4().hyphenated().to_string()]);
    }
    match (prompt, prompt_arg(kind)) {
        (Some(p), Some(PromptArg::Positional)) => args.extend(["--".into(), p.to_string()]),
        (Some(p), Some(PromptArg::Option(flag))) => args.extend([flag.to_string(), p.to_string()]),
        _ => {}
    }
    // On October Bus, some agents run under a wrapper (the October harness under `october-bus
    // agent run ... --`).
    match bus.map(|b| b.wrap.as_slice()) {
        Some([wrapper, rest @ ..]) => {
            let mut wrapped: Vec<String> = rest.to_vec();
            wrapped.push(program.to_string_lossy().into_owned());
            wrapped.extend(args);
            Ok((PathBuf::from(wrapper), wrapped))
        }
        _ => Ok((program, args)),
    }
}

/// Routines use one model run, then exit; a TUI left waiting would block every later occurrence.
fn once_args(kind: Kind, once: bool) -> &'static [&'static str] {
    if !once {
        return &[];
    }
    match kind {
        Kind::October => &["--print"],
        Kind::Claude => &["--print", "--allowedTools", "mcp__lantern__*"],
        Kind::Codex => &["exec", "--skip-git-repo-check", "--sandbox", "workspace-write"],
        _ => &[],
    }
}

/// Agents Lantern starts work without stopping to ask: Lantern's tasks often run in the
/// background, where nobody sees a question (one waited half an hour on "Do you trust this
/// folder?"). Each agent's own auto-approve mode: Claude Code bypasses its permission checks,
/// without the mode's one-time warning; its folder is trusted by `trust_claude_folder`. Codex
/// skips its folder question when it's given an approval mode. Pi, OpenCode and the October
/// harness don't ask. `ask_first` (Settings: "Ask before agents act") leaves them asking.
pub(crate) fn hands_free_args(kind: Kind, ask_first: bool) -> &'static [&'static str] {
    if ask_first {
        return &[];
    }
    match kind {
        Kind::Claude => &["--dangerously-skip-permissions", "--settings", r#"{"skipDangerousModePermissionPrompt":true}"#],
        Kind::Codex => &["--dangerously-bypass-approvals-and-sandbox"],
        Kind::Gemini | Kind::Qwen => &["--yolo"],
        Kind::Copilot => &["--allow-all-tools", "--allow-all-paths"],
        Kind::Cursor => &["--force"],
        Kind::Grok => &["--always-approve"],
        _ => &[],
    }
}

/// The environment that does the same, for agents set that way.
pub(crate) fn hands_free_env(kind: Kind, ask_first: bool) -> &'static [(&'static str, &'static str)] {
    if ask_first {
        return &[];
    }
    match kind {
        Kind::Goose => &[("GOOSE_MODE", "auto")],
        _ => &[],
    }
}

/// Marks `folder` trusted for Claude Code, as answering "Yes" to its "Do you trust the files in
/// this folder?" does: `projects[<folder>].hasTrustDialogAccepted` in `~/.claude.json` (its
/// key: the folder's path, with `/` on Windows too). Only when Claude Code has its file already,
/// and only that folder's entry changes.
pub(crate) fn trust_claude_folder(folder: &Path) -> Result<()> {
    trust_claude_folder_in(&crate::platform::home().join(".claude.json"), folder)
}

pub(crate) fn trust_claude_folder_in(file: &Path, folder: &Path) -> Result<()> {
    let Ok(text) = std::fs::read_to_string(file) else { return Ok(()) };
    let mut config: serde_json::Value = serde_json::from_str(&text).context("reading ~/.claude.json")?;
    let key = folder.to_string_lossy().replace('\\', "/");
    let projects =
        config.as_object_mut().context("~/.claude.json isn't an object")?.entry("projects").or_insert_with(|| serde_json::json!({}));
    let projects = projects.as_object_mut().context("~/.claude.json's projects isn't an object")?;
    if projects.get(&key).is_some_and(|p| p["hasTrustDialogAccepted"] == true) {
        return Ok(());
    }
    // A new entry has what Claude Code gives one.
    let entry = projects.entry(key).or_insert_with(|| {
        serde_json::json!({"allowedTools": [], "mcpContextUris": [], "mcpServers": {}, "enabledMcpjsonServers": [],
            "disabledMcpjsonServers": [], "hasClaudeMdExternalIncludesApproved": false,
            "hasClaudeMdExternalIncludesWarningShown": false})
    });
    entry.as_object_mut().context("a project entry isn't an object")?.insert("hasTrustDialogAccepted".into(), true.into());
    let tmp = file.with_extension(format!("json.lantern-{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(&config)?)?;
    std::fs::rename(&tmp, file).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// Codex runs its TUI through a shared background app-server ("daemon") by default since 0.160.
/// On Windows that daemon's own install can be missing ("daemon executable not found at
/// ~\.codex\packages\app-server-daemon\current\bin\codex.exe"), and then every session exits at
/// once. So Lantern's Codex sessions there run on their own (`--no-daemon`) when that Codex has
/// the flag. Its `--help` is read once per Codex executable (a first run through an npm shim and
/// Node can be slow, hence the generous wait); when it can't be read, `--version` decides. The
/// engine starts the check as soon as it finds Codex on PATH ([`on_path`]), so a launch rarely
/// waits for it; one that does waits for the check already running.
#[cfg(windows)]
fn codex_has_no_daemon(found: &Path, program: &Path, args: &[String]) -> bool {
    type Checks = std::collections::HashMap<PathBuf, std::sync::Arc<std::sync::OnceLock<bool>>>;
    static KNOWN: std::sync::Mutex<Option<Checks>> = std::sync::Mutex::new(None);
    let check = KNOWN.lock().unwrap().get_or_insert_with(Default::default).entry(found.to_path_buf()).or_default().clone();
    *check.get_or_init(|| {
        let run = |arg: &str, limit: u64| {
            crate::run::output(std::process::Command::new(program).args(args).arg(arg), Duration::from_secs(limit))
                .ok()
                .filter(|out| out.status.success())
                .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        };
        match run("--help", 20) {
            Some(help) if help.contains("Usage") || help.contains("Options") => help_lists_no_daemon(&help),
            _ => run("--version", 10).is_some_and(|version| version_has_no_daemon(&version)),
        }
    })
}

/// Checks the Codex on PATH for `--no-daemon` in the background, ahead of the first launch.
#[cfg(windows)]
fn check_codex_early() {
    // Working out Codex's arguments asks it, and remembers the answer.
    std::thread::spawn(|| agent_argv(Kind::Codex, None, &[], None, None, None, None, false, false).map(drop));
}

/// Whether `codex --help` offers `--no-daemon` (as an option, not just in prose).
#[cfg(any(windows, test))]
pub(crate) fn help_lists_no_daemon(help: &str) -> bool {
    help.lines().any(|line| line.trim_start().split([' ', ',', '\t']).next() == Some("--no-daemon"))
}

/// The first Codex with the shared daemon, and so with `--no-daemon`.
#[cfg(any(windows, test))]
const CODEX_DAEMON_SINCE: (u64, u64, u64) = (0, 160, 0);

/// Whether `codex --version` ("codex-cli 0.160.0") names a Codex with `--no-daemon`.
#[cfg(any(windows, test))]
pub(crate) fn version_has_no_daemon(version: &str) -> bool {
    codex_version(version).is_some_and(|v| v >= CODEX_DAEMON_SINCE)
}

/// The first `major.minor.patch` in `codex --version`'s output (a `v` prefix and a pre-release or
/// build suffix are allowed).
#[cfg(any(windows, test))]
pub(crate) fn codex_version(text: &str) -> Option<(u64, u64, u64)> {
    text.split_whitespace().find_map(|word| {
        let core = word.trim_start_matches(['v', 'V']).split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
        let version = (parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(version)
    })
}

/// One of Lantern's own sessions (pty.rs), shown in a terminal window or left in the background.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn launch_session(
    kind: Kind,
    cwd: &Path,
    prompt: Option<&str>,
    images: &[&Path],
    model: Option<&str>,
    bus: Option<crate::bus::Attach>,
    bus_problem: Option<String>,
    extra: &LaunchEnv,
    mode: Mode,
    deadline: Instant,
) -> Result<Launched> {
    let program_override = extra.program.as_deref().map(Path::new);
    let node_dir = extra.path_prefix.as_deref().map(Path::new);
    let (program, args) =
        agent_argv(kind, prompt, images, model, bus.as_ref(), program_override, node_dir, extra.ask_first, extra.run_once)?;
    let project = cwd.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let safe: String = project.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).take(24).collect();
    let session = extra.session.clone().unwrap_or_else(|| format!("{}-{}-{}", kind.as_str(), safe, now_ms() % 100_000));
    check_launch_deadline(deadline)?;
    // The launch's own PATH folder and environment, then October Bus's (as `agent_command_with`).
    let mut env: Vec<(String, String)> = Vec::new();
    if let Some(prefix) = &extra.path_prefix {
        env.push(("PATH".into(), format!("{prefix};{}", std::env::var("PATH").unwrap_or_default())));
    }
    env.extend(extra.env.iter().cloned());
    env.extend(hands_free_env(kind, extra.ask_first).iter().map(|(k, v)| (k.to_string(), v.to_string())));
    env.extend(bus.as_ref().map(|b| b.env.clone()).unwrap_or_default());
    let spec = crate::pty::Spec { cwd: cwd.to_path_buf(), program, args, env };
    crate::pty::start(&session, &spec)?;
    if let Some(b) = bus.as_ref().filter(|b| b.on_bus) {
        crate::bus::link_when_ready(b.id.clone());
    }
    if mode == Mode::Terminal {
        crate::pty::open_window(&session)?;
    }
    if let Some(p) = prompt.filter(|_| !takes_prompt_arg(kind)) {
        type_first_prompt_in_session(&session, kind, p).context("the session started, but its first message wasn't sent")?;
    }
    Ok(Launched { session: Some(session), bus_problem, launch: None })
}

/// `type_first_prompt` for a Lantern session: once the agent runs in it and its screen settles.
#[cfg(windows)]
fn type_first_prompt_in_session(session: &str, kind: Kind, text: &str) -> Result<()> {
    use crate::model::Route;
    let deadline = Instant::now() + Duration::from_secs(30);
    let (mut last, mut steady, mut up_since) = (String::new(), 0, None::<Instant>);
    let mut processes = sysinfo::System::new();
    let mut cache = crate::procs::ProcCache::default();
    loop {
        if Instant::now() >= deadline {
            bail!("{} didn't finish starting within 30 s; type your message in its window", program(kind));
        }
        std::thread::sleep(Duration::from_millis(300));
        let info = crate::pty::info(session)?;
        if !info.alive {
            bail!("{} exited", program(kind));
        }
        let table = crate::procs::ProcTable::capture(&mut processes, &mut cache);
        let route = Route::Pty { session: session.into() };
        let agent = table
            .procs
            .values()
            .filter(|p| p.pid == info.child_pid || table.ancestors(p.pid).iter().any(|a| a.pid == info.child_pid))
            .find_map(|p| {
                let live = crate::procs::live(p.pid)?;
                let agent = startup_agent(kind, p, &live, route.clone(), None)?;
                let ancestors: Vec<u32> = table.ancestors(p.pid).iter().map(|a| a.pid).collect();
                crate::deliver::check_session(&agent, &live, &info, &ancestors).ok().map(|_| agent)
            });
        let Some(agent) = agent else {
            steady = 0;
            up_since = None;
            continue;
        };
        let up = *up_since.get_or_insert_with(Instant::now);
        let screen = crate::pty::capture(session)?;
        if !screen.trim().is_empty() && screen == last {
            steady += 1;
        } else {
            steady = 0;
            last = screen;
        }
        if steady >= 3 || up.elapsed() >= Duration::from_secs(5) {
            return crate::deliver::send_text(&agent, text);
        }
    }
}

pub fn launch(
    kind: Kind,
    cwd: &Path,
    prompt: Option<&str>,
    extras: Extras,
    model: Option<&str>,
    mode: Mode,
    deadline: Instant,
) -> Result<Launched> {
    check_launch_deadline(deadline)?;
    if crate::computer::held() {
        bail!("App access is paused. Continue before starting another agent.");
    }
    let control_epoch = crate::computer::control_epoch();
    let screenshot = extras.screenshot;
    if !cwd.is_dir() {
        bail!("{} is not a folder", cwd.display());
    }
    let mut extra = if kind == Kind::October {
        let on_path = on_path().kinds.contains(&Kind::October);
        // Install Lantern's October Harness if needed, leaving a few seconds of the deadline for
        // starting the session itself (the app normally prepares it ahead: `harness.prepare`).
        let until = deadline.checked_sub(Duration::from_secs(8)).unwrap_or(deadline);
        let bundled = if on_path { None } else { Some(crate::harness::wait_installed(until)?) };
        october_env(on_path, bundled.as_ref(), crate::harness::session_env())
    } else {
        if !installed().kinds.contains(&kind) {
            bail!("{} isn't installed on this computer. Install it, or pick another agent.", agent_name(kind));
        }
        LaunchEnv::default()
    };
    // tmux may predate this debug engine, so it does not inherit its private profile. Keep the
    // fixture profile scoped to this launch and its MCP children, never the shared server.
    #[cfg(debug_assertions)]
    if let Some(profile) = std::env::var_os("LANTERN_TEST_PROFILE").filter(|p| Path::new(p).is_absolute()) {
        extra.env.push(("LANTERN_TEST_PROFILE".into(), profile.to_string_lossy().into_owned()));
    }
    // Only files the app saved in the screenshots folder, so a request can't hand an agent anything else.
    let images: Vec<&Path> = screenshot.into_iter().chain(extras.images.iter().map(PathBuf::as_path)).collect();
    for s in &images {
        let inside = s.canonicalize().ok().zip(screenshots_dir().canonicalize().ok()).is_some_and(|(s, dir)| s.starts_with(dir));
        if !inside || !s.is_file() {
            bail!("an image for this task wasn't saved; try again");
        }
    }
    extra.ask_first = extras.ask_first;
    extra.run_once = extras.run_once;
    if extras.run_once && !crate::cards::supported(kind) {
        bail!("This harness cannot run a local routine");
    }
    if kind == Kind::Claude
        && !extras.ask_first
        && let Err(error) = trust_claude_folder(cwd)
    {
        eprintln!("lantern-engine: couldn't mark {} trusted for Claude Code: {error:#}", cwd.display());
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
    // October Bus: a new identity for this agent, and the tools to reach the others. Cua Driver:
    // the computer's apps, through Lantern's own driver (started when the app turned it on).
    let mut bus_problem = None;
    let computer = extras.computer && crate::computer::ready();
    let server = if computer { crate::computer::mcp_server() } else { None };
    let on_bus = extras.bus
        && crate::bus::supported(kind)
        && match crate::bus::ensure_ready() {
            Ok(()) => true,
            Err(e) => {
                bus_problem = Some(format!("{e:#}"));
                false
            }
        };
    // Lantern's card tools: registered before the agent starts, whatever the Bus and Cua switches.
    let project = cwd.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let title =
        prompt.map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).unwrap_or_else(|| format!("{} in {project}", agent_name(kind)));
    let cards = crate::cards::register(kind, cwd, &title, extras.operation_id);
    if crate::cards::supported(kind) && cards.is_none() {
        bail!("Task cards could not be registered. Restart Lantern before starting another task.");
    }
    if let Some(c) = &cards {
        let session = format!("{}-{}", kind.as_str(), c.id);
        if let Some(store) = crate::cards::service() {
            store.started(&c.id, Some(&session))?;
        }
        extra.session = Some(session);
    }
    let bus = if on_bus || (server.is_some() && crate::bus::takes_mcp(kind)) || cards.is_some() {
        let name = format!("{} · {project}", display_name(kind));
        let id = crate::bus::new_id(kind);
        let card_server = cards.as_ref().map(|c| &c.server);
        match crate::bus::attach(kind, &id, &name, on_bus, server.as_ref(), card_server) {
            Ok(a) => Some(a),
            Err(e) => {
                bus_problem = Some(format!("{e:#}"));
                // Without the Bus, the agent still gets Cua Driver and the card tools.
                crate::bus::attach(kind, &id, &name, false, server.as_ref(), card_server).ok()
            }
        }
    } else {
        None
    };
    let cards = match cards {
        Some(c) if bus.as_ref().is_some_and(|b| b.cards) => Some(c),
        Some(c) => {
            crate::cards::forget(&c.id);
            None
        }
        None => None,
    };
    if crate::cards::supported(kind) && cards.is_none() {
        bail!("The card tools could not be attached; the agent was not started.");
    }
    let note = bus.as_ref().filter(|b| b.on_bus).map(|b| crate::bus::note(&b.name));
    let computer_note = if computer { crate::computer::note(bus.as_ref().is_some_and(|b| b.computer)) } else { None };
    let extras = Extras {
        bus_note: note.as_deref(),
        computer_note: computer_note.as_deref(),
        cards_note: cards.as_ref().map(|_| crate::cards::note()),
        ..extras
    };
    let message = first_message(prompt.map(str::trim).filter(|p| !p.is_empty()), extras).map(|m| with_guidance(m, cwd));
    let prompt = message.as_deref();

    let _dispatch_guard = crate::computer::action_guard();
    if crate::computer::held() || control_epoch != crate::computer::control_epoch() {
        if let Some(c) = &cards {
            crate::cards::forget(&c.id);
        }
        bail!("The start was cancelled by a control change; it was not queued for Continue.");
    }
    #[cfg(unix)]
    let launched = launch_tmux(kind, cwd, prompt, &images, model, bus, bus_problem, &extra, mode, deadline);
    #[cfg(windows)]
    let launched = launch_session(kind, cwd, prompt, &images, model, bus, bus_problem, &extra, mode, deadline);
    crate::cards::launched(cards, launched)
}

/// A tmux session on Lantern's own server, or (without tmux) the agent in a new terminal window.
#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn launch_tmux(
    kind: Kind,
    cwd: &Path,
    prompt: Option<&str>,
    images: &[&Path],
    model: Option<&str>,
    bus: Option<crate::bus::Attach>,
    bus_problem: Option<String>,
    extra: &LaunchEnv,
    mode: Mode,
    deadline: Instant,
) -> Result<Launched> {
    if !installed().tmux {
        if mode == Mode::Background {
            bail!("Running agents in the background needs tmux. Install it with your system's package manager, then try again.");
        }
        if prompt.is_some() && !takes_prompt_arg(kind) {
            bail!("Without tmux, Lantern can't give {} its first message. Leave the message empty, or install tmux.", agent_name(kind));
        }
        let name = format!("{}-{}", kind.as_str(), now_ms());
        check_launch_deadline(deadline)?;
        let script = LaunchScript::new(&agent_command_with(kind, cwd, prompt, images, model, bus.as_ref(), extra))?;
        open_in_terminal(&name, &script.command())?;
        script.handed_off();
        if let Some(b) = bus.as_ref().filter(|b| b.on_bus) {
            crate::bus::link_when_ready(b.id.clone());
        }
        return Ok(Launched { session: None, bus_problem, launch: None });
    }

    let project = cwd.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let safe: String = project.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).take(24).collect();
    let session = extra.session.clone().unwrap_or_else(|| format!("{}-{}-{}", kind.as_str(), safe, now_ms() % 100_000));
    let script = LaunchScript::new(&agent_command_with(kind, cwd, prompt, images, model, bus.as_ref(), extra))?;
    let status = &mut tmux::command();
    let status = status
        .args(["-L", LANTERN_SOCKET, "new-session", "-d", "-s", &session, "-x", "200", "-y", "50", "-c"])
        .arg(cwd)
        .arg(script.command());
    check_launch_deadline(deadline)?;
    let out = crate::run::output(status, Duration::from_secs(10)).context("starting tmux")?;
    if !out.status.success() {
        bail!("tmux couldn't start the session");
    }
    script.handed_off();

    if let Some(b) = bus.as_ref().filter(|b| b.on_bus) {
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
    Ok(Launched { session: Some(session), bus_problem, launch: None })
}

pub(crate) fn check_launch_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        bail!("Preparing the session took too long. No session was started; try again.");
    }
    Ok(())
}

/// Build a delivery target only from the expected program's current process identity. This
/// intentionally does not scan session files: startup readiness needs no transcript/history.
#[cfg_attr(windows, allow(dead_code))]
fn startup_owner(kind: Kind, p: &crate::procs::Proc, live: &crate::procs::Live, pane: &TmuxPane) -> Option<crate::model::Agent> {
    let agent = startup_agent(kind, p, live, crate::model::Route::Tmux, Some(pane.clone()))?;
    crate::deliver::check(&agent, live).ok().map(|_| agent)
}

/// The agent to type a first message into: the expected program, as it runs right now.
fn startup_agent(
    kind: Kind,
    p: &crate::procs::Proc,
    live: &crate::procs::Live,
    route: crate::model::Route,
    tmux: Option<TmuxPane>,
) -> Option<crate::model::Agent> {
    use crate::model::{Agent, SessionMatch, State, StateSource};
    if crate::scanner::classify(p) != Some(kind) || p.exe.as_ref().and_then(|e| e.to_str()) != live.exe.as_deref() {
        return None;
    }
    Some(Agent {
        id: format!("{}:{}:{}", kind.as_str(), p.pid, p.start_time),
        kind,
        handle: kind.as_str().into(),
        pid: p.pid,
        start_time: p.start_time,
        exe: p.exe.as_ref().map(|e| e.to_string_lossy().into_owned()),
        comm: p.name.clone(),
        tty: p.tty.clone(),
        cwd: None,
        project: None,
        title: None,
        session_id: None,
        state: State::Unknown,
        state_since: None,
        last_message: None,
        question: None,
        question_kind: None,
        question_detail: None,
        prompt_id: None,
        choices: None,
        transcript: None,
        bus_id: None,
        session_match: SessionMatch::None,
        source: None,
        live: true,
        host: None,
        tmux,
        can_reply: true,
        route,
        state_source: StateSource::None,
    })
}

/// Waits for the expected interactive harness in this pane's foreground and a settled screen.
/// Delivery rechecks the process identity and pane immediately before typing.
#[cfg_attr(windows, allow(dead_code))]
fn type_first_prompt(pane: &TmuxPane, kind: Kind, text: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let (mut last, mut steady, mut up_since) = (String::new(), 0, None::<Instant>);
    let mut processes = sysinfo::System::new();
    let mut cache = crate::procs::ProcCache::default();
    let mut owner = String::new();
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
        // Use the same classifier and foreground/identity checks as ordinary replies. A
        // background runtime or shell startup helper is not proof that the agent is ready.
        let table = crate::procs::ProcTable::capture(&mut processes, &mut cache);
        let agent = table
            .procs
            .values()
            .filter(|p| {
                p.tty.as_deref() == Some(tty.trim_start_matches("/dev/"))
                    && (p.pid == pid || table.ancestors(p.pid).iter().any(|parent| parent.pid == pid))
            })
            .find_map(|p| startup_owner(kind, p, &crate::procs::live(p.pid)?, pane));
        let Some(agent) = agent else {
            steady = 0;
            up_since = None;
            continue;
        };
        if owner != agent.id {
            owner = agent.id.clone();
            steady = 0;
            last.clear();
            up_since = None;
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
            if Instant::now() >= deadline {
                bail!("{} didn't finish starting within 30 s; type your message in its window", program(kind));
            }
            return crate::deliver::send_text(&agent, text);
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

#[cfg(test)]
mod startup_tests {
    use super::*;
    #[test]
    fn first_prompt_requires_the_expected_foreground_program() {
        let pane = TmuxPane { socket: None, target: "session:0.0".into(), pane_id: "%1".into() };
        let mut p = crate::procs::Proc {
            pid: 12,
            ppid: Some(1),
            name: "node".into(),
            exe: Some("/usr/bin/node".into()),
            cmd: vec!["node".into(), "/usr/bin/copilot".into()],
            cwd: None,
            start_time: 1,
            tty: Some("ttys001".into()),
        };
        let mut live = crate::procs::Live {
            pgid: 12,
            tpgid: 12,
            tty: p.tty.clone(),
            start: 1,
            exe: Some("/usr/bin/node".into()),
            comm: "node".into(),
        };
        assert!(startup_owner(Kind::Copilot, &p, &live, &pane).is_some());
        live.tpgid = 99;
        assert!(startup_owner(Kind::Copilot, &p, &live, &pane).is_none(), "A background runtime is not ready");
        live.tpgid = 12;
        p.cmd[1] = "/project/startup-helper.js".into();
        assert!(startup_owner(Kind::Copilot, &p, &live, &pane).is_none(), "An arbitrary runtime is not the harness");
        p.cmd[1] = "/usr/bin/copilot".into();
        live.exe = Some("/bin/zsh".into());
        assert!(startup_owner(Kind::Copilot, &p, &live, &pane).is_none(), "An exec between discovery and verification must fail");
    }
}

#[cfg(test)]
mod hands_free_tests {
    use super::*;

    #[test]
    fn agents_start_without_stopping_to_ask() {
        let claude = agent_command(Kind::Claude, Path::new("/tmp/p"), Some("hi"), None, None, None);
        assert!(claude.contains("--dangerously-skip-permissions") && claude.contains("skipDangerousModePermissionPrompt"), "{claude}");
        let codex = agent_command(Kind::Codex, Path::new("/tmp/p"), Some("hi"), None, None, None);
        assert!(codex.contains("--dangerously-bypass-approvals-and-sandbox"), "{codex}");
        assert!(agent_command(Kind::Goose, Path::new("/tmp/p"), None, None, None, None).contains("GOOSE_MODE="));
        assert!(hands_free_args(Kind::Pi, false).is_empty());
        // "Ask before agents act": no auto-approve flags at all.
        let asking = LaunchEnv { ask_first: true, ..Default::default() };
        let claude = agent_command_with(Kind::Claude, Path::new("/tmp/p"), None, &[], None, None, &asking);
        assert!(!claude.contains("--dangerously-skip-permissions"), "{claude}");
        assert!(hands_free_env(Kind::Goose, true).is_empty());
    }

    #[test]
    fn claude_trusts_only_the_launch_folder_and_keeps_the_rest_of_its_file() {
        let dir = std::env::temp_dir().join(format!("lantern-trust-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(".claude.json");
        std::fs::write(&file, r#"{"numStartups":3,"projects":{"/other":{"hasTrustDialogAccepted":false,"allowedTools":["x"]}}}"#).unwrap();
        trust_claude_folder_in(&file, Path::new(r"C:\Users\Some One\Desktop")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["numStartups"], 3);
        assert_eq!(v["projects"]["/other"]["hasTrustDialogAccepted"], false, "other folders stay as they were");
        assert_eq!(
            v["projects"]["C:/Users/Some One/Desktop"]["hasTrustDialogAccepted"], true,
            "Windows paths use /, as Claude Code's keys do"
        );
        assert_eq!(v["projects"]["C:/Users/Some One/Desktop"]["allowedTools"], serde_json::json!([]));
        // No file: Claude Code hasn't run yet, so nothing is written.
        let missing = dir.join("missing.json");
        trust_claude_folder_in(&missing, Path::new("/x")).unwrap();
        assert!(!missing.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(all(test, unix))]
mod private_launch_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn login_shell_consumes_a_private_file_without_putting_credentials_in_argv() {
        let dir = std::env::temp_dir().join(format!("lantern-private-launch-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        for shell in ["/bin/bash", "/bin/zsh"].into_iter().filter(|shell| Path::new(shell).is_file()) {
            let result = dir.join("receipt");
            let script = LaunchScript::in_dir(
                &dir,
                &format!(
                    "LANTERN_TEST_TOKEN={} exec /bin/sh -c {}",
                    quote("dummy-secret-not-a-real-token"),
                    quote(&format!("printf '%s' \"$LANTERN_TEST_TOKEN\" > {}", quote(&result.to_string_lossy())))
                ),
            )
            .unwrap();
            let path = script.0.as_ref().unwrap().clone();
            assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
            assert!(!script.command().contains("dummy-secret"));
            // Test shell parsing without running the developer's personal startup files.
            let mut command = Command::new(shell);
            if shell.ends_with("bash") {
                command.args(["--noprofile", "--norc"]);
            } else {
                command.args(["-d", "-f"]);
            }
            let output = command.args(["-lic", &script.reader_command()]).output().unwrap();
            assert!(output.status.success(), "{shell}: {}", String::from_utf8_lossy(&output.stderr));
            assert_eq!(std::fs::read_to_string(&result).unwrap(), "dummy-secret-not-a-real-token");
            assert!(!path.exists(), "The file must disappear before the agent starts");
            std::fs::remove_file(result).unwrap();
        }
        let cancelled = LaunchScript::in_dir(&dir, "exit 1").unwrap();
        let path = cancelled.0.as_ref().unwrap().clone();
        drop(cancelled);
        assert!(!path.exists(), "Failed launches must remove the file");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod install_folder_tests {
    use super::*;

    #[test]
    fn agents_are_found_in_their_install_folders_when_the_shell_is_too_slow() {
        let home = std::env::temp_dir().join(format!("lantern-install-folders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for dir in [".local/bin", ".nvm/versions/node/v24.1.0/bin"] {
            std::fs::create_dir_all(home.join(dir)).unwrap();
        }
        std::fs::write(home.join(".local/bin/claude"), "").unwrap();
        std::fs::write(home.join(".nvm/versions/node/v24.1.0/bin/codex"), "").unwrap();
        // A folder named like an agent isn't one.
        std::fs::create_dir_all(home.join(".local/bin/gemini")).unwrap();
        let found = in_install_folders(&["claude", "codex", "gemini", "lantern-test-not-an-agent"], &home, &[]);
        assert_eq!(found, ["claude", "codex"]);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn messages_name_agents_as_the_apps_do() {
        assert_eq!(agent_name(Kind::Claude), "Claude Code");
        assert_eq!(agent_name(Kind::Gemini), "Gemini CLI");
        assert_eq!(agent_name(Kind::Cursor), "Cursor");
        // October Bus names are unchanged.
        assert_eq!(display_name(Kind::Gemini), "gemini");
    }
}
