//! Which models an agent can start with, and the flag that picks one.
//!
//! Agents that can list their models are asked, through the user's login shell (so they see the
//! same accounts and config as in a terminal): Codex (`codex debug models`), Grok (`grok models`),
//! OpenCode (`opencode models`) and Pi (`--list-models`). Signed-in October uses its live account
//! catalogue; otherwise its CLI lists its own models. Claude Code and
//! Gemini CLI can't list theirs but take short names for the current models, which are offered
//! instead. Any model id can also be typed in the app.

use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::launch::program;
#[cfg(unix)]
use crate::launch::shell;
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
        | Kind::Copilot
        | Kind::Muse => Some("--model"),
        _ => None,
    }
}

/// A model id as an agent would take it: no spaces or shell-looking characters, not an option.
pub fn valid(id: &str) -> bool {
    !id.is_empty() && id.len() <= 120 && !id.starts_with('-') && id.chars().all(|c| c.is_ascii_alphanumeric() || "._:/[]~@+-".contains(c))
}

type Listed = HashMap<Kind, (Instant, Vec<Model>)>;

/// The models to offer for `kind`. CLI catalogues are cached for ten minutes except October,
/// whose current account and catalogue are read for each request.
pub fn list(kind: Kind) -> Result<Vec<Model>> {
    // The CLI's seeded catalogue can omit models available through the app's paid account.
    // Never substitute direct-provider credentials when that account's catalogue fails.
    if kind == Kind::October {
        return match crate::harness::model_session()? {
            Some(token) => october_catalogue(&token),
            None => Ok(discover(kind)),
        };
    }
    static CACHE: Mutex<Option<Listed>> = Mutex::new(None);
    if let Some((at, models)) = CACHE.lock().unwrap().get_or_insert_with(HashMap::new).get(&kind)
        && at.elapsed() < Duration::from_secs(600)
    {
        return Ok(models.clone());
    }
    let models = discover(kind);
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(kind, (Instant::now(), models.clone()));
    Ok(models)
}

fn october_catalogue(token: &str) -> Result<Vec<Model>> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
    let mut response = agent
        .get("https://www.october.dev/v1/models")
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .map_err(|_| anyhow::anyhow!("Couldn't load your October account's models. Check your connection and sign-in, then retry."))?;
    let value = response
        .body_mut()
        .with_config()
        .limit(4 * 1024 * 1024)
        .read_json::<serde_json::Value>()
        .context("October returned an unreadable model catalogue. Retry.")?;
    parse_october_catalogue(&value)
}

fn parse_october_catalogue(value: &serde_json::Value) -> Result<Vec<Model>> {
    let entries = value["data"].as_array().context("October returned an invalid model catalogue. Retry.")?;
    let mut models = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        let id = entry["id"].as_str().context("October returned a model without an id. Retry.")?;
        // The gateway id already names its upstream. The harness adds its own provider prefix;
        // preserve every segment, including an id that itself starts with `october/`.
        let canonical = format!("october/{id}");
        if !valid(&canonical) {
            bail!("October returned a model id Lantern cannot launch. Retry.");
        }
        if seen.insert(canonical.clone()) {
            models.push(Model { id: canonical, label: id.into(), group: Some("October account".into()) });
        }
    }
    if models.is_empty() {
        bail!("Your October account returned no models. Retry.");
    }
    Ok(models)
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
#[cfg(windows)]
fn ask(kind: Kind, args: &str) -> Option<String> {
    let bin = crate::platform::find_on_path(program(kind))?;
    let out = crate::run::output(Command::new(bin).args(args.split_whitespace()), Duration::from_secs(20)).ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Runs `<agent> <args>` in the user's login shell, with a time limit.
#[cfg(unix)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_catalogue_preserves_gateway_routing_and_rejects_partial_lists() {
        let parsed = parse_october_catalogue(&serde_json::json!({"data":[
            {"id":"openrouter/moonshotai/kimi-k2.6"}, {"id":"october/free"},
            {"id":"openrouter/moonshotai/kimi-k2.6"}
        ]}))
        .unwrap();
        assert_eq!(
            parsed.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["october/openrouter/moonshotai/kimi-k2.6", "october/october/free"]
        );
        for value in [
            serde_json::json!({}),
            serde_json::json!({"data":[]}),
            serde_json::json!({"data":[{"id":"valid/model"},{}]}),
            serde_json::json!({"data":[{"id":"bad model"}]}),
        ] {
            assert!(parse_october_catalogue(&value).is_err());
        }
    }
}
