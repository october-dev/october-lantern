//! The conversation with an agent, read from its session file, for the chat view.
//! Read-only, like everything else that touches the agents' files.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::model::truncate;
use crate::transcripts::tail_lines;

/// How far back to read. Long sessions are tens of MB; the recent part is what matters here.
pub(crate) const TAIL_BYTES: u64 = 3 * 1024 * 1024;
pub(crate) const MAX_MESSAGES: usize = 120;
pub(crate) const MESSAGE_CHARS: usize = 8000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Agent,
    /// A tool the agent ran, e.g. "Bash · npm test".
    Tool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: Role,
    pub text: String,
    /// ISO 8601, when the session file records it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// Which entry of its session this is, the same on every read: the same id with new text is
    /// that entry updated (a tool call filled in), not a new one. Unique within `History::source`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// An agent's conversation as read now, and which session it was read from.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    /// The session (file, or OpenCode session) the messages come from: a new one after `/clear`
    /// or `/new` in the same process. An opaque, stable name, not a path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub messages: Vec<ChatMessage>,
}

/// A short, stable name for a session (FNV-1a of its path or id): the same in every version.
pub(crate) fn source_id(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

pub(crate) fn push(out: &mut Vec<ChatMessage>, role: Role, text: &str, at: Option<&str>, id: Option<String>) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    out.push(ChatMessage { role, text: truncate(text, MESSAGE_CHARS), at: at.map(String::from), id });
}

/// The id of block `i` of the line at `offset`.
pub(crate) fn line_id(offset: u64, i: impl std::fmt::Display) -> Option<String> {
    Some(format!("{offset}.{i}"))
}

/// The last `MAX_MESSAGES` messages.
pub(crate) fn keep_recent(mut out: Vec<ChatMessage>) -> Vec<ChatMessage> {
    if out.len() > MAX_MESSAGES {
        out.drain(..out.len() - MAX_MESSAGES);
    }
    out
}

/// Text the harness adds to the user's side that the user didn't type.
fn is_injected(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with('<') || t.starts_with("# AGENTS.md") || t.starts_with("Caveat:")
}

/// A short description of a tool call: its name plus the most telling argument.
pub(crate) fn describe_tool(name: &str, input: &Value) -> String {
    if let Value::String(s) = input
        && serde_json::from_str::<Value>(s).is_err()
    {
        return describe_code_call(name, s);
    }
    let input = match input {
        Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
        v => v.clone(),
    };
    let detail =
        ["description", "command", "cmd", "file_path", "path", "pattern", "url", "query", "prompt"].iter().find_map(|k| match &input[*k] {
            Value::String(s) => Some(s.clone()),
            Value::Array(a) => Some(a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")),
            _ => None,
        });
    match detail {
        Some(d) => format!("{name} · {}", truncate(d.lines().next().unwrap_or(&d), 120)),
        None => name.to_string(),
    }
}

/// Newer Codex versions record tool calls as a small script, e.g.
/// `text(await tools.exec_command({cmd:"npm test", ...}))`. Show the shell command, or else the
/// tools it called.
pub(crate) fn describe_code_call(name: &str, code: &str) -> String {
    if let Some(start) = code.find("cmd:\"") {
        let rest = &code[start + 5..];
        let mut cmd = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(n) = chars.next() {
                        cmd.push(if n == 'n' { ' ' } else { n });
                    }
                }
                '"' => break,
                c => cmd.push(c),
            }
        }
        return format!("Run · {}", truncate(&cmd, 120));
    }
    let tools: Vec<&str> = code
        .split("tools.")
        .skip(1)
        .filter_map(|t| t.split(|c: char| !(c.is_alphanumeric() || c == '_')).next())
        .filter(|t| !t.is_empty() && *t != "write_stdin")
        .collect();
    if tools.is_empty() { name.to_string() } else { tools.join(", ").replace('_', " ") }
}

pub fn claude(path: &Path) -> Vec<ChatMessage> {
    let Some(entries) = tail_lines(path, TAIL_BYTES) else { return Vec::new() };
    let mut out = Vec::new();
    for (offset, entry) in entries {
        let kind = entry["type"].as_str().unwrap_or("");
        if !(kind == "user" || kind == "assistant") || entry["isSidechain"] == true || entry["isMeta"] == true {
            continue;
        }
        let at = entry["timestamp"].as_str();
        let content = &entry["message"]["content"];
        let blocks: Vec<Value> = match content {
            Value::String(s) => vec![serde_json::json!({"type": "text", "text": s})],
            Value::Array(a) => a.clone(),
            _ => continue,
        };
        for (i, b) in blocks.iter().enumerate() {
            let id = line_id(offset, i);
            match (kind, b["type"].as_str()) {
                ("user", Some("text")) => {
                    let t = b["text"].as_str().unwrap_or("");
                    if !is_injected(t) {
                        push(&mut out, Role::User, t, at, id);
                    }
                }
                ("assistant", Some("text")) => push(&mut out, Role::Agent, b["text"].as_str().unwrap_or(""), at, id),
                ("assistant", Some("tool_use")) => {
                    push(&mut out, Role::Tool, &describe_tool(b["name"].as_str().unwrap_or("tool"), &b["input"]), at, id)
                }
                _ => {}
            }
        }
    }
    keep_recent(out)
}

pub fn codex(path: &Path) -> Vec<ChatMessage> {
    let Some(entries) = tail_lines(path, TAIL_BYTES) else { return Vec::new() };
    let mut out = Vec::new();
    for (offset, entry) in entries {
        if entry["type"] != "response_item" {
            continue;
        }
        let at = entry["timestamp"].as_str();
        let p = &entry["payload"];
        match p["type"].as_str() {
            Some("message") => {
                let role = match p["role"].as_str() {
                    Some("user") => Role::User,
                    Some("assistant") => Role::Agent,
                    _ => continue,
                };
                for (i, b) in p["content"].as_array().into_iter().flatten().enumerate() {
                    let t = b["text"].as_str().unwrap_or("");
                    if matches!(role, Role::User) && is_injected(t) {
                        continue;
                    }
                    push(&mut out, role.clone(), t, at, line_id(offset, i));
                }
            }
            Some("function_call") | Some("custom_tool_call") => {
                let args = if p["arguments"].is_null() { &p["input"] } else { &p["arguments"] };
                push(&mut out, Role::Tool, &describe_tool(p["name"].as_str().unwrap_or("tool"), args), at, line_id(offset, 0));
            }
            _ => {}
        }
    }
    keep_recent(out)
}
