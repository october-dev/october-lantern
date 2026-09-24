//! Sessions that don't run in a terminal: the Claude desktop app (its Code tab and Cowork) and the
//! Codex app, plus a better view of every Claude Code session.
//!
//! - Claude Code keeps a status file per running session in `~/.claude/sessions/<pid>.json`: its
//!   session id, `busy` or `idle`, and where it was started (`entrypoint`: `cli`,
//!   `claude-desktop` for the app's Code tab, `local-agent` for Cowork). That gives the exact
//!   session and a live busy/idle for every Claude process, with or without hooks.
//! - The Codex app runs its threads inside one `codex app-server` process, which holds each
//!   thread's rollout file open while it runs; `~/.codex/state_5.sqlite` indexes every thread.
//! - Sessions from these apps that aren't running any more are listed for three days.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use crate::transcripts::home;

/// How long an app session that isn't running stays listed.
pub const RECENT_MS: u64 = 3 * 24 * 3600 * 1000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeLive {
    pub pid: u32,
    pub session_id: Option<String>,
    pub entrypoint: Option<String>,
    /// `busy` or `idle`.
    pub status: Option<String>,
    pub status_updated_at: Option<u64>,
}

impl ClaudeLive {
    pub fn busy(&self) -> Option<bool> {
        match self.status.as_deref() {
            Some("busy") => Some(true),
            Some("idle") => Some(false),
            _ => None,
        }
    }
}

/// The app a Claude session was started from, when it's not a terminal.
pub fn claude_source(entrypoint: Option<&str>) -> Option<&'static str> {
    match entrypoint? {
        "claude-desktop" | "claude-desktop-3p" | "remote_desktop" => Some("Claude Desktop"),
        "local-agent" | "local_agent" | "remote_cowork" => Some("Cowork"),
        _ => None,
    }
}

/// Every running Claude Code session's status file, by pid.
pub fn claude_live() -> HashMap<u32, ClaudeLive> {
    let dir = home().join(".claude/sessions");
    let Ok(entries) = std::fs::read_dir(&dir) else { return HashMap::new() };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice::<ClaudeLive>(&std::fs::read(e.path()).ok()?).ok())
        .map(|s| (s.pid, s))
        .collect()
}

/// Every Codex rollout a process holds open (the Codex app's app-server holds one per running
/// thread).
pub fn codex_open_rollouts(pid: u32) -> Vec<PathBuf> {
    let Ok(out) = crate::run::output(Command::new("/usr/sbin/lsof").args(["-p", &pid.to_string(), "-Fn"]), Duration::from_secs(3)) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix('n'))
        .filter(|p| p.contains("/.codex/sessions/") && p.ends_with(".jsonl"))
        .map(PathBuf::from)
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// A session from an app that isn't running now.
#[derive(Debug, Clone, PartialEq)]
pub struct Recent {
    /// "Claude Desktop", "Cowork" or "Codex app".
    pub source: &'static str,
    pub session_id: String,
    pub path: PathBuf,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub updated_ms: u64,
}

/// Claude Desktop and Cowork sessions changed in the last three days: session files whose entries
/// say they came from the app.
pub fn recent_claude(now_ms: u64) -> Vec<Recent> {
    let root = home().join(".claude/projects");
    let mut out = Vec::new();
    for project in list(&root) {
        for file in list(&project).into_iter().filter(|f| f.extension().is_some_and(|x| x == "jsonl")) {
            let Some(updated) = modified_ms(&file) else { continue };
            if now_ms.saturating_sub(updated) > RECENT_MS {
                continue;
            }
            if let Some(r) = claude_app_session(&file, updated) {
                out.push(r);
            }
        }
    }
    out
}

/// Reads the start of a Claude session file: its entrypoint, session id and working folder.
pub(crate) fn claude_app_session(file: &Path, updated_ms: u64) -> Option<Recent> {
    use std::io::{BufRead, BufReader};
    let reader = BufReader::new(std::fs::File::open(file).ok()?);
    for line in reader.lines().take(40).map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if let Some(entry) = v["entrypoint"].as_str() {
            let source = claude_source(Some(entry))?;
            return Some(Recent {
                source,
                session_id: v["sessionId"].as_str().map(String::from).or_else(|| stem(file))?,
                path: file.to_path_buf(),
                cwd: v["cwd"].as_str().map(String::from),
                title: None,
                updated_ms,
            });
        }
    }
    None
}

/// Codex threads started from the app (not the terminal, not `codex exec`, not subagents),
/// changed in the last three days, from Codex's own index.
pub fn recent_codex(now_ms: u64) -> Vec<Recent> {
    let db = home().join(".codex/state_5.sqlite");
    if !db.exists() {
        return Vec::new();
    }
    let since = now_ms.saturating_sub(RECENT_MS);
    let sql = format!(
        "select id, rollout_path, cwd, title, coalesce(updated_at_ms, updated_at * 1000) as updated from threads \
         where archived = 0 and coalesce(updated_at_ms, updated_at * 1000) >= {since} \
         and coalesce(thread_source, 'user') = 'user' and source not in ('cli', 'exec') and source not like '{{%' \
         order by updated desc limit 50"
    );
    let uri = format!("file:{}?mode=ro", db.display());
    let Ok(out) = crate::run::output(Command::new("/usr/bin/sqlite3").args(["-readonly", "-json", &uri, &sql]), Duration::from_secs(3))
    else {
        return Vec::new();
    };
    parse_codex_threads(&String::from_utf8_lossy(&out.stdout))
}

pub(crate) fn parse_codex_threads(json: &str) -> Vec<Recent> {
    let rows: Vec<serde_json::Value> = serde_json::from_str(json.trim()).unwrap_or_default();
    rows.into_iter()
        .filter_map(|r| {
            Some(Recent {
                source: "Codex app",
                session_id: r["id"].as_str()?.to_string(),
                path: PathBuf::from(r["rollout_path"].as_str()?),
                cwd: r["cwd"].as_str().filter(|c| !c.is_empty()).map(String::from),
                title: r["title"].as_str().filter(|t| !t.is_empty()).map(String::from),
                updated_ms: r["updated"].as_u64()?,
            })
        })
        .collect()
}

/// The working folder a Codex rollout records in its first line.
pub fn rollout_cwd(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let mut line = String::new();
    BufReader::new(std::fs::File::open(path).ok()?).read_line(&mut line).ok()?;
    let v: serde_json::Value = serde_json::from_str(&line).ok()?;
    v["payload"]["cwd"].as_str().map(String::from)
}

/// Where an app lives, for Open on its sessions.
pub fn app_bundle(source: &str) -> Option<PathBuf> {
    let candidates: &[&str] = match source {
        "Claude Desktop" | "Cowork" => &["/Applications/Claude.app"],
        "Codex app" => &["/Applications/Codex.app", "/Applications/ChatGPT Codex.app"],
        _ => &[],
    };
    candidates.iter().map(PathBuf::from).chain(candidates.iter().map(|c| home().join(c.trim_start_matches('/')))).find(|p| p.exists())
}

fn list(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

fn modified_ms(p: &Path) -> Option<u64> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64)
}

fn stem(p: &Path) -> Option<String> {
    p.file_stem().map(|s| s.to_string_lossy().into_owned())
}
