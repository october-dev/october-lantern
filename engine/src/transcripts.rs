//! Read agent state from the agents' own session files. Read-only: Lantern never writes to them.
//!
//! - Claude Code: `~/.claude/projects/<slug>/<session>.jsonl`
//! - Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (found through the process's open files)

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde_json::Value;

use crate::model::{SessionStatus, State, epoch_ms, truncate};

pub(crate) const TAIL_BYTES: u64 = 512 * 1024;
const HEAD_BYTES: u64 = 128 * 1024;
/// The inbox shows this much of the agent's last message.
pub(crate) const MESSAGE_CHARS: usize = 2000;
/// Parsed session files nobody has asked about for this long are dropped from the cache.
const CACHE_IDLE: Duration = Duration::from_secs(600);

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn file_stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
    Some((mtime, meta.len()))
}

pub fn read_range(path: &Path, from_end: bool, bytes: u64) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut buf = Vec::new();
    if from_end && len > bytes {
        f.seek(SeekFrom::Start(len - bytes)).ok()?;
        f.take(bytes).read_to_end(&mut buf).ok()?;
        // Drop the partial first line.
        let start = buf.iter().position(|&b| b == b'\n').map(|i| i + 1).unwrap_or(0);
        buf.drain(..start);
    } else {
        f.take(bytes).read_to_end(&mut buf).ok()?;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

pub fn lines(text: &str) -> impl Iterator<Item = Value> + '_ {
    text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok())
}

/// Caches parsed session files by (mtime, size) so unchanged files aren't re-read every scan.
#[derive(Default)]
pub struct Transcripts {
    parsed: HashMap<PathBuf, ((u64, u64), SessionStatus, Instant)>,
    claude_index: HashMap<String, PathBuf>,
    codex_open: HashMap<u32, (Instant, Option<PathBuf>)>,
}

impl Transcripts {
    pub fn cached(&mut self, path: &Path, parse: fn(&Path, u64) -> SessionStatus) -> Option<SessionStatus> {
        let stamp = file_stamp(path)?;
        if let Some((s, status, used)) = self.parsed.get_mut(path)
            && *s == stamp
        {
            *used = Instant::now();
            return Some(status.clone());
        }
        let status = parse(path, stamp.0);
        self.parsed.insert(path.to_path_buf(), (stamp, status.clone(), Instant::now()));
        Some(status)
    }

    /// Forgets sessions that haven't been looked at for a while, so a long-running engine that
    /// has seen many sessions doesn't keep them all.
    pub fn prune_stale(&mut self) {
        self.parsed.retain(|_, (_, _, used)| used.elapsed() < CACHE_IDLE);
        self.claude_index.retain(|_, p| p.exists());
    }

    // ---------- Claude Code ----------

    /// Finds `<session>.jsonl` under any project directory.
    pub fn claude_path_for_session(&mut self, session_id: &str) -> Option<PathBuf> {
        if let Some(p) = self.claude_index.get(session_id)
            && p.exists()
        {
            return Some(p.clone());
        }
        let projects = home().join(".claude/projects");
        let file = format!("{session_id}.jsonl");
        for entry in fs::read_dir(&projects).ok()?.flatten() {
            let candidate = entry.path().join(&file);
            if candidate.exists() {
                self.claude_index.insert(session_id.to_string(), candidate.clone());
                return Some(candidate);
            }
        }
        None
    }

    /// Best guess when the session id is unknown: the newest transcript in the cwd's project folder
    /// that changed since the process started and isn't claimed by another agent.
    pub fn claude_guess_path(&self, cwd: &Path, started_secs: u64, claimed: &[PathBuf]) -> Option<PathBuf> {
        let slug: String = cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        let dir = home().join(".claude/projects").join(slug);
        let started_ms = started_secs.saturating_sub(60) * 1000;
        fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl") && !claimed.contains(p))
            .filter_map(|p| file_stamp(&p).map(|(m, _)| (m, p)))
            .filter(|(m, _)| *m >= started_ms)
            .max_by_key(|(m, _)| *m)
            .map(|(_, p)| p)
    }

    pub fn claude_status(&mut self, path: &Path) -> Option<SessionStatus> {
        self.cached(path, parse_claude)
    }

    // ---------- Codex ----------

    /// The rollout file a Codex process has open. `lsof` is slow-ish, so results are cached briefly.
    pub fn codex_path_for_pid(&mut self, pid: u32) -> Option<PathBuf> {
        if let Some((at, path)) = self.codex_open.get(&pid)
            && at.elapsed() < Duration::from_secs(15)
        {
            return path.clone();
        }
        let path = codex_open_rollout(pid);
        self.codex_open.insert(pid, (Instant::now(), path.clone()));
        path
    }

    pub fn codex_status(&mut self, path: &Path) -> Option<SessionStatus> {
        self.cached(path, parse_codex)
    }

    pub fn forget_pids(&mut self, live: &[u32]) {
        self.codex_open.retain(|pid, _| live.contains(pid));
    }
}

fn text_blocks(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(blocks) => {
            let texts: Vec<&str> = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
            if texts.is_empty() { None } else { Some(texts.join("\n")) }
        }
        _ => None,
    }
}

fn has_block(content: &Value, kind: &str) -> bool {
    content.as_array().is_some_and(|b| b.iter().any(|b| b["type"] == kind))
}

/// Claude Code writes one JSONL entry per content block of an API message (a `text` entry, then
/// a `tool_use` entry, sharing `message.id`), and every entry carries the message's final
/// `stop_reason`. So the last entry's `stop_reason` says whether the turn is over, whatever block
/// that entry happens to hold.
pub(crate) fn parse_claude(path: &Path, mtime: u64) -> SessionStatus {
    let mut status = SessionStatus { since: Some(mtime), ..Default::default() };
    let Some(text) = read_range(path, true, TAIL_BYTES) else { return status };

    // The last user/assistant entry decides the state; the last assistant text since the
    // latest prompt is the message shown in the inbox.
    let mut last: Option<Value> = None;
    let mut reply_text: Option<String> = None;
    for entry in lines(&text) {
        if let Some(t) = entry["aiTitle"].as_str() {
            status.title = Some(t.to_string());
        }
        if let Some(id) = entry["sessionId"].as_str() {
            status.session_id = Some(id.to_string());
        }
        let kind = entry["type"].as_str().unwrap_or("");
        if !(kind == "user" || kind == "assistant") || entry["isSidechain"] == true || entry["isMeta"] == true {
            continue;
        }
        let content = &entry["message"]["content"];
        if kind == "user" && !has_block(content, "tool_result") {
            reply_text = None;
        }
        if kind == "assistant"
            && let Some(t) = text_blocks(content)
        {
            reply_text = Some(t);
        }
        last = Some(entry);
    }

    let Some(last) = last else {
        status.state = Some(State::Idle);
        return status;
    };
    if let Some(at) = last["timestamp"].as_str().and_then(epoch_ms) {
        status.since = Some(at);
    }
    let content = &last["message"]["content"];
    status.state = Some(match last["type"].as_str() {
        Some("assistant") if last["message"]["stop_reason"] == "tool_use" || has_block(content, "tool_use") => State::Working,
        Some("assistant") => State::Waiting,
        _ if has_block(content, "tool_result") => State::Working,
        _ => {
            let prompt = text_blocks(content).unwrap_or_default();
            if prompt.starts_with("[Request interrupted") { State::Waiting } else { State::Working }
        }
    });
    if status.state == Some(State::Waiting) {
        status.last_message = reply_text.map(|t| truncate(&t, MESSAGE_CHARS));
    }
    status
}

fn codex_open_rollout(pid: u32) -> Option<PathBuf> {
    let out = Command::new("/usr/sbin/lsof").args(["-p", &pid.to_string(), "-Fn"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix('n'))
        .filter(|p| p.contains("/.codex/sessions/") && p.ends_with(".jsonl"))
        .map(PathBuf::from)
        .filter_map(|p| file_stamp(&p).map(|(m, _)| (m, p)))
        .max_by_key(|(m, _)| *m)
        .map(|(_, p)| p)
}

/// Codex marks turns with `task_started` / `task_complete` events, each stamped with the turn's
/// own times. When the tail doesn't reach back to the last such event the state is unknown: a
/// long turn can write more than the tail between them.
pub(crate) fn parse_codex(path: &Path, mtime: u64) -> SessionStatus {
    let mut status = SessionStatus { since: Some(mtime), ..Default::default() };

    if let Some(head) = read_range(path, false, HEAD_BYTES) {
        for entry in lines(&head) {
            let p = &entry["payload"];
            if entry["type"] == "session_meta" {
                status.session_id = p["id"].as_str().map(String::from);
            }
            if entry["type"] == "response_item" && p["type"] == "message" && p["role"] == "user" {
                let first = p["content"].as_array().into_iter().flatten().filter_map(|b| b["text"].as_str()).find(|t| {
                    // Skip context Codex injects before the first real prompt.
                    let t = t.trim_start();
                    !t.starts_with('<') && !t.starts_with("# AGENTS.md")
                });
                if let Some(t) = first {
                    status.title = Some(truncate(t.lines().next().unwrap_or(t), 80));
                    break;
                }
            }
        }
    }

    let whole_file = fs::metadata(path).is_ok_and(|m| m.len() <= TAIL_BYTES);
    let Some(tail) = read_range(path, true, TAIL_BYTES) else { return status };
    let mut state = if whole_file { State::Idle } else { State::Unknown };
    for entry in lines(&tail) {
        if entry["type"] != "event_msg" {
            continue;
        }
        let p = &entry["payload"];
        let at = |secs: &str| p[secs].as_u64().map(|s| s * 1000).or_else(|| entry["timestamp"].as_str().and_then(epoch_ms));
        match p["type"].as_str() {
            Some("task_started") => {
                state = State::Working;
                status.last_message = None;
                status.since = at("started_at").or(status.since);
            }
            Some("task_complete") => {
                state = State::Waiting;
                status.last_message = p["last_agent_message"].as_str().map(|t| truncate(t, MESSAGE_CHARS));
                status.since = at("completed_at").or(status.since);
            }
            Some("turn_aborted") => {
                state = State::Waiting;
                status.since = at("completed_at").or(status.since);
            }
            _ => {}
        }
    }
    status.state = Some(state);
    status
}
