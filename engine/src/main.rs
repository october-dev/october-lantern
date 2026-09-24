//! lantern-engine: finds the coding agents on this machine and reports what they're doing.
//!
//! Usage:
//!   lantern-engine serve                  JSON-lines protocol on stdin/stdout (used by the app)
//!   lantern-engine agents                 print the current agents as JSON
//!   lantern-engine hook claude|codex      hook entry point (called by the agents)
//!   lantern-engine hooks install|uninstall|status|remove-all

mod actions;
mod apps;
mod bus;
mod deliver;
mod history;
mod hooks;
mod launch;
mod mobile;
mod model;
mod models;
mod october_core;
mod october_link;
mod procs;
mod readers;
mod run;
mod scanner;
mod serve;
#[cfg(test)]
mod tests;
mod tmux;
mod toolkit;
mod transcripts;

use anyhow::{Result, bail};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve::run(),
        Some("agents") | None => {
            let agents = scanner::Scanner::new().scan();
            println!("{}", serde_json::to_string_pretty(&agents)?);
            Ok(())
        }
        Some("hook") => {
            hooks::run_hook(args.get(1).map(String::as_str).unwrap_or(""), args.get(2).cloned());
            Ok(())
        }
        Some("hooks") => match args.get(1).map(String::as_str) {
            Some("install") => hooks::install(),
            Some("uninstall") => hooks::uninstall(),
            Some("remove-all") => hooks::uninstall_everything(),
            Some("status") | None => hooks::status(),
            Some(other) => bail!("unknown hooks command: {other}"),
        },
        Some("history") => {
            let id = args.get(1).map(String::as_str).unwrap_or("");
            let mut scanner = scanner::Scanner::new();
            let agents = scanner.scan();
            let agent = agents.iter().find(|a| a.id == id).ok_or_else(|| anyhow::anyhow!("no agent {id}"))?;
            println!("{}", serde_json::to_string_pretty(&scanner.history(agent))?);
            Ok(())
        }
        // Development: test a session reader against stored sessions.
        // lantern-engine probe opencode|pi|october|gemini <cwd> [started-secs]
        Some("probe") => {
            let kind = args.get(1).map(String::as_str).unwrap_or("");
            let cwd = std::path::PathBuf::from(args.get(2).map(String::as_str).unwrap_or("."));
            let start: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            let (status, chat) = match kind {
                "opencode" => (readers::opencode::status(&cwd, start), readers::opencode::history(&cwd, start)),
                "gemini" => {
                    let f = readers::gemini::session_file(&cwd, start);
                    println!("file: {f:?}");
                    let m = f.as_ref().and_then(|f| std::fs::metadata(f).ok()).map(|_| 0).unwrap_or(0);
                    (f.as_ref().map(|f| readers::gemini::parse(f, m)), f.map(|f| readers::gemini::history(&f)).unwrap_or_default())
                }
                k => {
                    let f = readers::pi::session_file(&cwd, start, k == "october");
                    println!("file: {f:?}");
                    (f.as_ref().map(|f| readers::pi::parse(f, 0)), f.map(|f| readers::pi::history(&f)).unwrap_or_default())
                }
            };
            println!("{status:#?}");
            for m in chat.iter().rev().take(6).rev() {
                println!("{:?}: {}", m.role, model::truncate(&m.text.replace('\n', " "), 100));
            }
            Ok(())
        }
        Some("installed") => {
            println!("{}", serde_json::to_string(launch::installed())?);
            Ok(())
        }
        Some("--version") => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => bail!("unknown command: {other} (try serve, agents, hooks)"),
    }
}
