//! tmux discovery and delivery. Agents in tmux panes are the ones Lantern can reply to directly.

use std::collections::{BTreeSet, HashMap};
use std::process::Command;

use anyhow::{Result, bail};

use crate::model::TmuxPane;
use crate::procs::{ProcTable, basename};

pub struct TmuxInfo {
    /// Pane tty (`ttys012`) → pane.
    pub panes_by_tty: HashMap<String, TmuxPane>,
    /// (socket, session name) → pid of an attached tmux client, used to find the terminal app.
    pub clients: HashMap<(Option<String>, String), u32>,
}

/// tmux from Homebrew, MacPorts, Nix or the system; "tmux" (not absolute: not installed) otherwise.
pub fn tmux_bin() -> &'static str {
    static BIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BIN.get_or_init(|| {
        let home = crate::transcripts::home();
        let user = std::env::var("USER").unwrap_or_default();
        let candidates = [
            "/opt/homebrew/bin/tmux".to_string(),
            "/usr/local/bin/tmux".into(),
            "/opt/local/bin/tmux".into(),
            home.join(".nix-profile/bin/tmux").to_string_lossy().into_owned(),
            format!("/etc/profiles/per-user/{user}/bin/tmux"),
            "/run/current-system/sw/bin/tmux".into(),
            "/usr/bin/tmux".into(),
        ];
        candidates.into_iter().find(|p| std::path::Path::new(p).exists()).unwrap_or_else(|| "tmux".into())
    })
}

fn base_command(socket: &Option<String>) -> Command {
    let mut cmd = Command::new(tmux_bin());
    match socket {
        Some(s) if s.starts_with('/') => {
            cmd.args(["-S", s]);
        }
        Some(s) => {
            cmd.args(["-L", s]);
        }
        None => {}
    }
    cmd
}

/// Every tmux server we can see: the default one plus any `-L`/`-S` sockets on running tmux processes.
fn sockets(table: &ProcTable) -> BTreeSet<Option<String>> {
    let mut out = BTreeSet::new();
    let mut any = false;
    for p in table.procs.values() {
        if p.name != "tmux" && p.cmd.first().map(|a| basename(a)) != Some("tmux") {
            continue;
        }
        any = true;
        let mut it = p.cmd.iter();
        let mut found = None;
        while let Some(a) = it.next() {
            if a == "-L" || a == "-S" {
                found = it.next().cloned();
                break;
            }
        }
        out.insert(found);
    }
    if !any {
        out.clear();
    }
    out
}

pub fn discover(table: &ProcTable) -> TmuxInfo {
    let mut info = TmuxInfo { panes_by_tty: HashMap::new(), clients: HashMap::new() };
    for socket in sockets(table) {
        let panes = base_command(&socket)
            .args(["list-panes", "-a", "-F", "#{pane_tty}\t#{pane_id}\t#{session_name}:#{window_index}.#{pane_index}"])
            .output();
        if let Ok(out) = panes {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() != 3 {
                    continue;
                }
                let tty = parts[0].trim_start_matches("/dev/").to_string();
                info.panes_by_tty
                    .insert(tty, TmuxPane { socket: socket.clone(), pane_id: parts[1].to_string(), target: parts[2].to_string() });
            }
        }
        let clients = base_command(&socket).args(["list-clients", "-F", "#{client_pid}\t#{session_name}"]).output();
        if let Ok(out) = clients {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Some((pid, session)) = line.split_once('\t')
                    && let Ok(pid) = pid.parse()
                {
                    info.clients.insert((socket.clone(), session.to_string()), pid);
                }
            }
        }
    }
    info
}

/// Runs a tmux command against the pane's server, with a time limit.
pub fn output(pane: &TmuxPane, args: &[&str]) -> Result<std::process::Output> {
    let out = crate::deliver::output_within(base_command(&pane.socket).args(args), LIMIT, false)?;
    if !out.status.success() {
        bail!("tmux {} failed: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out)
}

pub fn run(pane: &TmuxPane, args: &[&str]) -> Result<()> {
    output(pane, args).map(|_| ())
}

const LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Presses one key (a tmux key name such as `Escape`, or a single character).
pub fn send_key(pane: &TmuxPane, key: &str) -> Result<()> {
    typing(pane, &["send-keys", "-t", &pane.pane_id, key])
}

/// Type `text` into the pane and press Enter.
pub fn send(pane: &TmuxPane, text: &str) -> Result<()> {
    // A newline would submit early in most agent TUIs, so send one line.
    let line = text.replace(['\r', '\n'], " ");
    typing(pane, &["send-keys", "-t", &pane.pane_id, "-l", "--", &line])?;
    // Give the TUI a moment to take the pasted text before Enter.
    std::thread::sleep(std::time::Duration::from_millis(60));
    typing(pane, &["send-keys", "-t", &pane.pane_id, "Enter"])
        .map_err(|e| crate::deliver::Uncertain(format!("the text went in but Enter didn't ({e:#})")).into())
}

/// A tmux command that types: running out of time means it may have typed.
fn typing(pane: &TmuxPane, args: &[&str]) -> Result<()> {
    let out = crate::deliver::output_within(base_command(&pane.socket).args(args), LIMIT, true)?;
    if !out.status.success() {
        bail!("tmux send-keys failed for {}: {}", pane.pane_id, String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}
