//! Which models an agent can start with, and the flag that picks one.
//!
//! Agents that can list their models are asked, through the user's login shell (so they see the
//! same accounts and config as in a terminal): Codex (`codex debug models`), Grok (`grok models`),
//! OpenCode (`opencode models`), Pi and the October harness (`--list-models`). Claude Code and
//! Gemini CLI can't list theirs but take short names for the current models, which are offered
//! instead. Any model id can also be typed in the app.

use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::launch::{program, shell};
use crate::model::Kind;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Model {
    pub id: String,
    pub label: String,
    /// The provider, for agents that offer many (October, Pi, OpenCode).
    pub group: Option<String>,
}

/// The command-line option that picks a model, for agents where Lantern knows it.
pub fn flag(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Claude
        | Kind::Codex
        | Kind::Grok
        | Kind::October
        | Kind::Pi
        | Kind::Opencode
        | Kind::Gemini
        | Kind::Qwen
        | Kind::Aider
        | Kind::Cursor
        | Kind::Copilot => Some("--model"),
        _ => None,
    }
}

/// A model id as an agent would take it: no spaces or shell-looking characters, not an option.
pub fn valid(id: &str) -> bool {
    !id.is_empty() && id.len() <= 120 && !id.starts_with('-') && id.chars().all(|c| c.is_ascii_alphanumeric() || "._:/[]~@+-".contains(c))
}

type Listed = HashMap<Kind, (Instant, Vec<Model>)>;

/// The models to offer for `kind`, remembered for ten minutes (listing can take a few seconds).
pub fn list(kind: Kind) -> Vec<Model> {
    static CACHE: Mutex<Option<Listed>> = Mutex::new(None);
    if let Some((at, models)) = CACHE.lock().unwrap().get_or_insert_with(HashMap::new).get(&kind)
        && at.elapsed() < Duration::from_secs(600)
    {
        return models.clone();
    }
    let models = discover(kind);
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(kind, (Instant::now(), models.clone()));
    models
}

fn discover(kind: Kind) -> Vec<Model> {
    let listed = match kind {
        Kind::Codex => ask(kind, "debug models").map(|o| parse_codex(&o)),
        Kind::Grok => ask(kind, "models").map(|o| parse_bullets(&o)),
        Kind::October | Kind::Pi => ask(kind, "--list-models").map(|o| parse_table(&o)),
        Kind::Opencode => ask(kind, "models").map(|o| parse_slashed(&o)),
        _ => None,
    };
    match listed.filter(|m| !m.is_empty()) {
        Some(models) => models,
        None => known(kind),
    }
}

/// Short names the agent takes for its current models.
fn known(kind: Kind) -> Vec<Model> {
    let names: &[(&str, &str)] = match kind {
        Kind::Claude => &[("fable", "Fable"), ("opus", "Opus"), ("sonnet", "Sonnet"), ("haiku", "Haiku")],
        Kind::Gemini => &[("auto", "Auto"), ("pro", "Pro"), ("flash", "Flash"), ("flash-lite", "Flash Lite")],
        _ => &[],
    };
    names.iter().map(|(id, label)| Model { id: (*id).into(), label: (*label).into(), group: None }).collect()
}

/// Runs `<agent> <args>` in the user's login shell, with a time limit.
fn ask(kind: Kind, args: &str) -> Option<String> {
    let script = format!("{} {args} 2>/dev/null", program(kind));
    let out = crate::run::output(Command::new(shell()).args(["-lic", &script]), Duration::from_secs(20)).ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `codex debug models`: JSON with the models Codex lists in its own picker, best first.
pub(crate) fn parse_codex(out: &str) -> Vec<Model> {
    let Some(start) = out.find('{') else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&out[start..]) else { return Vec::new() };
    let mut listed: Vec<&serde_json::Value> =
        v["models"].as_array().into_iter().flatten().filter(|m| m["visibility"] == "list" && m["slug"].is_string()).collect();
    listed.sort_by_key(|m| m["priority"].as_i64().unwrap_or(i64::MAX));
    listed
        .into_iter()
        .map(|m| Model {
            id: m["slug"].as_str().unwrap_or_default().into(),
            label: m["display_name"].as_str().or(m["slug"].as_str()).unwrap_or_default().into(),
            group: None,
        })
        .filter(|m| valid(&m.id))
        .collect()
}

/// `grok models`: lines like `  * grok-4.7 (default)` and `  - grok-4.6`.
pub(crate) fn parse_bullets(out: &str) -> Vec<Model> {
    out.lines()
        .filter_map(|l| l.trim_start().strip_prefix("* ").or_else(|| l.trim_start().strip_prefix("- ")))
        .filter_map(|rest| rest.split_whitespace().next())
        .filter(|id| valid(id))
        .map(|id| Model { id: id.into(), label: id.into(), group: None })
        .collect()
}

/// `--list-models` (Pi, October): a table whose first two columns are provider and model. Only
/// rows after its header count (a login shell may print other things first).
pub(crate) fn parse_table(out: &str) -> Vec<Model> {
    out.lines()
        .skip_while(|l| !l.trim_start().starts_with("provider"))
        .filter_map(|l| {
            let mut cols = l.split_whitespace();
            let (provider, model) = (cols.next()?, cols.next()?);
            (provider != "provider" && cols.next().is_some()).then_some((provider, model))
        })
        .map(|(provider, model)| Model { id: format!("{provider}/{model}"), label: model.into(), group: Some(provider.into()) })
        .filter(|m| valid(&m.id))
        .collect()
}

/// `opencode models`: one `provider/model` per line.
pub(crate) fn parse_slashed(out: &str) -> Vec<Model> {
    out.lines()
        .map(str::trim)
        .filter_map(|l| l.split_once('/').map(|(p, m)| (l, p, m)))
        .filter(|(l, ..)| valid(l))
        .map(|(l, provider, model)| Model { id: l.into(), label: model.into(), group: Some(provider.into()) })
        .collect()
}
