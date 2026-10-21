//! Agents signed in to the user's own plan: Claude Code on a Claude subscription, Codex with a
//! ChatGPT plan, Grok with an x.ai account, Gemini CLI with a Google account. Ask Lantern starts
//! tasks with one of these before October, so a plan the user already pays for is used first.
//! Read from each agent's saved sign-in (no secrets are read out, and nothing is run).

use crate::model::Kind;
use base64::Engine;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// In the order Ask Lantern prefers them.
pub fn signed_in() -> Vec<Kind> {
    signed_in_at(&crate::platform::home(), &|var| std::env::var_os(var).map(PathBuf::from))
}

pub(crate) fn signed_in_at(home: &Path, env: &dyn Fn(&str) -> Option<PathBuf>) -> Vec<Kind> {
    let claude = env("CLAUDE_CONFIG_DIR").map(|d| d.join(".claude.json")).unwrap_or_else(|| home.join(".claude.json"));
    let codex = env("CODEX_HOME").unwrap_or_else(|| home.join(".codex")).join("auth.json");
    let mut kinds = Vec::new();
    if read(&claude).is_some_and(|v| claude_plan(&v)) {
        kinds.push(Kind::Claude);
    }
    if read(&codex).is_some_and(|v| codex_plan(&v)) {
        kinds.push(Kind::Codex);
    }
    if read(&home.join(".grok/auth.json")).is_some_and(|v| grok_account(&v)) {
        kinds.push(Kind::Grok);
    }
    if home.join(".gemini/oauth_creds.json").is_file() {
        kinds.push(Kind::Gemini);
    }
    kinds
}

/// Installed agents that have never been set up: Claude Code that has never signed in or finished
/// its first run, Codex with no sign-in saved. Started hidden, they'd sit on a sign-in screen
/// nobody sees, so the app shows them instead. Anything that might be signed in (an API key
/// from the environment, say) isn't listed.
pub fn never_set_up() -> Vec<Kind> {
    never_set_up_at(&crate::platform::home(), &|var| std::env::var_os(var).map(PathBuf::from))
}

pub(crate) fn never_set_up_at(home: &Path, env: &dyn Fn(&str) -> Option<PathBuf>) -> Vec<Kind> {
    let claude = env("CLAUDE_CONFIG_DIR").map(|d| d.join(".claude.json")).unwrap_or_else(|| home.join(".claude.json"));
    let codex = env("CODEX_HOME").unwrap_or_else(|| home.join(".codex")).join("auth.json");
    let mut kinds = Vec::new();
    let claude_ready = read(&claude)
        .is_some_and(|v| v["hasCompletedOnboarding"] == true || v.get("oauthAccount").is_some() || v.get("primaryApiKey").is_some());
    let api_key = |var: &str| env(var).is_some_and(|v| !v.as_os_str().is_empty());
    if !claude_ready && !api_key("ANTHROPIC_API_KEY") && !api_key("CLAUDE_CODE_USE_BEDROCK") && !api_key("CLAUDE_CODE_USE_VERTEX") {
        kinds.push(Kind::Claude);
    }
    if !codex.is_file() && !api_key("OPENAI_API_KEY") && !api_key("CODEX_API_KEY") {
        kinds.push(Kind::Codex);
    }
    kinds
}

fn read(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Claude Code signed in with claude.ai on Pro, Max, Team or Enterprise (`claude_max`, …). An API
/// key sign-in has no `oauthAccount`.
fn claude_plan(config: &Value) -> bool {
    let account = &config["oauthAccount"];
    account["organizationType"].as_str().is_some_and(|t| t.starts_with("claude_"))
        || account["billingType"].as_str().is_some_and(|t| t.contains("subscription"))
}

/// Codex signed in with ChatGPT on a paid plan. The plan is a claim in its id token; an id token
/// without one still counts.
fn codex_plan(auth: &Value) -> bool {
    if auth["auth_mode"].as_str() != Some("chatgpt") || auth["tokens"]["refresh_token"].as_str().is_none_or(str::is_empty) {
        return false;
    }
    let plan = auth["tokens"]["id_token"]
        .as_str()
        .and_then(jwt_claims)
        .and_then(|c| c["https://api.openai.com/auth"]["chatgpt_plan_type"].as_str().map(str::to_string));
    plan.is_none_or(|p| p != "free")
}

fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Grok signed in with an x.ai account (not just an API key).
fn grok_account(auth: &Value) -> bool {
    auth.as_object().is_some_and(|accounts| accounts.values().any(|a| a["refresh_token"].as_str().is_some_and(|t| !t.is_empty())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: Value) -> String {
        let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
        format!("e30.{body}.sig")
    }

    #[test]
    fn plans_the_user_pays_for_are_found_and_api_keys_are_not() {
        let home = std::env::temp_dir().join(format!("lantern-subs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::create_dir_all(home.join(".grok")).unwrap();
        let none = |_: &str| None;
        assert!(signed_in_at(&home, &none).is_empty());

        let write = |p: &str, v: Value| std::fs::write(home.join(p), v.to_string()).unwrap();
        write(".claude.json", serde_json::json!({"oauthAccount": {"organizationType": "claude_max"}}));
        let paid = jwt(serde_json::json!({"https://api.openai.com/auth": {"chatgpt_plan_type": "plus"}}));
        write(".codex/auth.json", serde_json::json!({"auth_mode": "chatgpt", "tokens": {"id_token": paid, "refresh_token": "r"}}));
        write(".grok/auth.json", serde_json::json!({"https://auth.x.ai::id": {"refresh_token": "r"}}));
        assert_eq!(signed_in_at(&home, &none), vec![Kind::Claude, Kind::Codex, Kind::Grok]);

        write(".claude.json", serde_json::json!({"primaryApiKey": "x"}));
        let free = jwt(serde_json::json!({"https://api.openai.com/auth": {"chatgpt_plan_type": "free"}}));
        write(".codex/auth.json", serde_json::json!({"auth_mode": "chatgpt", "tokens": {"id_token": free, "refresh_token": "r"}}));
        write(".grok/auth.json", serde_json::json!({}));
        assert!(signed_in_at(&home, &none).is_empty(), "an API key, a free plan and no account don't count");

        write(".codex/auth.json", serde_json::json!({"auth_mode": "apikey", "OPENAI_API_KEY": "k"}));
        assert!(signed_in_at(&home, &none).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn agents_never_set_up_are_found_and_anything_signed_in_is_not() {
        let home = std::env::temp_dir().join(format!("lantern-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        let none = |_: &str| None;
        assert_eq!(never_set_up_at(&home, &none), vec![Kind::Claude, Kind::Codex], "nothing saved yet");

        let key = |var: &str| (var == "ANTHROPIC_API_KEY" || var == "OPENAI_API_KEY").then(|| PathBuf::from("k"));
        assert!(never_set_up_at(&home, &key).is_empty(), "an API key in the environment might be enough");

        let write = |p: &str, v: Value| std::fs::write(home.join(p), v.to_string()).unwrap();
        write(".claude.json", serde_json::json!({"numStartups": 1}));
        assert_eq!(never_set_up_at(&home, &none), vec![Kind::Claude, Kind::Codex], "started once, never signed in");
        write(".claude.json", serde_json::json!({"hasCompletedOnboarding": true}));
        write(".codex/auth.json", serde_json::json!({"auth_mode": "apikey"}));
        assert!(never_set_up_at(&home, &none).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }
}
