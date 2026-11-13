//! Replying to agents in kitty, and jumping to their windows.
//!
//! kitty's remote control addresses windows by id: `kitty @ --to <socket> send-text
//! --match id:<id> -- <text>` and `kitty @ --to <socket> focus-window --match id:<id>`.
//! kitty exports the window id (`KITTY_WINDOW_ID`) in every child's environment, and the
//! control socket (`KITTY_LISTEN_ON`) only when remote control is fully on: `kitty.conf` needs
//! both `allow_remote_control` (`yes`, `socket` or `socket-only`, the tightest that works) and
//! `listen_on unix:/tmp/kitty` (or `kitty --listen-on`), then a restart. The scanner reads both
//! from the agent's own environment (see scanner.rs); a kitty without them simply has no route.
//!
//! `send-text` always reports success even when nothing arrived, so every reply and key first
//! checks the window is still the agent's: `kitty @ ls` lists each window with the pids in its
//! foreground, and the send is refused unless the agent's pid is in its own window's list. That
//! keeps "Sent" honest, stops typing into whatever sits between kitty and the agent (Neovim's
//! `:terminal`), and catches a kitty upgraded while running.
//!
//! `send-text` reads Python-style backslash escapes, so user text goes in with its backslashes
//! doubled, and keys go as the bytes the terminal would deliver (`\e`, `\t`, `\r`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// The CLI from the agent's own app (`<bundle>/Contents/MacOS/kitty`), when it exists: the CLI
/// and the instance always come from the same install, wherever it lives.
pub fn cli_for(bundle_path: &str) -> Option<PathBuf> {
    let cli = Path::new(bundle_path).join("Contents/MacOS/kitty");
    cli.is_file().then_some(cli)
}

/// How long the send-time window check may take. It types nothing, so running out of time is a
/// refusal, not an uncertainty.
const LS_LIMIT: Duration = Duration::from_secs(5);

/// The exact `kitty @ …` argv: the global `--to` before the subcommand, `--match` after it.
fn argv(socket: &str, subcommand: &str, window: &str, rest: &[&str]) -> Vec<String> {
    let mut full = vec![
        "@".to_string(),
        "--to".to_string(),
        socket.to_string(),
        subcommand.to_string(),
        "--match".to_string(),
        format!("id:{window}"),
    ];
    full.extend(rest.iter().map(|s| s.to_string()));
    full
}

/// The `send-text` argv for one payload: `--` sits immediately before it, so text starting with
/// a dash is typed, not read as options.
fn send_argv(socket: &str, window: &str, payload: &str) -> Vec<String> {
    argv(socket, "send-text", window, &["--", payload])
}

fn subcommand(argv: &[String]) -> &str {
    argv.get(3).map(String::as_str).unwrap_or("@")
}

fn run(cli: &Path, argv: Vec<String>, typing: bool) -> Result<()> {
    let out = crate::deliver::output_within(Command::new(cli).args(&argv), crate::deliver::SEND_LIMIT, typing)?;
    if !out.status.success() {
        bail!("kitty {} failed: {}", subcommand(&argv), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// What `kitty @ ls` reports, trimmed to what the send-time check needs: each window's id and
/// the pids in its foreground (kitty `window.py` `as_dict`). Everything else is ignored, so
/// newer kitty fields never break the parse.
#[derive(Deserialize)]
struct LsOsWindow {
    #[serde(default)]
    tabs: Vec<LsTab>,
}

#[derive(Deserialize)]
struct LsTab {
    #[serde(default)]
    windows: Vec<LsWindow>,
}

#[derive(Deserialize)]
struct LsWindow {
    id: u64,
    #[serde(default)]
    foreground_processes: Vec<LsProcess>,
}

#[derive(Deserialize)]
struct LsProcess {
    pid: u32,
}

/// Refuses unless the agent's window is in the `ls` output with the agent's pid in its
/// foreground: a window that moved, or something sitting between kitty and the agent, is a
/// refusal, not a mistype.
fn check_ls(os_windows: &[LsOsWindow], window: u64, pid: u32, handle: &str) -> Result<()> {
    let found = os_windows.iter().flat_map(|o| &o.tabs).flat_map(|t| &t.windows).find(|w| w.id == window);
    let Some(found) = found else { bail!("@{handle}'s kitty window is gone") };
    if !found.foreground_processes.iter().any(|p| p.pid == pid) {
        bail!("@{handle} isn't in the foreground of its kitty window (something else is running there)");
    }
    Ok(())
}

/// The send-time check: the agent's window still exists on its own instance with the agent's
/// pid in its foreground. Refuses when `ls` errors, so a "Sent" is never a guess.
pub fn verify_window(cli: &Path, socket: &str, window: &str, agent: &crate::model::Agent) -> Result<()> {
    let Ok(id) = window.parse::<u64>() else { bail!("@{}'s kitty window isn't one Lantern understands", agent.handle) };
    let out = crate::deliver::output_within(Command::new(cli).args(argv(socket, "ls", window, &[])), LS_LIMIT, false)?;
    if !out.status.success() {
        bail!("kitty ls failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let os_windows: Vec<LsOsWindow> =
        serde_json::from_slice(&out.stdout).with_context(|| format!("@{}'s kitty didn't list its windows", agent.handle))?;
    check_ls(&os_windows, id, agent.pid, &agent.handle)
}

/// User text with its backslashes doubled, so `send-text` takes it literally.
fn literal(text: &str) -> String {
    text.replace('\\', "\\\\")
}

/// Type `text` into the window and press Enter (one line; a newline would submit early).
pub fn send_text(cli: &Path, socket: &str, window: &str, text: &str) -> Result<()> {
    let line = text.replace(['\r', '\n'], " ");
    run(cli, send_argv(socket, window, &literal(&line)), true)?;
    // Give the TUI a moment to take the text before Enter.
    std::thread::sleep(Duration::from_millis(60));
    run(cli, send_argv(socket, window, "\\r"), true)
        .map_err(|e| crate::deliver::Uncertain(format!("the text went in but Enter didn't ({e:#})")).into())
}

/// One key as the bytes the terminal would deliver for it.
fn stroke(key: crate::deliver::Key) -> String {
    match key {
        crate::deliver::Key::Escape => "\\e".to_string(),
        crate::deliver::Key::Tab => "\\t".to_string(),
        crate::deliver::Key::Enter => "\\r".to_string(),
        crate::deliver::Key::Char(c) => literal(&c.to_string()),
    }
}

/// Press one key: Escape, Tab, Enter or a single character.
pub fn send_key(cli: &Path, socket: &str, window: &str, key: crate::deliver::Key) -> Result<()> {
    let stroke = stroke(key);
    run(cli, send_argv(socket, window, &stroke), true)
}

/// Bring the window's tab to the front (the app activates kitty afterwards).
pub fn activate(cli: &Path, socket: &str, window: &str) -> Result<()> {
    run(cli, argv(socket, "focus-window", window, &[]), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_commands_are_well_formed() {
        assert_eq!(
            send_argv("unix:/tmp/kitty-123", "3", "hello"),
            ["@", "--to", "unix:/tmp/kitty-123", "send-text", "--match", "id:3", "--", "hello"]
        );
        assert_eq!(
            argv("unix:/tmp/kitty-123", "focus-window", "3", &[]),
            ["@", "--to", "unix:/tmp/kitty-123", "focus-window", "--match", "id:3"]
        );
        assert_eq!(argv("unix:/tmp/kitty-123", "ls", "3", &[]), ["@", "--to", "unix:/tmp/kitty-123", "ls", "--match", "id:3"]);
    }

    #[test]
    fn dash_payloads_are_typed_not_parsed() {
        for payload in ["-h hello", "--", "--from-file=/etc/hosts"] {
            let full = send_argv("unix:/tmp/kitty-123", "3", payload);
            let sep = full.iter().position(|a| a == "--").expect("a -- separator");
            assert_eq!(full[sep + 1], payload, "the separator sits immediately before the payload");
        }
    }

    #[test]
    fn backslashes_stay_literal() {
        assert_eq!(literal("plain"), "plain");
        assert_eq!(literal("C:\\temp\\new"), "C:\\\\temp\\\\new");
        assert_eq!(literal("\\e"), "\\\\e", "an escape-looking reply must not become Escape");
    }

    #[test]
    fn keys_send_as_terminal_bytes() {
        assert_eq!(stroke(crate::deliver::Key::Escape), "\\e");
        assert_eq!(stroke(crate::deliver::Key::Tab), "\\t");
        assert_eq!(stroke(crate::deliver::Key::Enter), "\\r");
        assert_eq!(stroke(crate::deliver::Key::Char('1')), "1");
        assert_eq!(stroke(crate::deliver::Key::Char('\\')), "\\\\");
    }

    /// A trimmed `kitty @ ls` answer: the agent's window (id 3) with pid 4242 in its
    /// foreground, another window running something else, and an empty second OS window.
    const LS_SAMPLE: &str = r#"[
  {"id": 1, "tabs": [{"id": 1, "windows": [
    {"id": 3, "title": "agent", "pid": 4240, "foreground_processes": [{"pid": 4242, "cmdline": ["claude"], "cwd": "/tmp"}]},
    {"id": 4, "title": "other", "pid": 4300, "foreground_processes": [{"pid": 4310}]}
  ]}]},
  {"id": 2, "tabs": []}
]"#;

    #[test]
    fn window_check_reads_foreground_pids() {
        let os_windows: Vec<LsOsWindow> = serde_json::from_str(LS_SAMPLE).expect("the sample parses");
        check_ls(&os_windows, 3, 4242, "claude-1").expect("the agent's pid in its window passes");
        check_ls(&os_windows, 3, 9999, "claude-1").expect_err("a pid that isn't in the foreground refuses");
        check_ls(&os_windows, 4, 4242, "claude-1").expect_err("the agent's pid in another window refuses");
        check_ls(&os_windows, 7, 4242, "claude-1").expect_err("a missing window refuses");
    }
}
