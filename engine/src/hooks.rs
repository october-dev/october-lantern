//! Optional hooks for exact agent state.
//!
//! - `lantern-engine hook claude` is a Claude Code hook command (reads the hook JSON on stdin).
//! - `lantern-engine hook codex <json>` is a Codex `notify` program (JSON as the last argument).
//!
//! Each writes the latest event for its session to the events folder, which the scanner reads.
//! `hooks install` / `uninstall` edit `~/.claude/settings.json` and `~/.codex/config.toml`,
//! backing both up first.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::history::describe_tool;
use crate::model::{QuestionKind, SessionStatus, State, truncate};
use crate::transcripts::home;

pub fn support_dir() -> PathBuf {
    home().join("Library/Application Support/October Lantern")
}

pub fn events_dir() -> PathBuf {
    support_dir().join("events")
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The latest thing a hook said about a session, already reduced to Lantern's vocabulary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEvent {
    pub source: String,
    pub state: State,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    /// The agent's last message (Stop / agent-turn-complete).
    pub message: Option<String>,
    /// What it's asking, for `NeedsInput`.
    pub question: Option<String>,
    pub question_kind: Option<QuestionKind>,
    pub transcript_path: Option<String>,
    /// Process ids above the hook command, nearest first. The agent is one of them.
    pub ancestors: Vec<u32>,
    pub at: u64,
}

impl HookEvent {
    pub fn status(&self) -> SessionStatus {
        SessionStatus {
            state: Some(self.state),
            since: Some(self.at),
            question: self.question.clone(),
            question_kind: self.question_kind,
            last_message: self.message.clone(),
            session_id: self.session_id.clone(),
            title: None,
        }
    }
}

fn event_path(source: &str, session_id: Option<&str>, ancestors: &[u32]) -> PathBuf {
    let id = session_id.map(String::from).unwrap_or_else(|| format!("pid{}", ancestors.first().unwrap_or(&0)));
    let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    events_dir().join(format!("{source}-{safe}.json"))
}

fn write_event(ev: &HookEvent) -> Result<()> {
    fs::create_dir_all(events_dir())?;
    write_atomic(&event_path(&ev.source, ev.session_id.as_deref(), &ev.ancestors), &serde_json::to_vec(ev)?)
}

fn read_event(path: &Path) -> Option<HookEvent> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Entry point for `lantern-engine hook <source> [json]`. Never fails loudly: a broken hook
/// must not get in the agent's way.
pub fn run_hook(source: &str, arg: Option<String>) {
    let result = (|| -> Result<()> {
        match source {
            "claude" => {
                let mut input = String::new();
                std::io::stdin().read_to_string(&mut input)?;
                let v: Value = serde_json::from_str(&input)?;
                if let Some(ev) = claude_event(&v) {
                    write_event(&ev)?;
                }
                Ok(())
            }
            "codex" => {
                let raw = arg.context("codex notify passes JSON as an argument")?;
                chain_previous_codex_notify(&raw);
                let v: Value = serde_json::from_str(&raw)?;
                let state = if v["type"] == "agent-turn-complete" { State::Waiting } else { State::Unknown };
                write_event(&HookEvent {
                    source: "codex".into(),
                    state,
                    session_id: v["thread-id"].as_str().map(String::from),
                    cwd: v["cwd"].as_str().map(String::from),
                    message: v["last-assistant-message"].as_str().map(|t| truncate(t, 2000)),
                    question: None,
                    question_kind: None,
                    transcript_path: None,
                    ancestors: crate::procs::ancestors_of(std::process::id()),
                    at: now_ms(),
                })
            }
            other => bail!("unknown hook source {other}"),
        }
    })();
    if let Err(e) = result {
        eprintln!("lantern hook: {e:#}");
    }
}

/// Reduces one Claude Code hook payload to an event, or `None` for events that say nothing about
/// whether the session needs you.
fn claude_event(v: &Value) -> Option<HookEvent> {
    let ancestors = crate::procs::ancestors_of(std::process::id());
    let session_id = v["session_id"].as_str().map(String::from);
    let (state, question, question_kind, message) = match v["hook_event_name"].as_str().unwrap_or("") {
        // The moment Claude asks, with the exact tool and command.
        "PermissionRequest" => {
            // The command itself, when there is one: that's what the person is approving.
            let tool = v["tool_name"].as_str().unwrap_or("a tool");
            let what = match v["tool_input"]["command"].as_str() {
                Some(c) => format!("{tool} · {}", truncate(c.lines().next().unwrap_or(c), 200)),
                None => describe_tool(tool, &v["tool_input"]),
            };
            (State::NeedsInput, Some(format!("Permission to run {what}")), Some(QuestionKind::Permission), None)
        }
        "Notification" => match v["notification_type"].as_str().unwrap_or("") {
            // Fires only after the prompt has waited ~6 s; PermissionRequest already covered it,
            // unless this is a prompt PermissionRequest doesn't fire for (a sandboxed network request).
            "permission_prompt" => {
                let already = read_event(&event_path("claude", session_id.as_deref(), &ancestors))
                    .is_some_and(|e| e.state == State::NeedsInput && e.question_kind == Some(QuestionKind::Permission));
                if already {
                    return None;
                }
                (State::NeedsInput, v["message"].as_str().map(String::from), Some(QuestionKind::Permission), None)
            }
            "idle_prompt" => (State::Waiting, None, None, None),
            "elicitation_dialog" | "elicitation_url_dialog" | "agent_needs_input" => {
                (State::NeedsInput, v["message"].as_str().map(String::from), Some(QuestionKind::Other), None)
            }
            // Sign-in, elicitation bookkeeping, quota notices: nothing about the turn.
            _ => return None,
        },
        "Stop" => (State::Waiting, None, None, v["last_assistant_message"].as_str().map(|t| truncate(t, 2000))),
        // UserPromptSubmit, PostToolUse: Claude is at work.
        _ => (State::Working, None, None, None),
    };
    Some(HookEvent {
        source: "claude".into(),
        state,
        session_id,
        cwd: v["cwd"].as_str().map(String::from),
        message,
        question,
        question_kind,
        transcript_path: v["transcript_path"].as_str().map(String::from),
        ancestors,
        at: now_ms(),
    })
}

/// Latest hook event per session, dropping files older than a day.
pub fn read_events() -> Vec<HookEvent> {
    let Ok(dir) = fs::read_dir(events_dir()) else { return Vec::new() };
    let cutoff = now_ms().saturating_sub(24 * 3600 * 1000);
    dir.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let ev = read_event(&e.path())?;
            if ev.at < cutoff {
                let _ = fs::remove_file(e.path());
                return None;
            }
            Some(ev)
        })
        .collect()
}

// ---------- install / uninstall ----------

const CLAUDE_EVENTS: [&str; 5] = ["PermissionRequest", "Notification", "Stop", "UserPromptSubmit", "PostToolUse"];

fn hook_binary() -> PathBuf {
    support_dir().join("bin/lantern-engine")
}

/// Hooks run Lantern's own copy of the engine, so moving, updating or deleting the app never
/// leaves the agents calling a missing program.
fn install_hook_binary() -> Result<PathBuf> {
    let dest = hook_binary();
    fs::create_dir_all(dest.parent().unwrap())?;
    let exe = std::env::current_exe()?.canonicalize()?;
    let tmp = dest.with_extension("new");
    let _ = fs::remove_file(&tmp);
    fs::copy(&exe, &tmp)?;
    fs::rename(&tmp, &dest)?;
    Ok(dest)
}

/// Keeps the hooks' copy of the engine in step with the app (called when the app starts).
pub fn refresh_hook_binary() {
    let dest = hook_binary();
    if !dest.exists() {
        return;
    }
    let Ok(exe) = std::env::current_exe().and_then(|e| e.canonicalize()) else { return };
    if exe == dest {
        return;
    }
    if fs::read(&exe).ok() != fs::read(&dest).ok() {
        let _ = install_hook_binary();
    }
}

/// The Claude Code hook command: does nothing (successfully) if Lantern has been removed.
pub(crate) fn claude_hook_command(engine: &Path) -> String {
    let q = shell_quote(engine);
    format!("[ -x {q} ] && {q} hook claude; exit 0")
}

/// The Codex `notify` program. Codex appends the event JSON as the last argument, which becomes $1.
pub(crate) fn codex_notify_command(engine: &Path) -> Vec<String> {
    let q = shell_quote(engine);
    vec!["/bin/sh".into(), "-c".into(), format!("[ -x {q} ] && exec {q} hook codex \"$1\"; exit 0"), "lantern-engine-notify".into()]
}

fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', r"'\''"))
}

fn is_ours(command: &str) -> bool {
    command.contains("lantern-engine") && command.contains(" hook ")
}

/// Writes through a temporary file in the same folder, so a crash mid-write can't leave a
/// half-written config, and keeps the original file's permissions.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("{}.lantern-tmp", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
    fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    if let Ok(meta) = fs::metadata(path) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

fn backup(path: &Path) -> Result<()> {
    if path.exists() {
        let dest = path.with_file_name(format!("{}.lantern-backup-{}", path.file_name().unwrap().to_string_lossy(), now_ms()));
        fs::copy(path, &dest).with_context(|| format!("backing up {}", path.display()))?;
        println!("backed up {} → {}", path.display(), dest.display());
    }
    Ok(())
}

fn claude_settings_path() -> PathBuf {
    home().join(".claude/settings.json")
}

fn codex_config_path() -> PathBuf {
    home().join(".codex/config.toml")
}

fn codex_previous_path() -> PathBuf {
    support_dir().join("codex-notify-previous.json")
}

fn chain_previous_codex_notify(raw: &str) {
    let Ok(bytes) = fs::read(codex_previous_path()) else { return };
    let Ok(prev) = serde_json::from_slice::<Vec<String>>(&bytes) else { return };
    if let Some((prog, args)) = prev.split_first() {
        let _ = std::process::Command::new(prog).args(args).arg(raw).spawn();
    }
}

fn remove_claude_hooks(settings: &mut Value) -> usize {
    let mut removed = 0;
    let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) else { return 0 };
    for groups in hooks.values_mut() {
        let Some(groups) = groups.as_array_mut() else { continue };
        for group in groups.iter_mut() {
            if let Some(list) = group.get_mut("hooks").and_then(|l| l.as_array_mut()) {
                let before = list.len();
                list.retain(|h| !h["command"].as_str().is_some_and(is_ours));
                removed += before - list.len();
            }
        }
        groups.retain(|g| g["hooks"].as_array().is_none_or(|l| !l.is_empty()));
    }
    hooks.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
    removed
}

fn codex_notify(doc: &toml_edit::DocumentMut) -> Option<Vec<String>> {
    doc.get("notify")?.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
}

/// Adds Lantern to both agents' configs. Both files are read and checked before either is
/// changed, so a broken config in one leaves the other untouched.
pub fn install() -> Result<()> {
    let claude_path = claude_settings_path();
    let mut settings: Value = match fs::read(&claude_path) {
        Ok(b) => serde_json::from_slice(&b).context("~/.claude/settings.json is not valid JSON")?,
        Err(_) => json!({}),
    };
    settings.as_object().context("~/.claude/settings.json is not a JSON object")?;
    let codex_path = codex_config_path();
    let mut doc: toml_edit::DocumentMut =
        fs::read_to_string(&codex_path).unwrap_or_default().parse().context("~/.codex/config.toml is not valid TOML")?;

    let engine = install_hook_binary()?;

    // Claude Code
    backup(&claude_path)?;
    remove_claude_hooks(&mut settings);
    let command = claude_hook_command(&engine);
    let hooks = settings.as_object_mut().unwrap().entry("hooks").or_insert_with(|| json!({}));
    for event in CLAUDE_EVENTS {
        let list = hooks.as_object_mut().context("settings.hooks is not an object")?.entry(event).or_insert_with(|| json!([]));
        list.as_array_mut()
            .context("hook list is not an array")?
            .push(json!({"hooks": [{"type": "command", "command": command, "timeout": 5}]}));
    }
    fs::create_dir_all(claude_path.parent().unwrap())?;
    write_atomic(&claude_path, (serde_json::to_string_pretty(&settings)? + "\n").as_bytes())?;
    println!("Claude Code: added Lantern to {} hooks in {}", CLAUDE_EVENTS.join(", "), claude_path.display());

    // Codex
    if let Some(prev) = codex_notify(&doc)
        && !prev.iter().any(|a| a.contains("lantern-engine"))
    {
        fs::write(codex_previous_path(), serde_json::to_vec(&prev)?)?;
        println!("Codex: your existing notify program will still run: {}", prev.join(" "));
    }
    backup(&codex_path)?;
    let mut arr = toml_edit::Array::new();
    for part in codex_notify_command(&engine) {
        arr.push(part);
    }
    doc.insert("notify", toml_edit::value(arr));
    fs::create_dir_all(codex_path.parent().unwrap())?;
    write_atomic(&codex_path, doc.to_string().as_bytes())?;
    println!("Codex: set notify in {}", codex_path.display());
    println!("Restart running agents for the hooks to take effect.");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let path = claude_settings_path();
    if let Ok(b) = fs::read(&path) {
        let mut settings: Value = serde_json::from_slice(&b)?;
        let removed = remove_claude_hooks(&mut settings);
        if removed > 0 {
            backup(&path)?;
            write_atomic(&path, (serde_json::to_string_pretty(&settings)? + "\n").as_bytes())?;
        }
        println!("Claude Code: removed {removed} Lantern hook(s)");
    }

    let path = codex_config_path();
    if let Ok(text) = fs::read_to_string(&path) {
        let mut doc: toml_edit::DocumentMut = text.parse()?;
        if codex_notify(&doc).is_some_and(|n| n.iter().any(|a| a.contains("lantern-engine"))) {
            backup(&path)?;
            let prev: Option<Vec<String>> = fs::read(codex_previous_path()).ok().and_then(|b| serde_json::from_slice(&b).ok());
            match prev {
                Some(prev) => {
                    let mut arr = toml_edit::Array::new();
                    for a in &prev {
                        arr.push(a.as_str());
                    }
                    doc.insert("notify", toml_edit::value(arr));
                    println!("Codex: restored your previous notify program");
                }
                None => {
                    doc.remove("notify");
                    println!("Codex: removed notify");
                }
            }
            write_atomic(&path, doc.to_string().as_bytes())?;
            let _ = fs::remove_file(codex_previous_path());
        } else {
            println!("Codex: Lantern is not installed");
        }
    }
    Ok(())
}

/// Removes everything Lantern put on this machine outside the app: hooks (restoring previous
/// settings), the hooks' engine copy, hook events and launch scripts.
pub fn uninstall_everything() -> Result<()> {
    uninstall()?;
    let dir = support_dir();
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        println!("Removed {}", dir.display());
    }
    Ok(())
}

pub fn status() -> Result<()> {
    let claude = fs::read(claude_settings_path())
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .map(|s| {
            let hooked = |event: &str| {
                s["hooks"][event]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|g| g["hooks"].as_array().into_iter().flatten().any(|h| h["command"].as_str().is_some_and(is_ours)))
            };
            CLAUDE_EVENTS.iter().all(|e| hooked(e))
        })
        .unwrap_or(false);
    let codex = fs::read_to_string(codex_config_path())
        .ok()
        .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|d| codex_notify(&d))
        .is_some_and(|n| n.iter().any(|a| a.contains("lantern-engine")));
    println!("{}", json!({"claude": claude, "codex": codex}));
    Ok(())
}
