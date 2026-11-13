//! Replying to agents in kitty, and jumping to their windows.
//!
//! kitty's remote control addresses windows by id: `kitty @ --to <socket> send-text
//! --match id:<id> <text>` and `kitty @ --to <socket> focus-window --match id:<id>`.
//! kitty exports the window id (`KITTY_WINDOW_ID`) in every child's environment, and the
//! control socket (`KITTY_LISTEN_ON`) when remote control is on (`allow_remote_control` in
//! `kitty.conf`), so the scanner reads both from the agent's own environment (see scanner.rs):
//! the window can't be any window but the agent's, and a kitty without remote control simply
//! has no route.
//!
//! `send-text` reads Python-style backslash escapes, so user text goes in with its backslashes
//! doubled, and keys go as the bytes the terminal would deliver (`\e`, `\t`, `\r`).

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Result, bail};

/// `kitty` from Homebrew, MacPorts, Nix, the system, `~/.local/bin` (kitty's own installer
/// default) or the app bundle; else next to the engine (as for tmux); "kitty" (not absolute:
/// rely on PATH) otherwise.
fn kitty_bin() -> PathBuf {
    static BIN: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BIN.get_or_init(|| {
        let home = crate::transcripts::home();
        let candidates = [
            "/opt/homebrew/bin/kitty".to_string(),
            "/usr/local/bin/kitty".into(),
            "/opt/local/bin/kitty".into(),
            home.join(".nix-profile/bin/kitty").to_string_lossy().into_owned(),
            home.join(".local/bin/kitty").to_string_lossy().into_owned(),
            "/usr/bin/kitty".into(),
            "/Applications/kitty.app/Contents/MacOS/kitty".into(),
        ];
        let bundled = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("kitty").to_string_lossy().into_owned()));
        candidates.into_iter().chain(bundled).map(PathBuf::from).find(|p| p.is_file()).unwrap_or_else(|| "kitty".into())
    })
    .clone()
}

/// Whether `kitty @` can run: a known location, or somewhere on PATH.
pub fn cli_available() -> bool {
    let bin = kitty_bin();
    if bin.is_absolute() {
        return true;
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("kitty").is_file() || dir.join("kitty.exe").is_file()))
}

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

fn run(argv: Vec<String>, typing: bool) -> Result<()> {
    let out = crate::deliver::output_within(Command::new(kitty_bin()).args(&argv), crate::deliver::SEND_LIMIT, typing)?;
    if !out.status.success() {
        bail!("kitty {} failed: {}", argv.get(3).map(String::as_str).unwrap_or("@"), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// User text with its backslashes doubled, so `send-text` takes it literally.
fn literal(text: &str) -> String {
    text.replace('\\', "\\\\")
}

/// Type `text` into the window and press Enter (one line; a newline would submit early).
pub fn send_text(window: &str, socket: &str, text: &str) -> Result<()> {
    let line = text.replace(['\r', '\n'], " ");
    run(argv(socket, "send-text", window, &[&literal(&line)]), true)?;
    // Give the TUI a moment to take the text before Enter.
    std::thread::sleep(Duration::from_millis(60));
    run(argv(socket, "send-text", window, &["\\r"]), true)
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
pub fn send_key(window: &str, socket: &str, key: crate::deliver::Key) -> Result<()> {
    let stroke = stroke(key);
    run(argv(socket, "send-text", window, &[&stroke]), true)
}

/// Bring the window's tab to the front (the app activates kitty afterwards).
pub fn activate(window: &str, socket: &str) -> Result<()> {
    run(argv(socket, "focus-window", window, &[]), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_commands_are_well_formed() {
        assert_eq!(
            argv("unix:/tmp/kitty-123", "send-text", "3", &["hello"]),
            ["@", "--to", "unix:/tmp/kitty-123", "send-text", "--match", "id:3", "hello"]
        );
        assert_eq!(
            argv("unix:/tmp/kitty-123", "focus-window", "3", &[]),
            ["@", "--to", "unix:/tmp/kitty-123", "focus-window", "--match", "id:3"]
        );
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
}
