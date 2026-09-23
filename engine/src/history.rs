//! The conversation with an agent, read from its session file, for the chat view.
//! Read-only, like everything else that touches the agents' files.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::model::truncate;
use crate::transcripts::{lines, read_range};

/// How far back to read. Long sessions are tens of MB; the recent part is what matters here.
const TAIL_BYTES: u64 = 3 * 1024 * 1024;
const MAX_MESSAGES: usize = 120;
const MESSAGE_CHARS: usize = 8000;

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
}

fn push(out: &mut Vec<ChatMessage>, role: Role, text: &str, at: Option<&str>) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    out.push(ChatMessage { role, text: truncate(text, MESSAGE_CHARS), at: at.map(String::from) });
}

/// Text the harness adds to the user's side that the user didn't type.
fn is_injected(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with('<') || t.starts_with("# AGENTS.md") || t.starts_with("Caveat:")
}

/// A short description of a tool call: its name plus the most telling argument.
pub(crate) fn describe_tool(name: &str, input: &Value) -> String {
    if let Value::String(s) = input {
        if serde_json::from_str::<Value>(s).is_err() {
            return describe_code_call(name, s);
        }
    }
    let input = match input {
        Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
        v => v.clone(),
    };
    let detail = ["description", "command", "cmd", "file_path", "path", "pattern", "url", "query", "prompt"]
        .iter()
        .find_map(|k| match &input[*k] {
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
    let Some(text) = read_range(path, true, TAIL_BYTES) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in lines(&text) {
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
        for b in blocks {
            match (kind, b["type"].as_str()) {
                ("user", Some("text")) => {
                    let t = b["text"].as_str().unwrap_or("");
                    if !is_injected(t) {
                        push(&mut out, Role::User, t, at);
                    }
                }
                ("assistant", Some("text")) => push(&mut out, Role::Agent, b["text"].as_str().unwrap_or(""), at),
                ("assistant", Some("tool_use")) => {
                    push(&mut out, Role::Tool, &describe_tool(b["name"].as_str().unwrap_or("tool"), &b["input"]), at)
                }
                _ => {}
            }
        }
    }
    keep_recent(out)
}

pub fn codex(path: &Path) -> Vec<ChatMessage> {
    let Some(text) = read_range(path, true, TAIL_BYTES) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in lines(&text) {
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
                for b in p["content"].as_array().into_iter().flatten() {
                    let t = b["text"].as_str().unwrap_or("");
                    if matches!(role, Role::User) && is_injected(t) {
                        continue;
                    }
                    push(&mut out, role.clone(), t, at);
                }
            }
            Some("function_call") | Some("custom_tool_call") => {
                let args = if p["arguments"].is_null() { &p["input"] } else { &p["arguments"] };
                push(&mut out, Role::Tool, &describe_tool(p["name"].as_str().unwrap_or("tool"), args), at);
            }
            _ => {}
        }
    }
    keep_recent(out)
}

fn keep_recent(mut out: Vec<ChatMessage>) -> Vec<ChatMessage> {
    if out.len() > MAX_MESSAGES {
        out.drain(..out.len() - MAX_MESSAGES);
    }
    out
}
