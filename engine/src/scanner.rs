//! Finds agent processes and works out what each one is doing.

use std::collections::HashMap;
use std::path::PathBuf;

use sysinfo::System;

use crate::hooks::{HookEvent, read_events};
use crate::model::{Agent, HostApp, Kind, SessionStatus, State, StateSource};
use crate::procs::{Proc, ProcTable, basename};
use crate::tmux;
use crate::transcripts::Transcripts;

pub struct Scanner {
    sys: System,
    transcripts: Transcripts,
    /// Remember the transcript each Claude process was matched to, so guesses stay stable.
    claude_paths: HashMap<u32, PathBuf>,
    /// Handle numbers stay with an agent for its whole life; a new agent takes the lowest free one.
    handles: HashMap<u32, (Kind, usize)>,
}

/// Arguments that mean the process is a helper or a headless run, not an interactive agent.
fn is_helper(kind: Kind, args: &[String]) -> bool {
    let flags: &[&str] = match kind {
        Kind::Claude => &["daemon", "--bg-pty-host", "--bg-spare", "--chrome-native-host", "mcp", "-p", "--print"],
        Kind::Codex => &["exec", "mcp", "mcp-server", "app-server", "login", "logout", "proto", "e"],
        Kind::Opencode => &["serve", "run", "mcp"],
        Kind::Pi => &["-p", "--print"],
    };
    // Subcommands only count in first position; flags anywhere.
    args.first().is_some_and(|a| flags.contains(&a.as_str()))
        || args.iter().any(|a| a.starts_with('-') && flags.contains(&a.as_str()))
}

/// Classifies a process from argv. `node /path/to/codex ...` counts as codex.
pub fn classify(p: &Proc) -> Option<Kind> {
    let argv0 = p.cmd.first().map(|a| basename(a)).unwrap_or(p.name.as_str());
    let (prog, rest) = if matches!(argv0, "node" | "bun" | "deno") || argv0.starts_with("node") {
        (p.cmd.get(1).map(|a| basename(a))?, p.cmd.get(2..).unwrap_or(&[]))
    } else {
        (argv0, p.cmd.get(1..).unwrap_or(&[]))
    };
    let kind = match prog {
        "claude" => Kind::Claude,
        "codex" => Kind::Codex,
        "opencode" => Kind::Opencode,
        "pi" => Kind::Pi,
        _ if p.exe.as_ref().is_some_and(|e| e.to_string_lossy().contains("/.local/share/claude/versions/")) => {
            Kind::Claude
        }
        _ => return None,
    };
    if is_helper(kind, rest) { None } else { Some(kind) }
}

fn arg_after<'a>(args: &'a [String], names: &[&str]) -> Option<&'a str> {
    args.iter().position(|a| names.contains(&a.as_str())).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn looks_like_id(s: &str) -> bool {
    s.len() >= 16 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// The first GUI app above `pid`, e.g. Terminal, iTerm2, Ghostty, cmux.
fn host_app(table: &ProcTable, pid: u32) -> Option<HostApp> {
    table.ancestors(pid).into_iter().find_map(|p| {
        let exe = p.exe.as_ref()?.to_string_lossy().into_owned();
        let idx = exe.find(".app/Contents/")?;
        let bundle = &exe[..idx + 4];
        let app = basename(&bundle[..bundle.len() - 4]).to_string();
        Some(HostApp { app, pid: p.pid, bundle_path: bundle.to_string() })
    })
}

impl Scanner {
    pub fn new() -> Self {
        Scanner {
            sys: System::new(),
            transcripts: Transcripts::default(),
            claude_paths: HashMap::new(),
            handles: HashMap::new(),
        }
    }

    pub fn scan(&mut self) -> Vec<Agent> {
        let table = ProcTable::capture(&mut self.sys);

        // Interactive agents only (they have a terminal), and only the outermost process of each
        // kind (the node wrapper, not the native binary it spawns).
        let candidates: HashMap<u32, Kind> = table
            .procs
            .values()
            .filter(|p| p.tty.is_some())
            .filter_map(|p| classify(p).map(|k| (p.pid, k)))
            .collect();
        let mut agents: Vec<(&Proc, Kind)> = candidates
            .iter()
            .filter(|(pid, kind)| !table.ancestors(**pid).iter().any(|a| candidates.get(&a.pid) == Some(kind)))
            .filter_map(|(pid, kind)| table.get(*pid).map(|p| (p, *kind)))
            .collect();
        agents.sort_by_key(|(p, k)| (*k, p.pid));

        let tmux = if agents.is_empty() { None } else { Some(tmux::discover(&table)) };
        let events = read_events();
        let live: Vec<u32> = agents.iter().map(|(p, _)| p.pid).collect();
        self.transcripts.forget_pids(&live);
        self.claude_paths.retain(|pid, _| live.contains(pid));
        self.handles.retain(|pid, (kind, _)| agents.iter().any(|(p, k)| p.pid == *pid && k == kind));

        let mut out = Vec::new();
        for (p, kind) in agents {
            let n = self.handle_number(p.pid, kind);
            let args = p.cmd.get(1..).unwrap_or(&[]);

            let hook = events
                .iter()
                .filter(|e| e.source == kind.as_str())
                .filter(|e| e.ancestors.contains(&p.pid))
                .max_by_key(|e| e.at);
            let transcript = self.transcript_status(p, kind, args, hook);
            let (status, source) = merge(hook, transcript);

            let tmux_pane = tmux.as_ref().and_then(|t| p.tty.as_ref().and_then(|tty| t.panes_by_tty.get(tty))).cloned();
            let host = match &tmux_pane {
                Some(pane) => {
                    let session = pane.target.split(':').next().unwrap_or("").to_string();
                    tmux.as_ref()
                        .and_then(|t| t.clients.get(&(pane.socket.clone(), session)))
                        .and_then(|client| host_app(&table, *client))
                }
                None => host_app(&table, p.pid),
            };
            let cwd = p.cwd.as_ref().map(|c| c.to_string_lossy().into_owned());
            out.push(Agent {
                id: format!("{}:{}", kind.as_str(), p.pid),
                kind,
                handle: format!("{}-{n}", kind.as_str()),
                pid: p.pid,
                tty: p.tty.clone(),
                project: p.cwd.as_ref().and_then(|c| c.file_name()).map(|f| f.to_string_lossy().into_owned()),
                cwd,
                title: status.title,
                session_id: status.session_id,
                state: status.state.unwrap_or(State::Unknown),
                state_since: status.since,
                last_message: status.last_message,
                question: status.question,
                can_reply: tmux_pane.is_some(),
                host,
                tmux: tmux_pane,
                state_source: source,
            });
        }
        out
    }

    fn handle_number(&mut self, pid: u32, kind: Kind) -> usize {
        if let Some((_, n)) = self.handles.get(&pid) {
            return *n;
        }
        let n = (1..)
            .find(|n| !self.handles.values().any(|(k, used)| *k == kind && used == n))
            .unwrap_or(1);
        self.handles.insert(pid, (kind, n));
        n
    }

    fn transcript_status(&mut self, p: &Proc, kind: Kind, args: &[String], hook: Option<&HookEvent>) -> Option<SessionStatus> {
        match kind {
            Kind::Claude => {
                let from_hook = hook.and_then(|h| h.transcript_path.as_ref()).map(PathBuf::from);
                let from_args = arg_after(args, &["--session-id", "--resume", "-r"])
                    .filter(|id| looks_like_id(id))
                    .and_then(|id| self.transcripts.claude_path_for_session(id));
                let path = match from_hook.or(from_args) {
                    Some(path) => Some(path),
                    None => match self.claude_paths.get(&p.pid).filter(|p| p.exists()) {
                        Some(known) => Some(known.clone()),
                        None => {
                            let claimed: Vec<PathBuf> = self.claude_paths.values().cloned().collect();
                            p.cwd.as_deref().and_then(|cwd| self.transcripts.claude_guess_path(cwd, p.start_time, &claimed))
                        }
                    },
                };
                let path = path?;
                self.claude_paths.insert(p.pid, path.clone());
                self.transcripts.claude_status(&path)
            }
            Kind::Codex => {
                let path = self.transcripts.codex_path_for_pid(p.pid).or_else(|| {
                    // The node wrapper doesn't hold the file; its native child does.
                    let child = self_child(p.pid)?;
                    self.transcripts.codex_path_for_pid(child)
                })?;
                self.transcripts.codex_status(&path)
            }
            Kind::Opencode | Kind::Pi => None,
        }
    }
}

/// The native codex binary under a node wrapper. Looked up with `pgrep` to keep it simple.
fn self_child(pid: u32) -> Option<u32> {
    let out = std::process::Command::new("/usr/bin/pgrep").args(["-P", &pid.to_string()]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().next()?.trim().parse().ok()
}

/// A hook event is exact but only as fresh as the last event; the transcript may be newer.
/// Take the state from whichever is newer, and fill gaps from the other.
fn merge(hook: Option<&HookEvent>, transcript: Option<SessionStatus>) -> (SessionStatus, StateSource) {
    match (hook.map(HookEvent::status), transcript) {
        (Some(h), Some(t)) => {
            let hook_newer = h.since.unwrap_or(0) + 1500 >= t.since.unwrap_or(0);
            let mut s = if hook_newer { h.clone() } else { t.clone() };
            s.title = t.title.clone();
            s.session_id = s.session_id.or(t.session_id.clone());
            if s.last_message.is_none() && s.state == Some(State::Waiting) {
                s.last_message = t.last_message.clone();
            }
            (s, if hook_newer { StateSource::Hook } else { StateSource::Transcript })
        }
        (Some(h), None) => (h, StateSource::Hook),
        (None, Some(t)) => (t, StateSource::Transcript),
        (None, None) => (SessionStatus::default(), StateSource::None),
    }
}
