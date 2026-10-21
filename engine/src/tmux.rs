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

/// tmux from Homebrew, MacPorts, Nix or the system; else the one bundled next to the engine in the
/// Mac app (macOS doesn't come with tmux); "tmux" (not absolute: not installed) otherwise.
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
        let bundled = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("tmux").to_string_lossy().into_owned()));
        candidates.into_iter().chain(bundled).find(|p| std::path::Path::new(p).is_file()).unwrap_or_else(|| "tmux".into())
    })
}

/// A tmux command that works in a UTF-8 locale. Apps on macOS start with no locale at all, and
/// tmux without a UTF-8 one prints tabs and other characters as `_`: Lantern then can't read
/// `list-panes` (so no agent is matched to its session) and typed text loses non-ASCII letters.
/// A UTF-8 locale already set is kept.
pub fn command() -> Command {
    let mut cmd = Command::new(tmux_bin());
    if let Some((name, value)) = utf8_locale(|name| std::env::var(name).ok()) {
        cmd.env(name, value);
    }
    cmd
}

/// The locale variable to set for tmux, if the environment (`get`) has no UTF-8 one.
fn utf8_locale(get: impl Fn(&str) -> Option<String>) -> Option<(&'static str, &'static str)> {
    let utf8 = |value: &str| value.to_ascii_uppercase().replace('-', "").contains("UTF8");
    // The first of LC_ALL, LC_CTYPE and LANG that is set decides, as for tmux itself.
    let current = ["LC_ALL", "LC_CTYPE", "LANG"].into_iter().find_map(|name| get(name).filter(|v| !v.is_empty()).map(|v| (name, v)));
    let value = if cfg!(target_os = "macos") { "en_US.UTF-8" } else { "C.UTF-8" };
    match current {
        Some((_, v)) if utf8(&v) => None,
        // LC_ALL overrides LC_CTYPE, so a non-UTF-8 LC_ALL is the one to replace.
        Some(("LC_ALL", _)) => Some(("LC_ALL", value)),
        _ => Some(("LC_CTYPE", value)),
    }
}

fn base_command(socket: &Option<String>) -> Command {
    let mut cmd = command();
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
        if p.name != "tmux" && !p.name.starts_with("tmux:") && p.cmd.first().map(|a| basename(a)) != Some("tmux") {
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
    // Linux tmux rewrites argv to "tmux: server", losing its original -L/-S arguments.
    // Enumerate only our user's socket directory so named servers remain discoverable.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let uid = unsafe { libc::getuid() };
        let root = std::env::var_os("TMUX_TMPDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "/tmp".into());
        for entry in std::fs::read_dir(root.join(format!("tmux-{uid}"))).into_iter().flatten().flatten() {
            if entry.metadata().is_ok_and(|m| m.file_type().is_socket() && m.uid() == uid) {
                any = true;
                out.insert(Some(entry.path().to_string_lossy().into_owned()));
            }
        }
    }
    if !any {
        out.clear();
    }
    out
}

/// A tmux server that doesn't answer within this is skipped for this scan.
const DISCOVER_LIMIT: std::time::Duration = std::time::Duration::from_secs(2);

pub fn discover(table: &ProcTable) -> TmuxInfo {
    let mut info = TmuxInfo { panes_by_tty: HashMap::new(), clients: HashMap::new() };
    for socket in sockets(table) {
        let panes = &mut base_command(&socket);
        let panes = panes.args(["list-panes", "-a", "-F", "#{pane_tty}\t#{pane_id}\t#{session_name}:#{window_index}.#{pane_index}"]);
        let panes = crate::run::output(panes, DISCOVER_LIMIT);
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
        let clients =
            crate::run::output(base_command(&socket).args(["list-clients", "-F", "#{client_pid}\t#{session_name}"]), DISCOVER_LIMIT);
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

#[cfg(test)]
mod locale_tests {
    #[test]
    fn tmux_gets_a_utf8_locale_only_when_it_has_none() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
        };
        let set = super::utf8_locale(env(&[])).expect("an app with no locale gets one");
        assert_eq!(set.0, "LC_CTYPE");
        assert!(set.1.ends_with("UTF-8"));
        assert_eq!(super::utf8_locale(env(&[("LANG", "en_GB.UTF-8")])), None, "a UTF-8 locale is kept");
        assert_eq!(super::utf8_locale(env(&[("LC_CTYPE", "en_US.utf8")])), None);
        assert_eq!(super::utf8_locale(env(&[("LC_ALL", "C")])).map(|(name, _)| name), Some("LC_ALL"), "LC_ALL wins, so it's replaced");
        assert_eq!(super::utf8_locale(env(&[("LANG", "C")])).map(|(name, _)| name), Some("LC_CTYPE"));
    }
}
