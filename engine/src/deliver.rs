//! Typing into an agent's terminal, sending single keys, and bringing its tab to the front.
//!
//! How depends on where the agent runs (its `Route`):
//! - tmux: `tmux send-keys` into the pane
//! - cmux: the `cmux` command-line tool, addressed by the workspace and surface ids cmux puts in
//!   the agent's environment
//! - Terminal and iTerm2: their AppleScript, finding the tab by its tty (asks the user once for
//!   Automation permission)

use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::launch;
use crate::model::{Agent, Route, TmuxPane};
use crate::tmux;

pub const CMUX: &str = "/Applications/cmux.app/Contents/Resources/bin/cmux";

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

/// One line of text, then Enter. Newlines would submit early in most agent UIs.
pub fn send_text(agent: &Agent, text: &str) -> Result<()> {
    let line = text.replace(['\r', '\n'], " ");
    match &agent.route {
        Route::Tmux => tmux::send(pane(agent)?, &line),
        Route::Cmux { workspace, surface } => {
            // `cmux send` treats backslash sequences as escapes; keep the text literal.
            let literal = line.replace('\\', "\\\\");
            cmux(&["send", "--workspace", workspace, "--surface", surface, "--", &literal])?;
            std::thread::sleep(std::time::Duration::from_millis(60));
            cmux(&["send-key", "--workspace", workspace, "--surface", surface, "enter"])
        }
        Route::October { .. } => bail!("replies to October's agents go through October (handled by the serve loop)"),
        Route::Terminal { tty } => osascript(TERMINAL_SEND, &[tty, &line]),
        Route::Iterm { tty } => osascript(ITERM_SEND, &[tty, &line, "yes"]),
        Route::None => bail!("Lantern can't type into {} yet", host_name(agent)),
    }
}

pub fn send_key(agent: &Agent, key: Key) -> Result<()> {
    match (&agent.route, key) {
        (Route::Tmux, key) => {
            let p = pane(agent)?;
            let name = match key {
                Key::Escape => "Escape".to_string(),
                Key::Char(c) => c.to_string(),
            };
            tmux::send_key(p, &name)
        }
        (Route::Cmux { workspace, surface }, Key::Escape) => {
            cmux(&["send-key", "--workspace", workspace, "--surface", surface, "escape"])
        }
        (Route::Cmux { workspace, surface }, Key::Char(c)) => {
            cmux(&["send", "--workspace", workspace, "--surface", surface, "--", &c.to_string()])
        }
        (Route::Iterm { tty }, Key::Escape) => osascript(ITERM_SEND, &[tty, "\u{1b}", "no"]),
        (Route::Iterm { tty }, Key::Char(c)) => osascript(ITERM_SEND, &[tty, &c.to_string(), "no"]),
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
        Route::Cmux { workspace, .. } => cmux(&["select-workspace", "--workspace", workspace]),
        Route::Terminal { tty } => osascript(TERMINAL_FOCUS, &[tty]),
        Route::Iterm { tty } => osascript(ITERM_FOCUS, &[tty]),
        Route::None => Ok(()),
    }
}

fn pane(agent: &Agent) -> Result<&TmuxPane> {
    agent.tmux.as_ref().context("agent is not in tmux")
}

fn host_name(agent: &Agent) -> String {
    agent.host.as_ref().map(|h| h.app.clone()).unwrap_or_else(|| "this terminal".into())
}

fn cmux(args: &[&str]) -> Result<()> {
    let out = Command::new(CMUX).args(args).output().context("running cmux")?;
    if !out.status.success() {
        bail!("cmux: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Runs AppleScript with arguments (passed as `argv`, so no quoting problems). The script
/// returns "ok" when it found the tab.
fn osascript(script: &str, args: &[&str]) -> Result<()> {
    let mut cmd = Command::new("/usr/bin/osascript");
    for line in script.lines() {
        cmd.args(["-e", line]);
    }
    let out = cmd.args(args).output().context("running osascript")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("-1743") || err.contains("Not authorized") {
            bail!("Lantern needs permission to control this app. Allow it in System Settings › Privacy & Security › Automation.");
        }
        bail!("{}", err.trim());
    }
    if stdout.trim() != "ok" {
        bail!("couldn't find the agent's tab");
    }
    Ok(())
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
