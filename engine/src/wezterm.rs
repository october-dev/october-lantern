//! Replying to agents in WezTerm, and jumping to their panes.
//!
//! WezTerm's own CLI addresses panes by id, and lists them with their ttys:
//! `wezterm cli list --format json` (a stable schema: `pane_id`, `tty_name`, … —
//! `wezterm/src/cli/list.rs`), `wezterm cli send-text --pane-id <id> --no-paste <text>`
//! (raw bytes; `--no-paste` skips the bracketed paste — `wezterm/src/cli/send_text.rs`), and
//! `wezterm cli activate-pane --pane-id <id>` (`wezterm/src/cli/activate_pane.rs`).
//!
//! Like tmux, the pane is looked up by the agent's tty at send time, so a pane that moved is a
//! failure, not a mistype. Finding no pane, or two panes on one tty, refuses.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// `wezterm` from Homebrew, MacPorts, Nix, the system or the app bundle; else next to the engine
/// (as for tmux); "wezterm" (not absolute: rely on PATH) otherwise.
fn wezterm_bin() -> PathBuf {
    static BIN: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BIN.get_or_init(|| {
        let home = crate::transcripts::home();
        let candidates = [
            "/opt/homebrew/bin/wezterm".to_string(),
            "/usr/local/bin/wezterm".into(),
            "/opt/local/bin/wezterm".into(),
            home.join(".nix-profile/bin/wezterm").to_string_lossy().into_owned(),
            "/usr/bin/wezterm".into(),
            "/Applications/WezTerm.app/Contents/MacOS/wezterm".into(),
        ];
        let bundled = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("wezterm").to_string_lossy().into_owned()));
        candidates.into_iter().chain(bundled).map(PathBuf::from).find(|p| p.is_file()).unwrap_or_else(|| "wezterm".into())
    })
    .clone()
}

/// Whether `wezterm cli` can run: a known location, or somewhere on PATH.
pub fn cli_available() -> bool {
    let bin = wezterm_bin();
    if bin.is_absolute() {
        return true;
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("wezterm").is_file() || dir.join("wezterm.exe").is_file()))
}

const LIMIT: Duration = Duration::from_secs(5);

/// One entry of `wezterm cli list --format json` (only the fields Lantern reads).
#[derive(Debug, Clone, Deserialize)]
struct Pane {
    pane_id: u64,
    tty_name: Option<String>,
}

fn list() -> Result<Vec<Pane>> {
    let out = crate::deliver::output_within(Command::new(wezterm_bin()).args(["cli", "list", "--format", "json"]), LIMIT, false)?;
    if !out.status.success() {
        bail!("wezterm cli list failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    serde_json::from_slice(&out.stdout).context("reading wezterm cli list")
}

/// The pane on `tty` (`ttys012`, without `/dev/`), refusing ambiguity.
pub fn pane_for(tty: &str) -> Result<u64> {
    let panes = list()?;
    pane_on(&panes, tty)
}

fn pane_on(panes: &[Pane], tty: &str) -> Result<u64> {
    let want = tty.trim_start_matches("/dev/");
    let mut found = panes.iter().filter(|p| p.tty_name.as_deref().is_some_and(|t| t.trim_start_matches("/dev/") == want));
    match (found.next(), found.next()) {
        (Some(one), None) => Ok(one.pane_id),
        (None, _) => bail!("no WezTerm pane on /dev/{want}"),
        _ => bail!("two WezTerm panes on /dev/{want}; refusing to guess"),
    }
}

/// The exact `wezterm cli …` argv (`--pane-id` before the rest; clap positionals follow flags).
fn argv(pane_id: u64, args: &[&str]) -> Vec<String> {
    let mut full = vec!["cli".to_string(), args[0].to_string(), "--pane-id".to_string(), pane_id.to_string()];
    full.extend(args[1..].iter().map(|s| s.to_string()));
    full
}

fn run(pane_id: u64, args: &[&str], typing: bool) -> Result<std::process::Output> {
    let full = argv(pane_id, args);
    let out = crate::deliver::output_within(Command::new(wezterm_bin()).args(&full), crate::deliver::SEND_LIMIT, typing)?;
    if !out.status.success() {
        bail!("wezterm {} failed: {}", args[0], String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out)
}

/// Type `text` into the pane and press Enter (one line; a newline would submit early).
pub fn send_text(pane_id: u64, text: &str) -> Result<()> {
    let line = text.replace(['\r', '\n'], " ");
    run(pane_id, &["send-text", "--no-paste", &line], true)?;
    // Give the TUI a moment to take the text before Enter.
    std::thread::sleep(Duration::from_millis(60));
    run(pane_id, &["send-text", "--no-paste", "\r"], true)
        .map(|_| ())
        .map_err(|e| crate::deliver::Uncertain(format!("the text went in but Enter didn't ({e:#})")).into())
}

/// Press one key: Escape, Tab, Enter or a single character.
pub fn send_key(pane_id: u64, key: &str) -> Result<()> {
    run(pane_id, &["send-text", "--no-paste", key], true).map(|_| ())
}

/// Bring the pane's tab to the front (the app activates WezTerm afterwards).
pub fn activate(pane_id: u64) -> Result<()> {
    run(pane_id, &["activate-pane"], false).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `wezterm cli list --format json`, trimmed to the fields Lantern reads (plus one it
    /// ignores, to prove unknown fields are tolerated).
    const LIST: &str = r#"[
        {"window_id":0,"tab_id":0,"pane_id":3,"workspace":"default","size":{"rows":40,"cols":120,"pixel_width":0,"pixel_height":0,"dpi":0},"title":"~","cwd":"file:///Users/you","cursor_x":0,"cursor_y":0,"cursor_shape":"SteadyBlock","cursor_visibility":"Visible","left_col":0,"top_row":0,"tab_title":"","window_title":"","is_active":true,"is_zoomed":false,"tty_name":"/dev/ttys012"},
        {"pane_id":7,"tty_name":"/dev/ttys013"},
        {"pane_id":9,"tty_name":null}
    ]"#;

    #[test]
    fn panes_parse_and_match_by_tty() {
        let panes: Vec<Pane> = serde_json::from_str(LIST).expect("the documented schema parses");
        assert_eq!(panes.len(), 3);
        assert_eq!(pane_on(&panes, "ttys012").unwrap(), 3);
        assert_eq!(pane_on(&panes, "/dev/ttys013").unwrap(), 7, "a /dev/ prefix still matches");
        assert!(pane_on(&panes, "ttys999").is_err(), "an unknown tty refuses");
        assert!(pane_on(&panes, "s012").is_err(), "a tty suffix is not a match");
    }

    #[test]
    fn two_panes_on_one_tty_refuse() {
        let panes = vec![Pane { pane_id: 1, tty_name: Some("/dev/ttys012".into()) }, Pane { pane_id: 2, tty_name: Some("ttys012".into()) }];
        let err = pane_on(&panes, "ttys012").unwrap_err();
        assert!(err.to_string().contains("refusing to guess"), "ambiguous: {err}");
    }

    #[test]
    fn cli_commands_are_well_formed() {
        assert_eq!(argv(3, &["send-text", "--no-paste", "hi"]), ["cli", "send-text", "--pane-id", "3", "--no-paste", "hi"]);
        assert_eq!(argv(3, &["activate-pane"]), ["cli", "activate-pane", "--pane-id", "3"]);
    }
}
