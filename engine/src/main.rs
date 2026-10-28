//! lantern-engine: finds the coding agents on this machine and reports what they're doing.
//!
//! Usage:
//!   lantern-engine serve                  JSON-lines protocol on stdin/stdout (used by the app)
//!   lantern-engine agents                 print the current agents as JSON
//!   lantern-engine hook claude|codex      hook entry point (called by the agents)
//!   lantern-engine hooks install|uninstall|status|remove-all
//!   lantern-engine harness status|install [--root DIR] [--download-node]
//!   lantern-engine report [--app NAME] [--app-version X]   send logs to October (details on stdin)

mod actions;
mod apps;
mod bus;
mod cards;
mod computer;
mod console;
mod control;
mod deliver;
mod elog;
#[cfg(unix)]
mod environment;
mod harness;
mod history;
mod hooks;
mod launch;
mod mobile;
mod model;
mod models;
mod october_core;
mod october_link;
mod platform;
mod procs;
mod pty;
mod readers;
mod report;
mod routines;
mod run;
mod scanner;
mod serve;
mod show;
mod sqlite;
mod subscriptions;
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
        #[cfg(debug_assertions)]
        Some("test-profile") => {
            println!("{}", hooks::support_dir().display());
            Ok(())
        }
        // Lantern's own terminal sessions on Windows (see pty.rs).
        #[cfg(windows)]
        Some("pty-host") => pty::host(args.get(1).map(String::as_str).unwrap_or("")),
        #[cfg(windows)]
        Some("attach") => pty::attach(args.get(1).map(String::as_str).unwrap_or("")),
        // Types into an agent's own console (see console.rs); run by the engine, not by people.
        #[cfg(windows)]
        Some("console-type") => {
            let number = |i: usize| args.get(i).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            let text = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
            match console::type_into(number(1) as u32, number(2), text(3), text(4), text(5)) {
                // Keys may be in the console: say so with the exit code, not as a plain failure.
                Err(error) if error.downcast_ref::<deliver::Uncertain>().is_some() => {
                    eprintln!("{error:#}");
                    std::process::exit(console::UNCERTAIN_EXIT)
                }
                other => other,
            }
        }
        // Posts one message as Lantern's Bus identity, under `october-bus agent run` (see bus.rs);
        // run by the engine, not by people.
        Some("bus-post") => bus::post(
            args.get(1).map(String::as_str).unwrap_or(""),
            args.get(2).map(String::as_str).unwrap_or(""),
            args.get(3).map(String::as_str).unwrap_or(""),
        ),
        // The card tools for one launch's agent (see cards/mcp.rs); run by agents, not by people.
        Some("cards-mcp") => cards::mcp::run(args.get(1).map(String::as_str).unwrap_or("")),
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
        Some("report") => report::run(&args[1..]),
        Some("installed") => {
            println!("{}", serde_json::to_string(&launch::installed())?);
            Ok(())
        }
        // October Harness: its status, or install it now (into Lantern's support folder, or
        // `--root DIR` to try the install elsewhere; `--download-node` skips October Desktop's
        // Node.js and downloads the pinned one).
        Some("october-token") => {
            if !harness::print_token() {
                std::process::exit(1);
            }
            Ok(())
        }
        Some("harness") => match args.get(1).map(String::as_str) {
            Some("status") | None => {
                let on_path = launch::on_path().kinds.contains(&model::Kind::October);
                println!("{}", harness::status(on_path));
                Ok(())
            }
            Some("install") => {
                let root = args.iter().position(|a| a == "--root").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);
                let reuse = !args.iter().any(|a| a == "--download-node");
                let started = std::time::Instant::now();
                let b = match root {
                    Some(root) => harness::install_into(&root, reuse)?,
                    None if reuse => harness::ensure_installed()?,
                    None => harness::install_into(&hooks::support_dir(), false)?,
                };
                println!(
                    "October Harness {} at {} (Node.js in {}), {:.1} s",
                    b.version,
                    b.bin.display(),
                    b.node_bin.display(),
                    started.elapsed().as_secs_f32()
                );
                Ok(())
            }
            Some(other) => bail!("unknown harness command: {other}"),
        },
        Some("--version") => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => bail!("unknown command: {other} (try serve, agents, hooks)"),
    }
}
