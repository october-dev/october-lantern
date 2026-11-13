//! Replying to agents in WezTerm, and jumping to their panes.
//!
//! The agent's own WezTerm CLI addresses panes by id, and lists them with their ttys:
//! `wezterm cli --no-auto-start list --format json` (a stable schema: `pane_id`, `tty_name`, … —
//! `wezterm/src/cli/list.rs`), `wezterm cli --no-auto-start send-text --pane-id <id> --no-paste -- <text>`
//! (raw bytes; `--no-paste` skips the bracketed paste, and `--` keeps text starting with a
//! dash from being read as options — `wezterm/src/cli/send_text.rs`), and
//! `wezterm cli --no-auto-start activate-pane --pane-id <id>` (`wezterm/src/cli/activate_pane.rs`).
//! `--no-auto-start` keeps a failed connection from starting a background server Lantern didn't
//! ask for.
//!
//! Every call runs the CLI from the agent's own app bundle against the agent's own instance
//! (its `WEZTERM_UNIX_SOCKET`, set per command), so a second WezTerm process never receives the
//! typing. Like tmux, the pane is looked up by the agent's tty at send time, so a pane that moved
//! is a failure, not a mistype. Finding no pane, or two panes on one tty, refuses.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// The CLI from the agent's own app (`<bundle>/Contents/MacOS/wezterm`), when it exists: the CLI
/// and the instance always come from the same install, wherever it lives.
pub fn cli_for(bundle_path: &str) -> Option<PathBuf> {
    let cli = Path::new(bundle_path).join("Contents/MacOS/wezterm");
    cli.is_file().then_some(cli)
}

const LIMIT: Duration = Duration::from_secs(5);

/// One entry of `wezterm cli list --format json` (only the fields Lantern reads).
#[derive(Debug, Clone, Deserialize)]
struct Pane {
    pane_id: u64,
    tty_name: Option<String>,
}

/// One `wezterm cli` call against the agent's own instance: `--no-auto-start` right after
/// `cli`, so a GUI that isn't published never starts a background server (and shell) Lantern
/// didn't ask for, and the agent's `WEZTERM_UNIX_SOCKET` on this command only.
fn command(cli: &Path, socket: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(cli);
    cmd.env("WEZTERM_UNIX_SOCKET", socket);
    cmd.arg("cli").arg("--no-auto-start").args(args);
    cmd
}

fn list(cli: &Path, socket: &str) -> Result<Vec<Pane>> {
    let out = crate::deliver::output_within(&mut command(cli, socket, &["list", "--format", "json"]), LIMIT, false)?;
    if !out.status.success() {
        bail!("wezterm cli list failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    serde_json::from_slice(&out.stdout).context("reading wezterm cli list")
}

/// The pane on `tty` (`ttys012`, without `/dev/`), refusing ambiguity.
pub fn pane_for(cli: &Path, socket: &str, tty: &str) -> Result<u64> {
    let panes = list(cli, socket)?;
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

/// The `wezterm cli …` arguments after the binary (`--pane-id` before the rest; clap
/// positionals follow flags). `--` sits before every payload, so text starting with a dash is
/// typed, not read as options.
fn argv(pane_id: u64, args: &[&str]) -> Vec<String> {
    let mut full = vec![args[0].to_string(), "--pane-id".to_string(), pane_id.to_string()];
    full.extend(args[1..].iter().map(|s| s.to_string()));
    full
}

fn run(cli: &Path, socket: &str, pane_id: u64, args: &[&str], typing: bool) -> Result<std::process::Output> {
    let full = argv(pane_id, args);
    run_full(cli, socket, &full, args[0], typing)
}

fn run_full(cli: &Path, socket: &str, full: &[String], label: &str, typing: bool) -> Result<std::process::Output> {
    let refs: Vec<&str> = full.iter().map(String::as_str).collect();
    let out = crate::deliver::output_within(&mut command(cli, socket, &refs), crate::deliver::SEND_LIMIT, typing)?;
    if !out.status.success() {
        bail!("wezterm {label} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out)
}

/// The `send-text` call for one payload. `--` before the payload is part of the contract here,
/// not at each call site, so text starting with a dash is typed, never read as options.
fn send_payload_argv(pane_id: u64, payload: &str) -> Vec<String> {
    argv(pane_id, &["send-text", "--no-paste", "--", payload])
}

/// Type `text` into the pane and press Enter (one line; a newline would submit early).
pub fn send_text(cli: &Path, socket: &str, pane_id: u64, text: &str) -> Result<()> {
    let line = text.replace(['\r', '\n'], " ");
    run_full(cli, socket, &send_payload_argv(pane_id, &line), "send-text", true)?;
    // Give the TUI a moment to take the text before Enter.
    std::thread::sleep(Duration::from_millis(60));
    run_full(cli, socket, &send_payload_argv(pane_id, "\r"), "send-text", true)
        .map(|_| ())
        .map_err(|e| crate::deliver::Uncertain(format!("the text went in but Enter didn't ({e:#})")).into())
}

/// Press one key: Escape, Tab, Enter or a single character.
pub fn send_key(cli: &Path, socket: &str, pane_id: u64, key: &str) -> Result<()> {
    run_full(cli, socket, &send_payload_argv(pane_id, key), "send-text", true).map(|_| ())
}

/// Bring the pane's tab to the front (the app activates WezTerm afterwards).
pub fn activate(cli: &Path, socket: &str, pane_id: u64) -> Result<()> {
    run(cli, socket, pane_id, &["activate-pane"], false).map(|_| ())
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
        assert_eq!(argv(3, &["send-text", "--no-paste", "--", "hi"]), ["send-text", "--pane-id", "3", "--no-paste", "--", "hi"]);
        assert_eq!(argv(3, &["activate-pane"]), ["activate-pane", "--pane-id", "3"]);
    }

    const SOCKET: &str = "/Users/you/.local/share/wezterm/gui-sock-1234";

    /// The built command for one payload, as plain strings plus the socket it carries.
    /// Goes through the same production builder `send_text` and `send_key` use, so the
    /// separator test would fail if production lost it.
    fn built(payload: &str) -> (Vec<String>, Option<String>) {
        let full = send_payload_argv(3, payload);
        let refs: Vec<&str> = full.iter().map(String::as_str).collect();
        let cmd = command(Path::new("/Applications/WezTerm.app/Contents/MacOS/wezterm"), SOCKET, &refs);
        let args = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        let socket =
            cmd.get_envs().find(|(k, _)| *k == "WEZTERM_UNIX_SOCKET").and_then(|(_, v)| v).map(|v| v.to_string_lossy().into_owned());
        (args, socket)
    }

    #[test]
    fn commands_never_start_a_server_and_target_the_agents_instance() {
        for payload in ["hi", "-h hello", "--"] {
            let (args, socket) = built(payload);
            assert_eq!(&args[..2], ["cli", "--no-auto-start"], "payload {payload:?}");
            assert_eq!(socket.as_deref(), Some(SOCKET), "payload {payload:?}");
        }
    }

    #[test]
    fn dash_payloads_are_typed_not_parsed() {
        for payload in ["-h hello", "--"] {
            let (args, _) = built(payload);
            let sep = args.iter().position(|a| a == "--").expect("a -- separator");
            assert_eq!(args[sep + 1], payload, "the separator sits immediately before the payload");
            assert_eq!(args[sep - 1], "--no-paste");
        }
    }
}
