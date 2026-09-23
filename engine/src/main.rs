//! lantern-engine: finds the coding agents on this machine and reports what they're doing.
//!
//! Usage:
//!   lantern-engine serve                  JSON-lines protocol on stdin/stdout (used by the app)
//!   lantern-engine agents                 print the current agents as JSON
//!   lantern-engine hook claude|codex      hook entry point (called by the agents)
//!   lantern-engine hooks install|uninstall|status

mod hooks;
mod launch;
mod model;
mod procs;
mod scanner;
mod serve;
mod tmux;
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
            Some("status") | None => hooks::status(),
            Some(other) => bail!("unknown hooks command: {other}"),
        },
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
