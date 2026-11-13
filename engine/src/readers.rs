//! Session readers for OpenCode, Pi (and the October harness, a Pi fork), Gemini CLI, Muse
//! and GitHub Copilot CLI. Like the Claude Code and Codex readers, these only read the agents'
//! own files.
//!
//! Muse and Copilot hold their session's marker open, so a running process is matched to its
//! session exactly; the rest match by working directory and start time.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::history::{Chat, Role, describe_tool, keep_recent, line_id, push};
use crate::model::{SessionStatus, State, epoch_ms, truncate};
use crate::transcripts::{MESSAGE_CHARS, TAIL_BYTES, home, lines, read_range, tail_lines};

fn mtime_ms(path: &Path) -> Option<u64> {
    Some(fs::metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

fn resolved(cwd: &Path) -> PathBuf {
    cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf())
}

fn text_of(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let texts: Vec<&str> =
                parts.iter().filter(|p| p["type"].as_str().is_none_or(|t| t == "text")).filter_map(|p| p["text"].as_str()).collect();
            if texts.is_empty() { None } else { Some(texts.join("\n")) }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// OpenCode: one SQLite database for every session (~/.local/share/opencode/opencode.db).

pub mod opencode {
    use super::*;

    fn db() -> PathBuf {
        std::env::var_os("OPENCODE_DB").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share/opencode/opencode.db"))
    }

    /// Changes whenever OpenCode writes (it uses write-ahead logging).
    pub fn stamp() -> Option<(u64, u64)> {
        let db = db();
        let wal = db.with_file_name("opencode.db-wal");
        Some((mtime_ms(&db)?, mtime_ms(&wal).unwrap_or(0)))
    }

    fn sql_str(s: &str) -> String {
        format!("'{}'", s.replace('\'', "''"))
    }

    fn query(sql: &str) -> Vec<Value> {
        query_in(&db(), sql).unwrap_or_default()
    }

    /// `None` when the query failed (busy, timed out, too large, not a database): not the same as
    /// no rows. No database at all is no rows.
    fn query_in(db: &Path, sql: &str) -> Option<Vec<Value>> {
        if !db.exists() {
            return Some(Vec::new());
        }
        crate::sqlite::query(db, sql).ok()
    }

    /// The session a process in `cwd` is working in: the top-level session there with the most
    /// recent activity, preferring ones created after the process started.
    pub fn session_for(cwd: &Path, started_secs: u64) -> Option<(String, String)> {
        session_since(&db(), cwd, started_secs.saturating_sub(60) * 1000).flatten()
    }

    /// The session there with the most recent activity, among those created or active since
    /// `since_ms`. `None` when the database couldn't be read.
    fn session_since(db: &Path, cwd: &Path, since_ms: u64) -> Option<Option<(String, String)>> {
        let dir = resolved(cwd).to_string_lossy().into_owned();
        let rows = query_in(
            db,
            &format!(
                "SELECT s.id, s.title, s.time_created AS created, \
             (SELECT MAX(m.time_updated) FROM message m WHERE m.session_id = s.id) AS last \
             FROM session s WHERE s.directory = {} AND s.parent_id IS NULL \
             ORDER BY last DESC LIMIT 10",
                sql_str(&dir)
            ),
        )?;
        // A process with no session activity since it started hasn't begun one yet (or continued an
        // old one without writing): don't show a stale session. `started_secs == 0` means "any".
        let pick = rows.iter().find(|r| r["created"].as_u64().unwrap_or(0) >= since_ms || r["last"].as_u64().unwrap_or(0) >= since_ms);
        Some(pick.and_then(|p| Some((p["id"].as_str()?.to_string(), p["title"].as_str().unwrap_or("").to_string()))))
    }

    fn parts(message_id: &str) -> Vec<Value> {
        query(&format!("SELECT data FROM part WHERE message_id = {} ORDER BY id", sql_str(message_id)))
            .into_iter()
            .filter_map(|r| r["data"].as_str().and_then(|d| serde_json::from_str(d).ok()))
            .collect()
    }

    fn first_user_text(session: &str) -> Option<String> {
        let rows = query(&format!(
            "SELECT p.data FROM part p JOIN message m ON m.id = p.message_id \
             WHERE m.session_id = {} AND json_extract(m.data, '$.role') = 'user' \
             AND json_extract(p.data, '$.type') = 'text' ORDER BY p.id LIMIT 1",
            sql_str(session)
        ));
        let part: Value = serde_json::from_str(rows.first()?["data"].as_str()?).ok()?;
        part["text"].as_str().map(|t| truncate(t.lines().next().unwrap_or(t), 80))
    }

    pub fn status(cwd: &Path, started_secs: u64) -> Option<SessionStatus> {
        let (session, title) = session_for(cwd, started_secs)?;
        let mut status = SessionStatus { session_id: Some(session.clone()), ..Default::default() };
        // "New session - <date>" is the placeholder until OpenCode generates a title.
        status.title = if title.starts_with("New session - ") || title.starts_with("Child session - ") {
            first_user_text(&session)
        } else {
            Some(title)
        };

        let rows =
            query(&format!("SELECT id, data, time_updated FROM message WHERE session_id = {} ORDER BY id DESC LIMIT 1", sql_str(&session)));
        let Some(last) = rows.first() else {
            status.state = Some(State::Idle);
            return Some(status);
        };
        status.since = last["time_updated"].as_u64();
        let data: Value = last["data"].as_str().and_then(|d| serde_json::from_str(d).ok()).unwrap_or_default();
        let parts = parts(last["id"].as_str().unwrap_or(""));
        let tool_busy = parts.iter().any(|p| p["type"] == "tool" && matches!(p["state"]["status"].as_str(), Some("pending" | "running")));
        let finish = data["finish"].as_str();
        let done = data["role"] == "assistant"
            && (!data["error"].is_null() || (finish.is_some_and(|f| f != "tool-calls" && f != "unknown") && !tool_busy));
        status.state = Some(if done { State::Waiting } else { State::Working });
        if done {
            let text: Vec<&str> = parts.iter().filter(|p| p["type"] == "text").filter_map(|p| p["text"].as_str()).collect();
            status.last_message = Some(truncate(&text.join("\n"), MESSAGE_CHARS)).filter(|t| !t.is_empty());
        }
        Some(status)
    }

    /// The session's newest messages; each part keeps its id, so a tool call filled in later
    /// (saved first as pending, with no input) is the same entry updated.
    ///
    /// Only a session this process created or wrote since it started (`started_secs`, without the
    /// minute of tolerance `status` allows): a recent session of an earlier process in the same
    /// folder isn't its conversation. A session it resumed shows once it writes to it.
    /// `None` when the database couldn't be read; a default (no session) when this process has
    /// none yet.
    pub fn history(cwd: &Path, started_secs: u64) -> Option<crate::history::History> {
        history_in(&db(), cwd, started_secs)
    }

    pub(crate) fn history_in(db: &Path, cwd: &Path, started_secs: u64) -> Option<crate::history::History> {
        history_with(db, cwd, started_secs, || {})
    }

    /// `history_in`, with `between` run after the session is chosen and before its messages are
    /// read: where a test puts OpenCode writing at the same time.
    pub(crate) fn history_with(db: &Path, cwd: &Path, started_secs: u64, between: impl FnOnce()) -> Option<crate::history::History> {
        let Some((session, _)) = session_since(db, cwd, started_secs * 1000)? else { return Some(Default::default()) };
        between();
        let s = sql_str(&session);
        // One statement, so one snapshot: the session's message count (its coverage) and the
        // window come from the same moment, and a message written meanwhile is in both or in
        // neither. The count is of messages, not of those with parts: one just created, before its
        // first part, would otherwise make a long session look whole (it's a row with no part).
        let rows = query_in(
            db,
            &format!(
                "WITH recent AS (SELECT id, data FROM message WHERE session_id = {s} ORDER BY id DESC LIMIT {LAST_MESSAGES}) \
             SELECT (SELECT COUNT(*) FROM message WHERE session_id = {s}) AS total, \
             json_extract(m.data, '$.role') AS role, json_extract(m.data, '$.time.created') AS created, \
             p.id AS pid, p.data AS part FROM recent m LEFT JOIN part p ON p.message_id = m.id \
             ORDER BY m.id, p.id"
            ),
        )?;
        // No rows: no messages at all (each recent message gives at least one row).
        let total = rows.first().and_then(|r| r["total"].as_u64()).unwrap_or(0) as usize;
        let mut out = Vec::new();
        for r in &rows {
            let Some(part) = r["part"].as_str().and_then(|d| serde_json::from_str::<Value>(d).ok()) else { continue };
            let id = r["pid"].as_str().map(String::from);
            let at = r["created"].as_u64().map(crate::model::iso);
            let at = at.as_deref();
            match (r["role"].as_str(), part["type"].as_str()) {
                (Some("user"), Some("text")) => push(&mut out, Role::User, part["text"].as_str().unwrap_or(""), at, id),
                (Some("assistant"), Some("text")) => push(&mut out, Role::Agent, part["text"].as_str().unwrap_or(""), at, id),
                (Some("assistant"), Some("tool")) => {
                    push(&mut out, Role::Tool, &describe_tool(part["tool"].as_str().unwrap_or("tool"), &part["state"]["input"]), at, id)
                }
                _ => {}
            }
        }
        let chat = Chat { messages: keep_recent(out), whole: total <= LAST_MESSAGES };
        Some(crate::history::History::of(&format!("opencode:{session}"), chat))
    }

    /// How many of a session's newest messages `history` reads.
    const LAST_MESSAGES: usize = 80;
}

// ---------------------------------------------------------------------------------------------
// Pi and the October harness: JSONL per session in <agentDir>/sessions/--<cwd>--/.

pub mod pi {
    use super::*;

    /// `october` is the October harness (a Pi fork); it keeps its data in ~/.october.
    fn agent_dir(october: bool) -> PathBuf {
        let (var, default) = if october { ("OCTOBER_CODING_AGENT_DIR", ".october/agent") } else { ("PI_CODING_AGENT_DIR", ".pi/agent") };
        std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| home().join(default))
    }

    fn sessions_dir(cwd: &Path, october: bool) -> PathBuf {
        let path = resolved(cwd).to_string_lossy().into_owned();
        let encoded: String = path.trim_start_matches('/').chars().map(|c| if matches!(c, '/' | '\\' | ':') { '-' } else { c }).collect();
        agent_dir(october).join("sessions").join(format!("--{encoded}--"))
    }

    /// The newest session file changed since the process started (`/new` switches files mid-run).
    pub fn session_file(cwd: &Path, started_secs: u64, october: bool) -> Option<PathBuf> {
        let started_ms = started_secs.saturating_sub(60) * 1000;
        fs::read_dir(sessions_dir(cwd, october))
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .filter_map(|p| mtime_ms(&p).map(|m| (m, p)))
            .filter(|(m, _)| *m >= started_ms)
            .max_by_key(|(m, _)| *m)
            .map(|(_, p)| p)
    }

    fn has_tool_call(content: &Value) -> bool {
        content.as_array().is_some_and(|c| c.iter().any(|b| b["type"] == "toolCall"))
    }

    pub fn parse(path: &Path, mtime: u64) -> SessionStatus {
        let mut status = SessionStatus { since: Some(mtime), ..Default::default() };
        if let Some(head) = read_range(path, false, 128 * 1024) {
            for e in lines(&head) {
                if e["type"] == "session" {
                    status.session_id = e["id"].as_str().map(String::from);
                }
                if status.title.is_none() && e["type"] == "message" && e["message"]["role"] == "user" {
                    status.title = text_of(&e["message"]["content"]).map(|t| truncate(t.lines().next().unwrap_or(&t), 80));
                }
            }
        }
        let Some(tail) = read_range(path, true, TAIL_BYTES) else { return status };
        let mut last: Option<Value> = None;
        for e in lines(&tail) {
            if e["type"] == "session_info"
                && let Some(name) = e["name"].as_str()
            {
                status.title = Some(name.to_string());
            }
            if e["type"] == "message" {
                last = Some(e);
            }
        }
        let Some(e) = last else {
            status.state = Some(State::Idle);
            return status;
        };
        if let Some(at) = e["timestamp"].as_str().and_then(epoch_ms) {
            status.since = Some(at);
        }
        let m = &e["message"];
        let waiting = m["role"] == "assistant" && !has_tool_call(&m["content"]);
        status.state = Some(if waiting { State::Waiting } else { State::Working });
        if waiting {
            let text = text_of(&m["content"]).or_else(|| m["errorMessage"].as_str().map(String::from));
            status.last_message = text.map(|t| truncate(&t, MESSAGE_CHARS));
        }
        status
    }

    pub fn history(path: &Path) -> Option<Chat> {
        let (entries, whole) = tail_lines(path, crate::history::TAIL_BYTES)?;
        let mut out = Vec::new();
        for (offset, e) in entries {
            if e["type"] != "message" {
                continue;
            }
            let at = e["timestamp"].as_str();
            let m = &e["message"];
            match m["role"].as_str() {
                Some("user") => push(&mut out, Role::User, &text_of(&m["content"]).unwrap_or_default(), at, line_id(offset, 0)),
                Some("assistant") => {
                    for (i, b) in m["content"].as_array().into_iter().flatten().enumerate() {
                        let id = line_id(offset, i);
                        match b["type"].as_str() {
                            Some("text") => push(&mut out, Role::Agent, b["text"].as_str().unwrap_or(""), at, id),
                            Some("toolCall") => {
                                push(&mut out, Role::Tool, &describe_tool(b["name"].as_str().unwrap_or("tool"), &b["arguments"]), at, id)
                            }
                            _ => {}
                        }
                    }
                    if let Some(err) = m["errorMessage"].as_str() {
                        push(&mut out, Role::Agent, err, at, line_id(offset, "error"));
                    }
                }
                _ => {}
            }
        }
        Some(Chat { messages: keep_recent(out), whole })
    }
}

// ---------------------------------------------------------------------------------------------
// Gemini CLI: JSONL per session in ~/.gemini/tmp/<project>/chats/, replayed in order
// (a message id written again replaces the earlier copy).

pub mod gemini {
    use super::*;

    /// Everything after this much of a session log is ignored.
    const REPLAY_BYTES: u64 = 16 * 1024 * 1024;

    fn root() -> PathBuf {
        std::env::var_os("GEMINI_CLI_HOME").map(PathBuf::from).unwrap_or_else(home).join(".gemini")
    }

    fn project_dir(cwd: &Path) -> Option<PathBuf> {
        let cwd = resolved(cwd).to_string_lossy().into_owned();
        let tmp = root().join("tmp");
        if let Ok(bytes) = fs::read(root().join("projects.json"))
            && let Ok(v) = serde_json::from_slice::<Value>(&bytes)
            && let Some(slug) = v["projects"][&cwd].as_str()
        {
            return Some(tmp.join(slug));
        }
        fs::read_dir(&tmp)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|dir| fs::read_to_string(dir.join(".project_root")).is_ok_and(|r| r.trim() == cwd))
    }

    pub fn session_file(cwd: &Path, started_secs: u64) -> Option<PathBuf> {
        let chats = project_dir(cwd)?.join("chats");
        let started_ms = started_secs.saturating_sub(60) * 1000;
        fs::read_dir(chats)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "jsonl"))
            .filter_map(|p| mtime_ms(&p).map(|m| (m, p)))
            .filter(|(m, _)| *m >= started_ms)
            .max_by_key(|(m, _)| *m)
            .map(|(_, p)| p)
    }

    /// Replays the file into the current list of messages.
    fn messages(path: &Path) -> Vec<Value> {
        replay(path).map(|(list, _)| list).unwrap_or_default()
    }

    /// The file replayed, and whether all of it was (it's no longer than `REPLAY_BYTES`). `None`
    /// when it can't be read.
    fn replay(path: &Path) -> Option<(Vec<Value>, bool)> {
        let whole = fs::metadata(path).ok()?.len() <= REPLAY_BYTES;
        let text = read_range(path, false, REPLAY_BYTES)?;
        let mut list: Vec<Value> = Vec::new();
        for e in lines(&text) {
            if let Some(set) = e.get("$set") {
                if let Some(msgs) = set["messages"].as_array() {
                    list = msgs.clone();
                }
                continue;
            }
            if let Some(id) = e.get("$rewindTo").and_then(Value::as_str) {
                if let Some(i) = list.iter().position(|m| m["id"] == id) {
                    list.truncate(i);
                }
                continue;
            }
            let Some(id) = e["id"].as_str() else { continue };
            if e["type"].is_null() {
                continue;
            }
            match list.iter().position(|m| m["id"] == id) {
                Some(i) => list[i] = e,
                None => list.push(e),
            }
        }
        Some((list, whole))
    }

    fn content_text(m: &Value) -> String {
        text_of(&m["displayContent"]).or_else(|| text_of(&m["content"])).unwrap_or_default()
    }

    fn is_real_prompt(m: &Value) -> bool {
        let t = content_text(m);
        let t = t.trim_start();
        !t.is_empty() && !t.starts_with('<') && !t.starts_with('/') && !t.starts_with('?')
    }

    pub fn parse(path: &Path, mtime: u64) -> SessionStatus {
        let mut status = SessionStatus { since: Some(mtime), ..Default::default() };
        let list = messages(path);
        status.title = list
            .iter()
            .find(|m| m["type"] == "user" && is_real_prompt(m))
            .map(|m| truncate(content_text(m).lines().next().unwrap_or(""), 80));
        if let Some(head) = read_range(path, false, 4096) {
            status.session_id = lines(&head).next().and_then(|h| h["sessionId"].as_str().map(String::from));
        }
        // Gemini's own context message isn't a prompt; tool results (user messages without text)
        // follow a reply with tool calls, which already counts as working.
        let Some(last) = list.iter().rev().find(|m| match m["type"].as_str() {
            Some("user") => is_real_prompt(m),
            Some("gemini" | "error" | "info") => true,
            _ => false,
        }) else {
            status.state = Some(State::Idle);
            return status;
        };
        if let Some(at) = last["timestamp"].as_str().and_then(epoch_ms) {
            status.since = Some(at);
        }
        let with_tools = last["toolCalls"].as_array().is_some_and(|t| !t.is_empty());
        let state = match last["type"].as_str() {
            Some("gemini") if with_tools || content_text(last).trim().is_empty() => State::Working,
            Some("gemini") | Some("error") | Some("info") => State::Waiting,
            _ => State::Working,
        };
        status.state = Some(state);
        if state == State::Waiting {
            status.last_message = Some(truncate(&content_text(last), MESSAGE_CHARS)).filter(|t| !t.is_empty());
        }
        status
    }

    pub fn history(path: &Path) -> Option<Chat> {
        let (list, whole) = replay(path)?;
        let mut out = Vec::new();
        for m in list {
            let at = m["timestamp"].as_str();
            // Its record id: one written again (updated) replaces the earlier copy.
            let id = |part: &str| m["id"].as_str().map(|id| format!("{id}.{part}"));
            match m["type"].as_str() {
                Some("user") if is_real_prompt(&m) => push(&mut out, Role::User, &content_text(&m), at, id("text")),
                Some("gemini") => {
                    push(&mut out, Role::Agent, &content_text(&m), at, id("text"));
                    for (i, call) in m["toolCalls"].as_array().into_iter().flatten().enumerate() {
                        let name = call["name"].as_str().unwrap_or("tool");
                        push(&mut out, Role::Tool, &describe_tool(name, &call["args"]), at, id(&format!("tool{i}")));
                    }
                }
                Some("error") => push(&mut out, Role::Agent, &content_text(&m), at, id("text")),
                _ => {}
            }
        }
        Some(Chat { messages: keep_recent(out), whole })
    }
}

// ---------------------------------------------------------------------------------------------
// Muse (Meta): an event log per session, `~/.local/share/muse/sessions/YYYY/MM/DD/<id>/session.jsonl`.
// The running process holds the session's `.session.lock` open, so it's matched exactly.

pub mod muse {
    use super::*;

    fn root() -> PathBuf {
        home().join(".local/share/muse/sessions")
    }

    /// The main session (not a subagent's) whose lock the process holds.
    pub fn open_session(pid: u32) -> Option<PathBuf> {
        let root = root();
        crate::platform::open_files(pid).into_iter().find(|p| is_main_lock(&root, p)).map(|p| p.with_file_name("session.jsonl"))
    }

    /// `<root>/YYYY/MM/DD/<id>/.session.lock`; a subagent's lock is deeper.
    pub(crate) fn is_main_lock(root: &Path, path: &Path) -> bool {
        path.file_name().is_some_and(|n| n == ".session.lock") && path.strip_prefix(root).is_ok_and(|rel| rel.components().count() == 5)
    }

    /// Events are recorded in microseconds.
    fn at_ms(e: &Value) -> Option<u64> {
        e["recorded_at"].as_u64().map(|us| us / 1000)
    }

    /// The session's own event (`runtime.session`): its kind (`run`, `approval`, …) and the event.
    fn event(e: &Value) -> Option<(&str, &Value)> {
        (e["payload_type"] == "runtime.session").then(|| (e["payload"]["kind"].as_str().unwrap_or(""), &e["payload"]["event"]))
    }

    /// A run (one turn) starts with your prompt and ends with `terminal`. A tool waiting for your
    /// approval is an `approval` `requested` with no `decision_applied` yet; Muse may first have
    /// a model review it, so it only counts as waiting on you once that review didn't approve it.
    pub fn parse(path: &Path, mtime: u64) -> SessionStatus {
        let mut status = SessionStatus { since: Some(mtime), ..Default::default() };
        // The title is the first prompt, near the start of the file.
        if let Some(head) = read_range(path, false, 256 * 1024) {
            status.title = lines(&head)
                .filter_map(|e| {
                    event(&e)
                        .filter(|(k, ev)| *k == "run" && ev["kind"] == "started")
                        .and_then(|(_, ev)| ev["prompt"].as_str().map(String::from))
                })
                .next()
                .map(|p| truncate(p.lines().next().unwrap_or(&p), 80));
        }
        let Some(text) = read_range(path, true, TAIL_BYTES) else { return status };
        let mut state = State::Idle;
        let mut reply: Option<String> = None;
        // Pending approvals: id → (question, under automated review).
        let mut pending: Vec<(String, String, bool)> = Vec::new();
        for e in lines(&text) {
            if e["stream"]["kind"] == "session" && status.session_id.is_none() {
                status.session_id = e["stream"]["id"].as_str().map(String::from);
            }
            let Some((kind, ev)) = event(&e) else { continue };
            let id = ev["pending_action_id"].as_str().unwrap_or("").to_string();
            match (kind, ev["kind"].as_str().unwrap_or("")) {
                ("run", "started") => {
                    state = State::Working;
                    reply = None;
                    status.since = at_ms(&e).or(status.since);
                    if status.title.is_none()
                        && let Some(p) = ev["prompt"].as_str()
                    {
                        status.title = Some(truncate(p.lines().next().unwrap_or(p), 80));
                    }
                }
                ("run", "assistant_message_committed") => reply = ev["text"].as_str().map(String::from),
                ("run", "terminal") => {
                    state = State::Waiting;
                    pending.clear();
                    status.since = at_ms(&e).or(status.since);
                }
                ("approval", "requested") => {
                    let tool = ev["tool_name"].as_str().unwrap_or("a tool");
                    let args = ev["raw_args"].as_str().unwrap_or("");
                    let q = if args.is_empty() { format!("Allow {tool}?") } else { format!("Allow {tool}: {}?", truncate(args, 200)) };
                    pending.push((id, q, false));
                }
                ("approval", "automated_review_started") => pending.iter_mut().filter(|p| p.0 == id).for_each(|p| p.2 = true),
                ("approval", "automated_review_completed") if ev["status"] != "approved" => {
                    pending.iter_mut().filter(|p| p.0 == id).for_each(|p| p.2 = false)
                }
                ("approval", "decision_applied") => pending.retain(|p| p.0 != id),
                _ => {}
            }
        }
        if let Some((_, question, _)) = pending.iter().find(|p| !p.2) {
            state = State::NeedsInput;
            status.question = Some(question.clone());
            // Muse's approval keys aren't Claude Code's: answer it in its terminal.
            status.question_kind = Some(crate::model::QuestionKind::Other);
        }
        if state == State::Waiting {
            status.last_message = reply.map(|t| truncate(&t, MESSAGE_CHARS));
        }
        status.state = Some(state);
        status
    }

    pub fn history(path: &Path) -> Option<Chat> {
        let (entries, whole) = tail_lines(path, crate::history::TAIL_BYTES)?;
        let mut out = Vec::new();
        for (offset, e) in entries {
            let Some(("run", ev)) = event(&e) else { continue };
            let at = at_ms(&e).map(crate::model::iso);
            let at = at.as_deref();
            match ev["kind"].as_str() {
                Some("started") => push(&mut out, Role::User, ev["prompt"].as_str().unwrap_or(""), at, line_id(offset, 0)),
                Some("assistant_message_committed") => {
                    push(&mut out, Role::Agent, ev["text"].as_str().unwrap_or(""), at, line_id(offset, 0))
                }
                Some("assistant_tool_calls_committed") => {
                    for (i, call) in ev["tool_calls"].as_array().into_iter().flatten().enumerate() {
                        let name = call["name"].as_str().unwrap_or("tool");
                        push(&mut out, Role::Tool, &describe_tool(name, &call["args"]), at, line_id(offset, i));
                    }
                }
                _ => {}
            }
        }
        Some(Chat { messages: keep_recent(out), whole })
    }
}

// ---------------------------------------------------------------------------------------------
// GitHub Copilot CLI: one directory per session, `$COPILOT_HOME/session-state/<id>/`
// (`~/.copilot` when unset), holding the session's append-only event log (`events.jsonl`)
// and its metadata (`workspace.yaml`, with the session's working directory). A resumed
// session appends to the same log, so one session is always one file. The running process
// holds `inuse.<pid>.hold` in its session directory open and removes it on exit, so it's
// matched exactly; otherwise the newest session for the process's folder is guessed.
//
// Each line is an event envelope, `{id, timestamp, parentId, type, data}` with an optional
// `agentId` (a subagent's traffic: its parent's tool plumbing, skipped) and `ephemeral`
// (never written to disk). The envelope is pinned by the `session-events.schema.json` the
// CLI ships with every install; new event types arrive with releases, so anything outside
// the few types read here is ignored. The CLI never persists its idle events
// (`assistant.idle` and `session.idle` are const-`ephemeral` in that schema), so waiting is
// read the way Goose reads it: a turn that ended with no tool call still open and the last
// word a reply. An unanswered `permission.requested` is waiting on you instead.

pub mod copilot {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// Where the CLI keeps its files: `$COPILOT_HOME` replaces the whole path when set (a
    /// relative one resolves against the agent's working directory, as the CLI does).
    pub fn root_for(cwd: &Path) -> PathBuf {
        match std::env::var_os("COPILOT_HOME").map(PathBuf::from) {
            Some(dir) if dir.is_absolute() => dir,
            Some(dir) => cwd.join(dir),
            None => home().join(".copilot"),
        }
    }

    fn root() -> PathBuf {
        root_for(&std::env::current_dir().unwrap_or_else(|_| home()))
    }

    /// The `workspace.yaml` keys this reader needs. The file is a flat `key: value` map.
    pub(crate) struct Workspace {
        pub(crate) cwd: Option<String>,
        pub(crate) name: Option<String>,
        pub(crate) updated_secs: u64,
    }

    pub(crate) fn read_workspace(path: &Path) -> Option<Workspace> {
        let mut ws = Workspace { cwd: None, name: None, updated_secs: 0 };
        for line in fs::read_to_string(path).ok()?.lines() {
            let Some((key, value)) = line.split_once(':') else { continue };
            let value = unquote(value.trim());
            match key.trim() {
                "cwd" => ws.cwd = Some(value),
                "name" => ws.name = Some(value),
                "updated_at" => ws.updated_secs = epoch_ms(&value).map(|ms| ms / 1000).unwrap_or(0),
                _ => {}
            }
        }
        Some(ws)
    }

    /// One pair of matching quotes off, when the value has them.
    fn unquote(value: &str) -> String {
        let quoted = value.len() >= 2 && value.starts_with(['\'', '"']) && value.ends_with(value.chars().next().unwrap_or('"'));
        if quoted { value[1..value.len() - 1].to_string() } else { value.to_string() }
    }

    /// The session `pid` has open: the CLI holds `inuse.<pid>.hold` in its session directory
    /// open while it runs. Several matches (a crashed run's leftover plus a live one after a
    /// pid reuse) resolve to the most recently written log.
    pub fn session_for_pid_in(root: &Path, pid: u32) -> Option<PathBuf> {
        let want = format!("inuse.{pid}.hold");
        let mut best: Option<(u64, PathBuf)> = None;
        for entry in fs::read_dir(root.join("session-state")).ok()?.flatten() {
            let log = entry.path().join("events.jsonl");
            if entry.path().join(&want).exists() && log.is_file() {
                let stamp = mtime_ms(&log).unwrap_or(0);
                if best.as_ref().is_none_or(|(ms, _)| stamp >= *ms) {
                    best = Some((stamp, log));
                }
            }
        }
        best.map(|(_, log)| log)
    }

    pub fn session_for_pid(pid: u32) -> Option<PathBuf> {
        session_for_pid_in(&root(), pid)
    }

    /// The newest session for `cwd` with activity since the process started (a minute's
    /// tolerance, like the other readers): the fallback where no in-use marker names the pid.
    /// Recency comes from the log, whose appends outrun the metadata's `updated_at`.
    pub fn session_file_in(root: &Path, cwd: &Path, started_secs: u64) -> Option<PathBuf> {
        let want = resolved(cwd);
        let since = started_secs.saturating_sub(60);
        let mut best: Option<(u64, PathBuf)> = None;
        for entry in fs::read_dir(root.join("session-state")).ok()?.flatten() {
            let log = entry.path().join("events.jsonl");
            if !log.is_file() {
                continue;
            }
            let Some(ws) = read_workspace(&entry.path().join("workspace.yaml")) else { continue };
            let Some(wd) = ws.cwd.as_deref() else { continue };
            if resolved(Path::new(wd)) != want {
                continue;
            }
            let stamp = mtime_ms(&log).unwrap_or(0);
            if stamp / 1000 < since && ws.updated_secs < since {
                continue;
            }
            if best.as_ref().is_none_or(|(ms, _)| stamp >= *ms) {
                best = Some((stamp, log));
            }
        }
        best.map(|(_, log)| log)
    }

    pub fn session_file(cwd: &Path, started_secs: u64) -> Option<PathBuf> {
        session_file_in(&root_for(cwd), cwd, started_secs)
    }

    /// From the root agent (no `agentId`): a subagent's prompts and answers are the parent
    /// turn's tool plumbing, not the conversation.
    fn root_event(e: &Value) -> bool {
        e.get("agentId").is_none_or(|v| v.is_null())
    }

    fn at_ms(e: &Value) -> Option<u64> {
        e["timestamp"].as_str().and_then(epoch_ms)
    }

    /// What the pending permission prompt asks, from its request's kind.
    fn permission_question(data: &Value) -> String {
        let req = &data["permissionRequest"];
        let what = match req["kind"].as_str() {
            Some("shell") => req["fullCommandText"].as_str().unwrap_or("a command").to_string(),
            Some("write") => format!("writing {}", req["fileName"].as_str().unwrap_or("a file")),
            Some("read") => format!("reading {}", req["path"].as_str().unwrap_or("a file")),
            Some("mcp") => req["toolName"].as_str().unwrap_or("a tool").to_string(),
            Some("url") => req["url"].as_str().unwrap_or("a URL").to_string(),
            _ => "this action".to_string(),
        };
        format!("Allow {}?", truncate(&what, 200))
    }

    /// A session's state from its event log: your prompt, a turn, or a tool call starting
    /// means working; a turn ending with nothing open means waiting with the last reply; a
    /// permission request with no answer yet means waiting on you. A log with no recognized
    /// event (a format newer than this reader) is unknown, never misread.
    pub fn parse(path: &Path, mtime: u64) -> SessionStatus {
        let mut status = SessionStatus { since: Some(mtime), ..Default::default() };
        status.session_id = path.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned());
        if let Some(dir) = path.parent()
            && let Some(ws) = read_workspace(&dir.join("workspace.yaml"))
            && let Some(name) = ws.name.filter(|n| !n.trim().is_empty())
        {
            status.title = Some(truncate(name.lines().next().unwrap_or(&name), 80));
        }
        if status.title.is_none()
            && let Some(head) = read_range(path, false, 256 * 1024)
        {
            status.title = lines(&head)
                .filter(|e| root_event(e) && e["type"] == "user.message")
                .filter_map(|e| e["data"]["content"].as_str().map(String::from))
                .next()
                .map(|p| truncate(p.lines().next().unwrap_or(&p), 80));
        }
        let Some(text) = read_range(path, true, TAIL_BYTES) else { return status };
        let mut state = State::Idle;
        let mut reply: Option<String> = None;
        let mut anchored = false;
        let mut recognized = false;
        let mut open: HashSet<String> = HashSet::new();
        // Pending permission prompts: request id → (question, asked at).
        let mut pending: HashMap<String, (String, u64)> = HashMap::new();
        for e in lines(&text) {
            if !root_event(&e) {
                continue;
            }
            let data = &e["data"];
            let at = at_ms(&e);
            match e["type"].as_str() {
                Some("user.message") => {
                    recognized = true;
                    state = State::Working;
                    reply = None;
                    anchored = false;
                    status.since = at.or(status.since);
                }
                Some("assistant.message") => {
                    recognized = true;
                    if let Some(text) = data["content"].as_str().filter(|t| !t.trim().is_empty()) {
                        reply = Some(text.to_string());
                        status.since = at.or(status.since);
                    }
                    if data["toolRequests"].as_array().is_some_and(|r| !r.is_empty()) {
                        state = State::Working;
                        anchored = false;
                        status.since = at.or(status.since);
                    }
                }
                Some("assistant.turn_start") => {
                    recognized = true;
                    state = State::Working;
                    anchored = false;
                    status.since = at.or(status.since);
                }
                Some("assistant.turn_end") => {
                    recognized = true;
                    if open.is_empty() {
                        state = State::Waiting;
                        status.since = at.or(status.since);
                    }
                }
                Some("tool.execution_start") => {
                    recognized = true;
                    if let Some(id) = data["toolCallId"].as_str() {
                        open.insert(id.to_string());
                    }
                    state = State::Working;
                    anchored = false;
                    status.since = at.or(status.since);
                }
                Some("tool.execution_complete") => {
                    recognized = true;
                    if let Some(id) = data["toolCallId"].as_str() {
                        open.remove(id);
                    }
                }
                Some("permission.requested") => {
                    recognized = true;
                    if let Some(id) = data["requestId"].as_str() {
                        pending.insert(id.to_string(), (permission_question(data), at.unwrap_or(0)));
                    }
                }
                Some("permission.completed") => {
                    recognized = true;
                    if let Some(id) = data["requestId"].as_str() {
                        pending.remove(id);
                    }
                }
                Some("session.task_complete") => {
                    recognized = true;
                    if let Some(summary) = data["summary"].as_str().filter(|t| !t.trim().is_empty()) {
                        reply = Some(summary.to_string());
                    }
                    if open.is_empty() {
                        state = State::Waiting;
                        anchored = true;
                        status.since = at.or(status.since);
                    }
                }
                // Lifecycle bookkeeping: recognized, but says nothing about this turn.
                Some("session.start" | "session.resume" | "session.shutdown") => recognized = true,
                _ => {}
            }
        }
        if !recognized {
            status.state = Some(State::Unknown);
            return status;
        }
        if let Some((_, (question, asked))) = pending.iter().max_by_key(|(_, (_, at))| at) {
            status.state = Some(State::NeedsInput);
            status.question = Some(question.clone());
            // Copilot's approval keys aren't Lantern's: answer it in its terminal.
            status.question_kind = Some(crate::model::QuestionKind::Other);
            status.since = Some(*asked).filter(|at| *at > 0).or(status.since);
        } else if !open.is_empty() {
            status.state = Some(State::Working);
        } else {
            // A turn that ended with no reply yet is about to say more (or the rest of it
            // hasn't reached the log); a completed task with nothing to show still finished.
            if state == State::Waiting && reply.as_ref().is_none_or(|r| r.trim().is_empty()) && !anchored {
                state = State::Working;
            }
            if state == State::Waiting {
                status.last_message = reply.map(|t| truncate(&t, MESSAGE_CHARS));
            }
            status.state = Some(state);
        }
        status
    }

    pub fn history(path: &Path) -> Option<Chat> {
        let (entries, whole) = tail_lines(path, crate::history::TAIL_BYTES)?;
        let mut out = Vec::new();
        for (offset, e) in entries {
            if !root_event(&e) {
                continue;
            }
            let at = at_ms(&e).map(crate::model::iso);
            let at = at.as_deref();
            let data = &e["data"];
            match e["type"].as_str() {
                // `content` is what was said; `transformedContent` wraps it for the model.
                Some("user.message") => {
                    if let Some(text) = data["content"].as_str().filter(|t| !t.trim().is_empty()) {
                        push(&mut out, Role::User, text, at, line_id(offset, 0));
                    }
                }
                Some("assistant.message") => {
                    if let Some(text) = data["content"].as_str().filter(|t| !t.trim().is_empty()) {
                        push(&mut out, Role::Agent, text, at, line_id(offset, 0));
                    }
                    for (i, call) in data["toolRequests"].as_array().into_iter().flatten().enumerate() {
                        let name = call["name"].as_str().unwrap_or("tool");
                        push(&mut out, Role::Tool, &describe_tool(name, &call["arguments"]), at, line_id(offset, i + 1));
                    }
                }
                _ => {}
            }
        }
        Some(Chat { messages: keep_recent(out), whole })
    }
}
