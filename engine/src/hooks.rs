//! Optional hooks for exact agent state.
//!
//! - `lantern-engine hook claude` is a Claude Code hook command (reads the hook JSON on stdin).
//! - `lantern-engine hook codex <json>` is a Codex `notify` program (JSON as the last argument).
//!
//! Each writes the latest event for its session to the events folder, which the scanner reads.
//! `hooks install` / `uninstall` edit `~/.claude/settings.json` and `~/.codex/config.toml`,
//! backing both up first.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use crate::model::{SessionStatus, State, truncate};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEvent {
    pub source: String,
    pub event: String,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub message: Option<String>,
    pub transcript_path: Option<String>,
    /// Process ids above the hook command, nearest first. The agent is one of them.
    pub ancestors: Vec<u32>,
    pub at: u64,
}

impl HookEvent {
    pub fn status(&self) -> SessionStatus {
        let (state, question) = match (self.source.as_str(), self.event.as_str()) {
            ("claude", "Notification") => {
                let msg = self.message.clone().unwrap_or_default();
                // Claude also notifies after 60s of idling at the prompt; that's not a question.
                if msg.contains("waiting for your input") {
                    (State::Waiting, None)
                } else {
                    (State::NeedsInput, Some(msg))
                }
            }
            ("claude", "Stop") | ("codex", "agent-turn-complete") => (State::Waiting, None),
            ("claude", _) => (State::Working, None),
            _ => (State::Unknown, None),
        };
        SessionStatus {
            state: Some(state),
            since: Some(self.at),
            question,
            last_message: if self.source == "codex" { self.message.clone() } else { None },
            session_id: self.session_id.clone(),
            title: None,
        }
    }
}

fn ancestors() -> Vec<u32> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, false, ProcessRefreshKind::nothing());
    let mut out = Vec::new();
    let mut cur = sysinfo::get_current_pid().ok().and_then(|p| sys.process(p)).and_then(|p| p.parent());
    while let Some(pid) = cur {
        if pid.as_u32() <= 1 || out.len() >= 8 {
            break;
        }
        out.push(pid.as_u32());
        cur = sys.process(pid).and_then(|p| p.parent());
    }
    out
}

fn write_event(ev: &HookEvent) -> Result<()> {
    let dir = events_dir();
    fs::create_dir_all(&dir)?;
    let id = ev.session_id.clone().unwrap_or_else(|| format!("pid{}", ev.ancestors.first().unwrap_or(&0)));
    let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    let path = dir.join(format!("{}-{}.json", ev.source, safe));
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec(ev)?)?;
    fs::rename(tmp, path)?;
    Ok(())
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
                write_event(&HookEvent {
                    source: "claude".into(),
                    event: v["hook_event_name"].as_str().unwrap_or("").into(),
                    session_id: v["session_id"].as_str().map(String::from),
                    cwd: v["cwd"].as_str().map(String::from),
                    message: v["message"].as_str().map(String::from),
                    transcript_path: v["transcript_path"].as_str().map(String::from),
                    ancestors: ancestors(),
                    at: now_ms(),
                })
            }
            "codex" => {
                let raw = arg.context("codex notify passes JSON as an argument")?;
                chain_previous_codex_notify(&raw);
                let v: Value = serde_json::from_str(&raw)?;
                write_event(&HookEvent {
                    source: "codex".into(),
                    event: v["type"].as_str().unwrap_or("").into(),
                    session_id: v["thread-id"].as_str().map(String::from),
                    cwd: v["cwd"].as_str().map(String::from),
                    message: v["last-assistant-message"].as_str().map(|t| truncate(t, 2000)),
                    transcript_path: None,
                    ancestors: ancestors(),
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

/// Latest hook event per session, dropping files older than a day.
pub fn read_events() -> Vec<HookEvent> {
    let Ok(dir) = fs::read_dir(events_dir()) else { return Vec::new() };
    let cutoff = now_ms().saturating_sub(24 * 3600 * 1000);
    dir.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let ev: HookEvent = serde_json::from_slice(&fs::read(e.path()).ok()?).ok()?;
            if ev.at < cutoff {
                let _ = fs::remove_file(e.path());
                return None;
            }
            Some(ev)
        })
        .collect()
}

// ---------- install / uninstall ----------

const CLAUDE_EVENTS: [&str; 4] = ["Notification", "Stop", "UserPromptSubmit", "PostToolUse"];

/// A stable path for hook commands, so moving the app doesn't break them.
fn stable_engine_path() -> Result<PathBuf> {
    let link = support_dir().join("bin/lantern-engine");
    fs::create_dir_all(link.parent().unwrap())?;
    let exe = std::env::current_exe()?.canonicalize()?;
    let _ = fs::remove_file(&link);
    std::os::unix::fs::symlink(&exe, &link)?;
    Ok(link)
}

fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', r"'\''"))
}

fn is_ours(command: &str) -> bool {
    command.contains("lantern-engine") && command.contains(" hook ")
}

fn backup(path: &Path) -> Result<()> {
    if path.exists() {
        let dest = path.with_file_name(format!(
            "{}.lantern-backup-{}",
            path.file_name().unwrap().to_string_lossy(),
            now_ms() / 1000
        ));
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
    doc.get("notify")?
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
}

pub fn install() -> Result<()> {
    let engine = stable_engine_path()?;

    // Claude Code
    let path = claude_settings_path();
    let mut settings: Value = match fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b).context("~/.claude/settings.json is not valid JSON")?,
        Err(_) => json!({}),
    };
    backup(&path)?;
    remove_claude_hooks(&mut settings);
    let command = format!("{} hook claude", shell_quote(&engine));
    let hooks = settings
        .as_object_mut()
        .context("settings.json is not an object")?
        .entry("hooks")
        .or_insert_with(|| json!({}));
    for event in CLAUDE_EVENTS {
        let list = hooks
            .as_object_mut()
            .context("settings.hooks is not an object")?
            .entry(event)
            .or_insert_with(|| json!([]));
        list.as_array_mut()
            .context("hook list is not an array")?
            .push(json!({"hooks": [{"type": "command", "command": command, "timeout": 5}]}));
    }
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, serde_json::to_string_pretty(&settings)? + "\n")?;
    println!("Claude Code: added Lantern to {} hooks in {}", CLAUDE_EVENTS.join(", "), path.display());

    // Codex
    let path = codex_config_path();
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text.parse().context("~/.codex/config.toml is not valid TOML")?;
    if let Some(prev) = codex_notify(&doc) {
        if !prev.iter().any(|a| a.contains("lantern-engine")) {
            fs::write(codex_previous_path(), serde_json::to_vec(&prev)?)?;
            println!("Codex: your existing notify program will still run: {}", prev.join(" "));
        }
    }
    backup(&path)?;
    let mut arr = toml_edit::Array::new();
    arr.push(engine.to_string_lossy().as_ref());
    arr.push("hook");
    arr.push("codex");
    doc.insert("notify", toml_edit::value(arr));
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, doc.to_string())?;
    println!("Codex: set notify in {}", path.display());
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
            fs::write(&path, serde_json::to_string_pretty(&settings)? + "\n")?;
        }
        println!("Claude Code: removed {removed} Lantern hook(s)");
    }

    let path = codex_config_path();
    if let Ok(text) = fs::read_to_string(&path) {
        let mut doc: toml_edit::DocumentMut = text.parse()?;
        if codex_notify(&doc).is_some_and(|n| n.iter().any(|a| a.contains("lantern-engine"))) {
            backup(&path)?;
            let prev: Option<Vec<String>> =
                fs::read(codex_previous_path()).ok().and_then(|b| serde_json::from_slice(&b).ok());
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
            fs::write(&path, doc.to_string())?;
            let _ = fs::remove_file(codex_previous_path());
        } else {
            println!("Codex: Lantern is not installed");
        }
    }
    Ok(())
}

pub fn status() -> Result<()> {
    let claude = fs::read(claude_settings_path())
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .map(|s| {
            let mut found: HashMap<String, bool> = HashMap::new();
            if let Some(hooks) = s["hooks"].as_object() {
                for (event, groups) in hooks {
                    let ours = groups.as_array().into_iter().flatten().any(|g| {
                        g["hooks"].as_array().into_iter().flatten().any(|h| h["command"].as_str().is_some_and(is_ours))
                    });
                    if ours {
                        found.insert(event.clone(), true);
                    }
                }
            }
            CLAUDE_EVENTS.iter().all(|e| found.contains_key(*e))
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
