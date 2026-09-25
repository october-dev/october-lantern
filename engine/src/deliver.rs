//! Typing into an agent's terminal, sending single keys, and bringing its tab to the front.
//!
//! How depends on where the agent runs (its `Route`):
//! - tmux: `tmux send-keys` into the pane
//! - cmux: the `cmux` command-line tool, addressed by the workspace and surface ids cmux puts in
//!   the agent's environment
//! - Terminal and iTerm2: their AppleScript, finding the tab by its tty (asks the user once for
//!   Automation permission)
//!
//! Every send first checks that the terminal still belongs to the agent Lantern saw: the same
//! process (pid and start time), still running the same program, on the same terminal, and in
//! that terminal's foreground. An agent that has exited, or `exec`ed a shell, leaves a shell
//! behind, and typing an instruction plus Enter into a shell would run it. When Lantern can't tell
//! who owns the terminal it refuses rather than guess.
//!
//! The check runs immediately before typing (after any Automation prompt has been answered); what
//! remains is the time one `tmux`/`osascript` call takes to start. Every call has a time limit: a
//! send that runs out of time after it started is reported as uncertain, not as failed.

use std::process::{Command, Output};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::launch;
use crate::model::{Agent, Route, TmuxPane};
use crate::procs::{self, Live};
use crate::tmux;

pub const CMUX: &str = "/Applications/cmux.app/Contents/Resources/bin/cmux";

/// How long one send (text + Enter) may take once it has started.
const SEND_LIMIT: Duration = Duration::from_secs(10);

/// A single keypress, for answering prompts without Enter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Escape,
}

impl Key {
    pub fn parse(s: &str) -> Option<Key> {
        match s {
            "Escape" | "escape" | "esc" => Some(Key::Escape),
            s if s.chars().count() == 1 => s.chars().next().map(Key::Char),
            _ => None,
        }
    }
}

/// A send that started but whose outcome Lantern can't know (it ran out of time, or the text went
/// in and Enter didn't). Retrying could type the message twice.
#[derive(Debug)]
pub struct Uncertain(pub String);

impl std::fmt::Display for Uncertain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Uncertain {}

/// Refuses to type unless the terminal still belongs to the agent Lantern saw (see the module
/// comment).
pub fn verify(agent: &Agent) -> Result<()> {
    let Some(live) = procs::live(agent.pid) else { bail!("@{} has exited", agent.handle) };
    check(agent, &live)
}

/// `verify` against a given reading of the process, so every rule can be tested.
pub fn check(agent: &Agent, live: &Live) -> Result<()> {
    let h = &agent.handle;
    if live.start != agent.start_time {
        bail!("@{h} has exited and another process took its place");
    }
    // The program must be the one Lantern classified. When the kernel can't say which file a
    // process runs (its executable was deleted by an update), its short name must still match.
    let same_program = match (&agent.exe, &live.exe) {
        (Some(saw), Some(now)) => saw == now,
        _ => !agent.comm.is_empty() && agent.comm == live.comm,
    };
    if !same_program {
        bail!("@{h} is no longer running its agent (the process now runs {})", live.exe.as_deref().unwrap_or(&live.comm));
    }
    let Some(tty) = &agent.tty else { bail!("@{h} has no terminal Lantern can type into") };
    if live.tty.as_ref() != Some(tty) {
        bail!("@{h} is no longer on the terminal Lantern saw it in");
    }
    if live.tpgid == 0 {
        bail!("Lantern can't tell what's in the foreground of @{h}'s terminal, so it won't type there");
    }
    if live.tpgid != live.pgid {
        bail!("@{h} isn't in the foreground of its terminal (suspended, or something else is running there)");
    }
    match &agent.route {
        Route::Terminal { tty: route_tty } | Route::Iterm { tty: route_tty } if *route_tty != format!("/dev/{tty}") => {
            bail!("@{h}'s terminal tab changed")
        }
        _ => Ok(()),
    }
}

/// tmux panes are addressed by id; checks the pane still sits on the agent's terminal.
fn verify_pane(agent: &Agent, pane: &TmuxPane) -> Result<()> {
    let out = tmux::output(pane, &["display-message", "-p", "-t", &pane.pane_id, "#{pane_tty}"])?;
    let pane_tty = String::from_utf8_lossy(&out.stdout).trim().trim_start_matches("/dev/").to_string();
    if agent.tty.as_deref() != Some(pane_tty.as_str()) {
        bail!("@{}'s tmux pane changed", agent.handle);
    }
    Ok(())
}

/// Asks the terminal app for Automation permission (and waits while the person answers the
/// dialog) without typing anything, so the check before typing happens after the wait.
pub fn prepare(agent: &Agent, limit: Duration) -> Result<()> {
    match &agent.route {
        Route::Terminal { .. } => osascript("tell application \"Terminal\" to count windows", &[], limit, false).map(|_| ()),
        Route::Iterm { .. } => osascript("tell application id \"com.googlecode.iterm2\" to count windows", &[], limit, false).map(|_| ()),
        _ => Ok(()),
    }
}

/// One line of text, then Enter. Newlines would submit early in most agent UIs.
pub fn send_text(agent: &Agent, text: &str) -> Result<()> {
    verify(agent)?;
    let line = text.replace(['\r', '\n'], " ");
    match &agent.route {
        Route::Tmux => {
            let p = pane(agent)?;
            verify_pane(agent, p)?;
            tmux::send(p, &line)
        }
        Route::Cmux { workspace, surface } => {
            // `cmux send` treats backslash sequences as escapes; keep the text literal.
            let literal = line.replace('\\', "\\\\");
            cmux(&["send", "--workspace", workspace, "--surface", surface, "--", &literal])?;
            std::thread::sleep(Duration::from_millis(60));
            cmux(&["send-key", "--workspace", workspace, "--surface", surface, "enter"])
                .map_err(|e| Uncertain(format!("the text went in but Enter didn't ({e:#})")).into())
        }
        Route::October { .. } => bail!("replies to October's agents go through October"),
        Route::Terminal { tty } => osascript(TERMINAL_SEND, &[tty, &line], SEND_LIMIT, true).map(|_| ()),
        Route::Iterm { tty } => osascript(ITERM_SEND, &[tty, &line, "yes"], SEND_LIMIT, true).map(|_| ()),
        Route::None => bail!("Lantern can't type into {} yet", host_name(agent)),
    }
}

pub fn send_key(agent: &Agent, key: Key) -> Result<()> {
    verify(agent)?;
    match (&agent.route, key) {
        (Route::Tmux, key) => {
            let p = pane(agent)?;
            verify_pane(agent, p)?;
            let name = match key {
                Key::Escape => "Escape".to_string(),
                Key::Char(c) => c.to_string(),
            };
            tmux::send_key(p, &name)
        }
        (Route::Cmux { workspace, surface }, Key::Escape) => cmux(&["send-key", "--workspace", workspace, "--surface", surface, "escape"]),
        (Route::Cmux { workspace, surface }, Key::Char(c)) => {
            cmux(&["send", "--workspace", workspace, "--surface", surface, "--", &c.to_string()])
        }
        (Route::Iterm { tty }, Key::Escape) => osascript(ITERM_SEND, &[tty, "\u{1b}", "no"], SEND_LIMIT, true).map(|_| ()),
        (Route::Iterm { tty }, Key::Char(c)) => osascript(ITERM_SEND, &[tty, &c.to_string(), "no"], SEND_LIMIT, true).map(|_| ()),
        (Route::October { .. }, _) => bail!("single keys aren't supported for October's agents yet"),
        // Terminal's `do script` always adds Enter, which could confirm the wrong thing.
        (Route::Terminal { .. }, _) => bail!("single keys aren't supported in Terminal"),
        (Route::None, _) => bail!("Lantern can't type into {} yet", host_name(agent)),
    }
}

/// Brings the agent's own tab to the front. The app activates the host app afterwards.
pub fn focus(agent: &Agent) -> Result<()> {
    match &agent.route {
        Route::Tmux => {
            let p = pane(agent)?;
            if agent.host.is_none() {
                // Nobody is attached (e.g. a background session Lantern started).
                return launch::attach_in_terminal(p);
            }
            let window = p.target.rsplit_once('.').map(|(w, _)| w).unwrap_or(&p.target);
            tmux::run(p, &["select-window", "-t", window])?;
            tmux::run(p, &["select-pane", "-t", &p.pane_id])
        }
        Route::October { .. } => Ok(()),
        Route::None if agent.host.as_ref().and_then(|h| h.canvas.as_ref()).is_some() => {
            // An agent in October Desktop that Lantern isn't paired with: open its canvas.
            let canvas = agent.host.as_ref().and_then(|h| h.canvas.as_deref()).unwrap_or_default();
            let url = format!("october://canvas/{canvas}");
            let out = output_within(Command::new("/usr/bin/open").arg(&url), SEND_LIMIT, false)?;
            if !out.status.success() {
                bail!("October didn't open the canvas");
            }
            Ok(())
        }
        Route::Cmux { workspace, .. } => cmux(&["select-workspace", "--workspace", workspace]),
        Route::Terminal { tty } => osascript(TERMINAL_FOCUS, &[tty], SEND_LIMIT, false).map(|_| ()),
        Route::Iterm { tty } => osascript(ITERM_FOCUS, &[tty], SEND_LIMIT, false).map(|_| ()),
        Route::None => Ok(()),
    }
}

fn pane(agent: &Agent) -> Result<&TmuxPane> {
    agent.tmux.as_ref().context("agent is not in tmux")
}

fn host_name(agent: &Agent) -> String {
    agent.host.as_ref().map(|h| h.app.clone()).unwrap_or_else(|| "this terminal".into())
}

/// Runs a command with a time limit (see `run.rs`). Running out of time, for a command that
/// types (`typing`), is an `Uncertain` error: some of it may have gone in.
pub fn output_within(cmd: &mut Command, limit: Duration, typing: bool) -> Result<Output> {
    crate::run::output(cmd, limit).map_err(|e| match e.downcast_ref::<crate::run::TimedOut>() {
        Some(t) if typing => Uncertain(format!("{t}; the message may or may not have gone in")).into(),
        _ => e,
    })
}

fn cmux(args: &[&str]) -> Result<()> {
    let out = output_within(Command::new(CMUX).args(args), SEND_LIMIT, true)?;
    if !out.status.success() {
        bail!("cmux: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Runs AppleScript with arguments (passed as `argv`, so no quoting problems). Scripts that look
/// for a tab return "ok" when they found it.
fn osascript(script: &str, args: &[&str], limit: Duration, typing: bool) -> Result<String> {
    let mut cmd = Command::new("/usr/bin/osascript");
    for line in script.lines() {
        cmd.args(["-e", line]);
    }
    let out = output_within(cmd.args(args), limit, typing)?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("-1743") || err.contains("Not authorized") {
            bail!("Lantern needs permission to control this app. Allow it in System Settings › Privacy & Security › Automation.");
        }
        bail!("{}", err.trim());
    }
    if script.contains("return \"missing\"") && stdout != "ok" {
        bail!("couldn't find the agent's tab");
    }
    Ok(stdout)
}

const TERMINAL_SEND: &str = r#"on run argv
set theTTY to item 1 of argv
tell application "Terminal"
repeat with w in windows
repeat with t in tabs of w
if tty of t is theTTY then
do script (item 2 of argv) in t
return "ok"
end if
end repeat
end repeat
end tell
return "missing"
end run"#;

const TERMINAL_FOCUS: &str = r#"on run argv
set theTTY to item 1 of argv
tell application "Terminal"
repeat with w in windows
repeat with t in tabs of w
if tty of t is theTTY then
set selected of t to true
set index of w to 1
activate
return "ok"
end if
end repeat
end repeat
end tell
return "missing"
end run"#;

const ITERM_SEND: &str = r#"on run argv
set theTTY to item 1 of argv
tell application id "com.googlecode.iterm2"
repeat with w in windows
repeat with t in tabs of w
repeat with s in sessions of t
if tty of s is theTTY then
if item 3 of argv is "yes" then
tell s to write text (item 2 of argv)
else
tell s to write text (item 2 of argv) newline no
end if
return "ok"
end if
end repeat
end repeat
end repeat
end tell
return "missing"
end run"#;

const ITERM_FOCUS: &str = r#"on run argv
set theTTY to item 1 of argv
tell application id "com.googlecode.iterm2"
repeat with w in windows
repeat with t in tabs of w
repeat with s in sessions of t
if tty of s is theTTY then
tell s to select
tell t to select
tell w to select
activate
return "ok"
end if
end repeat
end repeat
end repeat
end tell
return "missing"
end run"#;
