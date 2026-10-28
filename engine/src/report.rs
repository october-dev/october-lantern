//! Send Logs: a problem report for the October team, in October Desktop's format. One text file
//! goes to the October project's private `log-reports` bucket and a row to `log_reports`, so
//! support finds it by its code (LAN-XXXXXXXX). Anyone can send one, signed in or not; nobody
//! can read them back except October's team.
//!
//!   lantern-engine report [--app mac|windows|linux] [--app-version X] [--save-only]
//!
//! The app writes its own details (what it shows, its settings) on stdin. The report adds the
//! system, Lantern's files and the tails of its logs, with secrets and the home folder's path
//! taken out. If it can't be sent (no connection, a blocked network), it's saved to the Desktop
//! instead, to send some other way. Prints one JSON line: `{"sent":true,"code":…}` or
//! `{"sent":false,"code":…,"saved":…,"error":…}`.
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use serde_json::json;

use crate::harness::{SUPABASE_ANON_KEY, SUPABASE_URL};
use crate::hooks::support_dir;

const BUCKET: &str = "log-reports";
/// The bucket takes up to 5 MB a file.
const TOTAL: usize = 4_500_000;
const PER_LOG: usize = 1_000_000;

pub fn run(args: &[String]) -> Result<()> {
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned().unwrap_or_default();
    let (app, app_version) = (arg("--app"), arg("--app-version"));
    let mut details = String::new();
    std::io::stdin().take(256 * 1024).read_to_string(&mut details).context("reading the app's details")?;
    let code = format!("LAN-{}", uuid::Uuid::new_v4().simple().to_string()[..8].to_ascii_uppercase());
    let text = build(&code, &app, &app_version, &details, &seen());
    // The signed-in person's October token (from the app's environment, never an argument), so
    // the report is tied to their account.
    let token = std::env::var("LANTERN_REPORT_TOKEN").ok().filter(|t| !t.is_empty());
    // `--save-only`: to the Desktop, nothing sent.
    let sent = if args.iter().any(|a| a == "--save-only") {
        Err(anyhow::anyhow!("not sent (saved only)"))
    } else {
        upload(&code, &text, &app, &app_version, token.as_deref())
    };
    let result = match sent {
        Ok(()) => json!({"sent": true, "code": code}),
        Err(error) => match save(&code, &text) {
            Ok(path) => json!({"sent": false, "code": code, "saved": path, "error": format!("{error:#}")}),
            Err(saving) => json!({"sent": false, "code": code, "error": format!("{error:#}; couldn't save it either: {saving:#}")}),
        },
    };
    println!("{result}");
    Ok(())
}

fn build(code: &str, app: &str, app_version: &str, details: &str, seen: &str) -> String {
    let now = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default();
    let os = sysinfo::System::long_os_version().unwrap_or_else(|| std::env::consts::OS.into());
    let mut text = format!(
        "October Lantern problem report {code}\n{now}\nApp: {app} {app_version}\nEngine: {}\nSystem: {os} ({})\n\n== From the app ==\n{}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH,
        details.trim()
    );
    let dir = support_dir();
    text.push_str(&format!("\n== Lantern's folder ({}) ==\n", dir.display()));
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let size = entry.metadata().map(|m| if m.is_dir() { "folder".to_string() } else { format!("{} bytes", m.len()) });
        text.push_str(&format!("{}  {}\n", entry.file_name().to_string_lossy(), size.unwrap_or_default()));
    }
    text.push_str(seen);
    let mut logs: Vec<PathBuf> = ["logs/app.log", "logs/engine.log", "logs/engine-previous.log", "october-bus.log", "cua-driver.log"]
        .iter()
        .map(|name| dir.join(name))
        .collect();
    logs.extend(crash_reports());
    for log in logs {
        let Ok(bytes) = std::fs::read(&log) else { continue };
        let tail = &bytes[bytes.len().saturating_sub(PER_LOG)..];
        let skipped = if tail.len() < bytes.len() { format!(", last {} bytes", tail.len()) } else { String::new() };
        text.push_str(&format!("\n== {} ({} bytes{skipped}) ==\n", log.display(), bytes.len()));
        text.push_str(&String::from_utf8_lossy(tail));
        text.push('\n');
        if text.len() > TOTAL {
            break;
        }
    }
    let mut text = redact(&text, &crate::platform::home().to_string_lossy());
    if text.len() > TOTAL {
        let mut cut = TOTAL;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n[report cut to fit]\n");
    }
    text
}

/// What's there right now, for when the logs don't say why: the agents Lantern can start,
/// Lantern's own tmux sessions (an agent it started that never showed up is still one of these),
/// and each agent's latest hook event. Names, states and times only; never what anyone said.
fn seen() -> String {
    let mut text = String::new();
    let installed = crate::launch::installed();
    let names = |kinds: &[crate::model::Kind]| kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ");
    text.push_str(&format!(
        "\n== Agents Lantern can start ==\nInstalled: {}\nSigned in: {}\nNever set up: {}\ntmux: {}\n",
        names(&installed.kinds),
        names(&installed.signed_in),
        names(&installed.never_set_up),
        if installed.tmux { crate::tmux::tmux_bin() } else { "not found" },
    ));
    let now = crate::hooks::now_ms() / 1000;
    let ago = |at: u64| match now.saturating_sub(at) {
        s if s < 120 => format!("{s} s ago"),
        s if s < 7200 => format!("{} min ago", s / 60),
        s => format!("{} h ago", s / 3600),
    };
    text.push_str(&format!("\n== Lantern's sessions (tmux -L {}) ==\n", crate::launch::LANTERN_SOCKET));
    let format = "#{session_name}\t#{session_created}\t#{pane_pid}\t#{pane_current_command}\t#{pane_dead}";
    let panes = crate::run::output(
        crate::tmux::command().args(["-L", crate::launch::LANTERN_SOCKET, "list-panes", "-a", "-F", format]),
        Duration::from_secs(5),
    );
    match panes {
        Ok(out) if out.status.success() => {
            let listed = String::from_utf8_lossy(&out.stdout).into_owned();
            for line in listed.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                let [session, created, pid, command, dead] = f[..] else { continue };
                let created = created.parse().map(ago).unwrap_or_default();
                let dead = if dead == "1" { ", ended" } else { "" };
                text.push_str(&format!("{session}: started {created}, pid {pid}, running {command}{dead}\n"));
            }
            if listed.trim().is_empty() {
                text.push_str("none\n");
            }
        }
        Ok(out) => text.push_str(&format!("none ({})\n", String::from_utf8_lossy(&out.stderr).trim())),
        Err(error) => text.push_str(&format!("couldn't ask tmux: {error:#}\n")),
    }
    text.push_str("\n== Agents' latest hook events ==\n");
    let mut events: Vec<(u64, String)> = std::fs::read_dir(crate::hooks::events_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let event: serde_json::Value = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
            let at = event["at"].as_u64()? / 1000;
            Some((at, format!("{name}: {} from {}, {}", event["state"].as_str()?, event["source"].as_str().unwrap_or("?"), ago(at))))
        })
        .collect();
    events.sort_by_key(|e| std::cmp::Reverse(e.0));
    for (_, line) in events.iter().take(30) {
        text.push_str(line);
        text.push('\n');
    }
    if events.is_empty() {
        text.push_str("none\n");
    }
    text
}

/// The Mac's crash reports for Lantern from the last week (the newest three).
fn crash_reports() -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let week = std::time::SystemTime::now() - Duration::from_secs(7 * 86400);
    let dir = crate::platform::home().join("Library/Logs/DiagnosticReports");
    let mut reports: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("OctoberLantern") || name.starts_with("lantern-engine") || name.starts_with("October Lantern")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .filter(|(at, _)| *at > week)
        .collect();
    reports.sort_by_key(|r| std::cmp::Reverse(r.0));
    reports.into_iter().take(3).map(|(_, path)| path).collect()
}

fn upload(code: &str, text: &str, app: &str, app_version: &str, token: Option<&str>) -> Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).build().into();
    let path = format!("reports/{code}.txt");
    let put = |bearer: &str| {
        agent
            .post(&format!("{SUPABASE_URL}/storage/v1/object/{BUCKET}/{path}"))
            .header("apikey", SUPABASE_ANON_KEY)
            .header("Authorization", &format!("Bearer {bearer}"))
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("x-upsert", "false")
            .send(text.as_bytes())
    };
    // A sign-in that has expired is refused: send it as anyone instead.
    let mut user = token.and_then(user_id);
    let mut bearer = token.filter(|_| user.is_some()).unwrap_or(SUPABASE_ANON_KEY);
    if let Err(error) = put(bearer) {
        if bearer == SUPABASE_ANON_KEY || !matches!(error, ureq::Error::StatusCode(400..=403)) {
            return Err(error).context("sending the report to October");
        }
        (user, bearer) = (None, SUPABASE_ANON_KEY);
        put(bearer).context("sending the report to October")?;
    }
    let os = format!("{} {}", std::env::consts::OS, sysinfo::System::os_version().unwrap_or_default());
    let row = json!({
        "code": code,
        "storage_path": path,
        "user_id": user,
        "app_version": app_version,
        "os": os.trim(),
        "arch": std::env::consts::ARCH,
        "meta": {"source": "lantern", "app": app, "appVersion": app_version, "engineVersion": env!("CARGO_PKG_VERSION")},
    });
    // The file is what matters: a row that doesn't go in still leaves it findable by its code.
    if let Err(error) = agent
        .post(&format!("{SUPABASE_URL}/rest/v1/log_reports"))
        .header("apikey", SUPABASE_ANON_KEY)
        .header("Authorization", &format!("Bearer {bearer}"))
        .header("Prefer", "return=minimal")
        .send_json(row)
    {
        eprintln!("report {code}: the file was sent, its log_reports row wasn't: {error}");
    }
    Ok(())
}

/// The account a sign-in token belongs to (its `sub`), only to fill in `user_id`.
fn user_id(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims["sub"].as_str().map(str::to_string)
}

/// Saved where people find it: the Desktop, else Downloads, else the home folder.
fn save(code: &str, text: &str) -> Result<PathBuf> {
    let home = crate::platform::home();
    let dir = [home.join("Desktop"), home.join("Downloads")].into_iter().find(|d| d.is_dir()).unwrap_or(home);
    let path = dir.join(format!("October Lantern report {code}.txt"));
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// `text` with `word` replaced wherever it isn't part of a longer word.
fn whole_word(text: &str, word: &str, with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(word) {
        let (before, after) = (&rest[..at], &rest[at + word.len()..]);
        let joined = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric());
        out.push_str(before);
        out.push_str(if joined(before.chars().next_back()) || joined(after.chars().next()) { word } else { with });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Takes secrets out, best effort (October Desktop's rules): tokens and keys by their shape,
/// values after secret-sounding names (`API_KEY=`, `"accessToken":`), `Bearer` tokens and
/// passwords in URLs. The home folder's path becomes `~`, which also keeps the person's name out.
pub fn redact(text: &str, home: &str) -> String {
    let mut text = text.to_string();
    if home.len() > 1 {
        text = text.replace(home, "~");
        if home.contains('\\') {
            text = text.replace(&home.replace('\\', "/"), "~").replace(&home.replace('\\', "\\\\"), "~");
        }
    }
    // The user name alone, too: Lantern's session names carry their folder's name.
    let name = home.rsplit(['/', '\\']).next().unwrap_or_default();
    if name.len() >= 3 {
        text = whole_word(&text, name, "user");
    }
    let text = url_credentials(&text);
    let word = |b: u8| b.is_ascii_alphanumeric() || b"_-./+".contains(&b);
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    // After a secret's name: `named` until its `:`/`=`, then the next word is its value.
    let (mut named, mut separated) = (false, false);
    let mut i = 0;
    while i < bytes.len() {
        if word(bytes[i]) {
            let start = i;
            while i < bytes.len() && word(bytes[i]) {
                i += 1;
            }
            let w = &text[start..i];
            let upper = w.to_ascii_uppercase();
            if named && separated && !matches!(upper.as_str(), "BEARER" | "BASIC" | "TOKEN") {
                out.push_str("[redacted]");
                (named, separated) = (false, false);
                continue;
            }
            out.push_str(if secret_shaped(w) { "[redacted]" } else { w });
            if named && separated {
                continue; // "Authorization: Bearer <token>": the token is next.
            }
            named = SECRET_NAMES.iter().any(|name| upper.contains(name));
            separated = named && upper == "BEARER";
        } else {
            let ch = text[i..].chars().next().unwrap_or(' ');
            if named {
                match ch {
                    ':' | '=' => separated = true,
                    ' ' | '\t' | '"' | '\'' => {}
                    _ => (named, separated) = (false, false),
                }
            }
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

const SECRET_NAMES: [&str; 14] = [
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "API_KEY",
    "APIKEY",
    "API-KEY",
    "ACCESS_KEY",
    "PRIVATE_KEY",
    "DATABASE_URL",
    "AUTHORIZATION",
    "BEARER",
    "COOKIE",
    "CREDENTIAL",
];

/// Keys and tokens recognisable on their own: JWTs and the providers' key prefixes.
fn secret_shaped(w: &str) -> bool {
    const PREFIXES: [&str; 11] = ["sk-ant-", "sk-", "oct_", "xai-", "sbp_", "ck_", "sm_", "ghp_", "gho_", "github_pat_", "AKIA"];
    (w.starts_with("eyJ") && w.matches('.').count() >= 2 && w.len() > 30)
        || PREFIXES.iter().any(|p| w.starts_with(p) && w.len() >= p.len() + 12)
}

/// `scheme://user:password@host` → `scheme://[redacted]@host`.
fn url_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (before, after) = rest.split_at(at + 3);
        out.push_str(before);
        let end = after.find(|c: char| c.is_whitespace() || "/?#\"'".contains(c)).unwrap_or(after.len());
        match after[..end].rfind('@') {
            Some(user) if after[..user].contains(':') => {
                out.push_str("[redacted]");
                rest = &after[user..];
            }
            _ => rest = after,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_and_the_home_folder_never_leave_the_machine() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.c2lnbmF0dXJlc2lnbmF0dXJl";
        let text = format!(
            "Authorization: Bearer abc123secret\n\
             {{\"type\":\"october.session\",\"accessToken\":\"{jwt}\"}}\n\
             OPENAI_API_KEY=sk-proj1234567890abcdef ok\n\
             key sk-ant-api03-abcdefghijklmnop in a trace\n\
             postgres://admin:hunter2@db.example.com/x\n\
             C:\\Users\\Bhagyesh Rajesh Jaju\\AppData\\Local\\October Lantern\\logs\n\
             Session expired; sign in again: token refresh failed\n"
        );
        let out = redact(&text, r"C:\Users\Bhagyesh Rajesh Jaju");
        for secret in ["abc123secret", jwt, "sk-proj1234567890abcdef", "sk-ant-api03", "hunter2", "Bhagyesh"] {
            assert!(!out.contains(secret), "{secret} leaked:\n{out}");
        }
        assert!(out.contains(r"~\AppData\Local\October Lantern\logs"));
        assert!(out.contains("postgres://[redacted]@db.example.com/x"));
        assert!(out.contains("Session expired; sign in again: token refresh failed"), "prose about tokens stays:\n{out}");
        assert!(out.contains("OPENAI_API_KEY=[redacted] ok"));
    }

    #[test]
    fn the_user_name_in_session_names_stays_home() {
        let out = redact("claude-harsh-68256: started 3 min ago\nharshly worded\n/Users/harsh/x", "/Users/harsh");
        assert_eq!(out, "claude-user-68256: started 3 min ago\nharshly worded\n~/x");
    }

    #[test]
    fn a_report_names_itself_and_says_what_the_app_said() {
        let text = build("LAN-TEST0001", "windows", "0.6.8", "Engine: stopped\n", "\n== Agents Lantern can start ==\n");
        assert!(text.starts_with("October Lantern problem report LAN-TEST0001\n"));
        assert!(text.contains("App: windows 0.6.8") && text.contains("Engine: stopped"));
        assert!(text.len() <= TOTAL + 64);
    }

    #[test]
    fn the_account_comes_from_the_tokens_subject() {
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"sub":"user-1","role":"authenticated"}"#);
        assert_eq!(user_id(&format!("eyJhbGciOiJIUzI1NiJ9.{claims}.sig")).as_deref(), Some("user-1"));
        assert_eq!(user_id("not-a-token"), None);
    }
}
