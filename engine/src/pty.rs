//! Lantern's own terminal sessions, in place of tmux on Windows.
//!
//! Each session is a `lantern-engine pty-host <name>` process (see `pty/host.rs`) that runs one
//! agent in a pseudo console (ConPTY) and outlives the app and engine, as tmux's server does. It
//! runs the agent itself, not a shell: when the agent exits the session ends, so a reply can't
//! land in a shell left behind. `lantern-engine attach <name>` shows a session in a terminal
//! window (see `pty/attach.rs`).
//!
//! The host serves a named pipe only this user can open. Each connection sends one JSON request
//! line and gets one JSON line back: `info`, `text`, `key`, `capture`, `resize`, `clients`. Two
//! requests turn the connection into a stream for an attached window: `output` (the screen, then
//! everything the agent prints) and `input` (keystrokes to the agent). Synchronous pipe handles
//! can't read and write at once, so the two directions are separate connections.
//!
//! Hosts record themselves in `<support>/pty/<name>.json`; the engine finds sessions there.

// Only Windows runs sessions; the rest of the engine reaches them through this module everywhere.
#![cfg_attr(not(windows), allow(dead_code))]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(windows)]
#[path = "pty/attach.rs"]
mod attach;
#[cfg(windows)]
#[path = "pty/host.rs"]
mod host;
#[cfg(windows)]
pub use attach::{attach, open_window};
#[cfg(windows)]
pub use host::{host, start};

/// What a host runs, written by the engine and read (then deleted) by the new host.
#[derive(Debug, Serialize, Deserialize)]
pub struct Spec {
    pub cwd: PathBuf,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// A running session, as its host recorded it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub name: String,
    pub host_pid: u32,
    pub host_start: u64,
    pub child_pid: u32,
    pub child_start: u64,
}

/// What a host says about its agent right now.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub child_pid: u32,
    pub child_start: u64,
    /// `con:<pid>` of the session's console host.
    pub child_tty: Option<String>,
    pub alive: bool,
}

pub const COLS: u16 = 200;
pub const ROWS: u16 = 50;

pub fn dir() -> PathBuf {
    crate::hooks::support_dir().join("pty")
}

/// Session names go into file and pipe names: letters, digits and dashes only.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn pipe_name(name: &str) -> String {
    let user: String = std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!(r"\\.\pipe\lantern-pty-{user}-{name}")
}

/// Sessions whose host is still the process that recorded them.
pub fn sessions() -> Vec<Session> {
    std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json") && !p.to_string_lossy().ends_with(".spec.json"))
        .filter_map(|p| serde_json::from_slice::<Session>(&std::fs::read(p).ok()?).ok())
        .filter(|s| valid_name(&s.name) && crate::procs::live(s.host_pid).is_some_and(|l| l.start == s.host_start))
        .collect()
}

/// One request to a session's host and its answer.
pub fn request(name: &str, request: &Value) -> Result<Value> {
    let mut pipe = connect(name)?;
    writeln!(pipe, "{request}")?;
    let mut line = String::new();
    BufReader::new(&pipe).read_line(&mut line)?;
    let answer: Value = serde_json::from_str(&line).context("the session didn't answer")?;
    if answer["ok"] != true {
        bail!("{}", answer["message"].as_str().unwrap_or("the session refused"));
    }
    Ok(answer)
}

/// Opens a connection to the host, waiting briefly when every pipe instance is busy.
pub fn connect(name: &str) -> Result<std::fs::File> {
    if !valid_name(name) {
        bail!("invalid session name");
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(pipe_name(name)) {
            Ok(pipe) => {
                #[cfg(windows)]
                crate::platform::verify_pipe_server(&pipe)?;
                return Ok(pipe);
            }
            // ERROR_PIPE_BUSY: the host is between connections.
            Err(e) if e.raw_os_error() == Some(231) && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e).context("the session has ended"),
        }
    }
}

pub fn info(name: &str) -> Result<Info> {
    Ok(serde_json::from_value(request(name, &json!({"op": "info"}))?)?)
}

/// Types `text` (one line), then Enter. Once the text is in, a failed Enter is uncertain.
pub fn send_text(name: &str, text: &str) -> Result<()> {
    request(name, &json!({"op": "text", "text": text}))?;
    // Return as a key press of its own, not part of the pasted text (deliver::ENTER_AFTER_PASTE).
    std::thread::sleep(crate::deliver::ENTER_AFTER_PASTE);
    request(name, &json!({"op": "key", "bytes": "\r"}))
        .map(|_| ())
        .map_err(|e| crate::deliver::Uncertain(format!("the text went in but Enter didn't ({e:#})")).into())
}

pub fn send_bytes(name: &str, bytes: &str) -> Result<()> {
    request(name, &json!({"op": "key", "bytes": bytes})).map(|_| ())
}

/// The session's screen as text.
pub fn capture(name: &str) -> Result<String> {
    Ok(request(name, &json!({"op": "capture"}))?["screen"].as_str().unwrap_or_default().to_string())
}

/// Processes with the session open in a window (`lantern-engine attach`).
pub fn clients(name: &str) -> Result<Vec<u32>> {
    let answer = request(name, &json!({"op": "clients"}))?;
    Ok(answer["pids"].as_array().into_iter().flatten().filter_map(|p| p.as_u64().map(|p| p as u32)).collect())
}

/// Quotes one argument the way Windows programs split their command line (the Microsoft C
/// runtime's rules, which Rust, Node and Python follow).
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0b', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// The command line for a program and its arguments.
pub fn command_line(program: &Path, args: &[String]) -> String {
    std::iter::once(quote_arg(&program.to_string_lossy())).chain(args.iter().map(|a| quote_arg(a))).collect::<Vec<_>>().join(" ")
}

/// The script an npm (or pnpm/yarn) `.cmd` shim runs with Node, relative to the shim's folder.
/// Running Node on it directly keeps `cmd.exe`, and its quoting rules, away from the message.
pub fn shim_script(shim: &str) -> Option<String> {
    let mut found = None;
    for prefix in ["\"%dp0%\\", "\"%~dp0\\"] {
        let mut rest = shim;
        while let Some(at) = rest.find(prefix) {
            rest = &rest[at + prefix.len()..];
            let Some(end) = rest.find('"') else { break };
            let path = &rest[..end];
            if [".js", ".mjs", ".cjs"].iter().any(|ext| path.to_ascii_lowercase().ends_with(ext)) {
                found = Some(path.to_string());
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_survive_windows_command_line_splitting() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg("two words"), "\"two words\"");
        assert_eq!(quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote_arg(r"C:\dir with space\"), r#""C:\dir with space\\""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(quote_arg("50% & more | <x>"), "\"50% & more | <x>\"");
    }

    #[test]
    fn npm_shims_name_their_script() {
        let npm = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\r\n\
            IF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n)\r\n\r\n\
            endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\cli.js\" %*\r\n";
        assert_eq!(shim_script(npm).as_deref(), Some(r"node_modules\@anthropic-ai\claude-code\cli.js"));
        let pnpm = "@SETLOCAL\r\n\"%~dp0\\node.exe\" \"%~dp0\\..\\@openai\\codex\\bin\\codex.js\" %*\r\n";
        assert_eq!(shim_script(pnpm).as_deref(), Some(r"..\@openai\codex\bin\codex.js"));
        assert_eq!(shim_script("@echo off\r\ncodex.exe %*\r\n"), None);
    }

    #[test]
    fn session_names_are_plain() {
        assert!(valid_name("claude-my-app-12345"));
        assert!(!valid_name("../x") && !valid_name("a b") && !valid_name("") && !valid_name(&"x".repeat(65)));
    }
}
