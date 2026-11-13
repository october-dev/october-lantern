//! Finds agent processes and works out what each one is doing.

use std::collections::HashMap;
use std::path::PathBuf;

use sysinfo::{Pid, System};

use crate::hooks::{HookEvent, read_events};
use crate::model::{Agent, HostApp, Kind, Route, SessionMatch, SessionStatus, State, StateSource};
use crate::procs::{Proc, ProcCache, ProcTable, basename};
use crate::tmux;
use crate::transcripts::Transcripts;

pub struct Scanner {
    sys: System,
    history_cache: HashMap<String, (HistoryStamp, crate::history::History)>,
    proc_cache: ProcCache,
    /// Agent pid → the child process that holds its session file open (Codex's native binary).
    children: HashMap<u32, u32>,
    transcripts: Transcripts,
    /// Remember the transcript each Claude process was matched to (and whether exactly), so
    /// guesses stay stable.
    claude_paths: HashMap<u32, (PathBuf, bool)>,
    /// Codex rollouts found without the process's open files (Windows): path, and whether exact.
    codex_paths: HashMap<u32, (PathBuf, bool)>,
    /// Handle numbers stay with an agent for its whole life; a new agent takes the lowest free one.
    handles: HashMap<u32, (Kind, usize)>,
    opencode_cache: HashMap<u32, ((u64, u64), SessionStatus)>,
    /// Working directory and start time per agent, for readers that match sessions by them.
    started: HashMap<u32, (PathBuf, u64)>,
    /// When each agent's process started, in milliseconds (where the platform says): a guessed
    /// session counts as its history only if written after that.
    started_ms: HashMap<u32, (u64, u64)>,
    /// App sessions (Codex app threads, recent Claude Desktop/Cowork/Codex app sessions) by agent
    /// id: their session file, for history.
    app_paths: HashMap<String, (Kind, PathBuf)>,
    /// Handle numbers for app sessions, by agent id ("claude-app-2", "codex-app-1").
    app_handles: HashMap<String, (String, usize)>,
    /// App sessions that aren't running, re-read every 30 s.
    recent: Option<(std::time::Instant, Vec<crate::apps::Recent>)>,
}

#[derive(PartialEq)]
enum HistoryStamp {
    File(Kind, PathBuf, u64, u64, std::time::SystemTime),
    OpenCode(PathBuf, u64, (u64, u64)),
}

/// Arguments that mean the process is a helper or a headless run, not an interactive agent.
fn is_helper(kind: Kind, args: &[String]) -> bool {
    let (subcommands, flags): (&[&str], &[&str]) = match kind {
        Kind::Claude => (&["daemon", "mcp"], &["--bg-pty-host", "--bg-spare", "--chrome-native-host", "-p", "--print"]),
        Kind::Codex => (&["exec", "e", "mcp", "mcp-server", "app-server", "login", "logout", "proto"], &[]),
        Kind::Opencode => (&["serve", "run", "mcp"], &[]),
        // GitHub Copilot's one-shot and server modes (`--prompt=PROMPT` is a documented
        // spelling; `--server`/`--headless`/`--remote` are legacy SDK server startup, not
        // interactive). This runs for native and npm launches alike.
        Kind::Copilot => (
            &["mcp", "serve", "exec", "run", "login"],
            &["-p", "--print", "--prompt", "--acp", "--server", "--headless", "--remote", "-h", "--help", "-v", "--version"],
        ),
        _ => (&["mcp", "serve", "exec", "run", "login"], &["-p", "--print", "--prompt"]),
    };
    // `--flag=value` spells the same flag.
    args.first().is_some_and(|a| subcommands.contains(&a.as_str()))
        || args.iter().any(|a| flags.contains(&a.split('=').next().unwrap_or(a.as_str())))
}

const INTERPRETERS: [&str; 5] = ["node", "bun", "deno", "python", "uv"];

/// npm packages whose entry script doesn't carry the program's name. On Windows, npm starts an
/// agent as `node …\node_modules\@anthropic-ai\claude-code\cli.js` (through a `claude.cmd`
/// shim) rather than through a `claude` link.
const NPM_PACKAGES: [(&str, &str); 8] = [
    ("@anthropic-ai/claude-code", "claude"),
    ("@openai/codex", "codex"),
    ("@google/gemini-cli", "gemini"),
    ("@qwen-code/qwen-code", "qwen"),
    ("@github/copilot", "copilot"),
    ("@sourcegraph/amp", "amp"),
    ("@mariozechner/pi-coding-agent", "pi"),
    ("opencode-ai", "opencode"),
];

/// The program a script run by an interpreter stands for.
fn script_program(script: &str) -> &str {
    let path = script.replace('\\', "/").to_ascii_lowercase();
    // GitHub Copilot's npm entry script doesn't carry the program's name on any OS.
    if path.contains("/node_modules/@github/copilot/") {
        return "copilot";
    }
    if cfg!(windows)
        && let Some((_, prog)) = NPM_PACKAGES.iter().find(|(pkg, _)| path.contains(&format!("/node_modules/{pkg}/")))
    {
        return prog;
    }
    basename(script)
}

/// AWS Copilot's top-level commands, from its root command definition. AWS ended support
/// in June 2026, so this list no longer changes. (`run` is the `run local` command's name;
/// `help` is added by its parser.)
const AWS_COMMANDS: [&str; 15] =
    ["app", "completion", "deploy", "docs", "env", "help", "init", "job", "pipeline", "run", "secret", "storage", "svc", "task", "version"];

/// GitHub Copilot CLI options that take a value, from `copilot --help` (1.0.110, unchanged in
/// 1.0.111): `--name value` and `--name=value` alike. `--config-dir` (deprecated) and
/// `--worktree`/`-w` come from the current command reference, which is newer than either
/// binary; listing them keeps newer launches visible, and a wrong guess here only ever shows a
/// process that exits at once (both CLIs reject a misparsed line immediately).
const COPILOT_VALUE_OPTS: [&str; 29] = [
    "add-dir",
    "add-github-mcp-tool",
    "add-github-mcp-toolset",
    "additional-mcp-config",
    "agent",
    "attachment",
    "auto-tier",
    "config-dir",
    "context",
    "disable-mcp-server",
    "dynamic-retrieval",
    "enable-mcp-server",
    "extension-sdk-path",
    "interactive",
    "log-dir",
    "log-level",
    "max-ai-credits",
    "max-autopilot-continues",
    "mcp-github-auth",
    "mode",
    "model",
    "name",
    "output-format",
    "plugin-dir",
    "prompt",
    "reasoning-effort",
    "session-id",
    "stream",
    "usage-output-file",
];

/// GitHub Copilot CLI options that take an optional single value (`--resume sessions` resumes
/// the session named `sessions`; `--resume --model x` leaves `--model` alone).
const COPILOT_OPTIONAL_OPTS: [&str; 7] = ["bash-env", "connect", "mouse", "resume", "share", "sync-session", "worktree"];

/// GitHub Copilot CLI options that take any number of values, up to the next option
/// (`--allow-tool a b sessions` consumes all three).
const COPILOT_GREEDY_OPTS: [&str; 7] =
    ["allow-tool", "allow-url", "available-tools", "deny-tool", "deny-url", "excluded-tools", "secret-env-vars"];

/// GitHub Copilot CLI short options: `-nweb` and `-n web` alike, `-sn foo` combined.
const COPILOT_VALUE_SHORTS: [char; 4] = ['C', 'i', 'n', 'p'];
const COPILOT_OPTIONAL_SHORTS: [char; 2] = ['r', 'w'];

/// GitHub's own subcommands that Lantern recognizes (its one-shots and anything unknown stay
/// unrecognized).
const COPILOT_SUBCOMMANDS: [&str; 9] = ["config", "sessions", "plugin", "sandbox", "skill", "workflow", "memories", "instruction", "lsp"];

/// Whether the next argument is taken as an option's value: anything but another option or `--`.
/// A lone `-` is a value (both parsers read it as one); on its own it is a word neither CLI
/// accepts.
fn takes_value(next: Option<&&str>) -> bool {
    next.is_some_and(|n| !n.starts_with('-') || *n == "-")
}

/// Whether a native `copilot` is GitHub's CLI in an interactive form: bare, flags only, or one
/// of its own subcommands. AWS's deploy tool shares the name and takes options before its
/// command (`copilot --name web deploy`), so the first word left after GitHub's own options is
/// checked against AWS's frozen command list first; a session named `deploy`
/// (`copilot --yolo -n deploy`) is consumed as `--name`'s value and stays visible. One-shot and
/// server modes are filtered by `is_helper`, not here.
fn is_github_copilot_native(p: &Proc) -> bool {
    let exe = p.exe.as_ref().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let argv0 = p.cmd.first().map(|a| a.to_ascii_lowercase()).unwrap_or_default();
    // The npm package's platform binary bears its origin in its path.
    if exe.contains("@github/copilot") || argv0.contains("@github/copilot") {
        return true;
    }
    let args: Vec<&str> = p.cmd.get(1..).unwrap_or(&[]).iter().map(String::as_str).collect();
    if args.is_empty() {
        // Bare `copilot` starts GitHub's interactive session (AWS's prints help and exits).
        return true;
    }
    let mut first_word: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if a == "--" {
            // Words after `--` are never subcommands (GitHub errors on them); a bare `--`
            // is neutral.
            if i + 1 < args.len() {
                return false;
            }
            break;
        }
        if let Some(name) = a.strip_prefix("--") {
            let (opt, attached) = match name.split_once('=') {
                Some((o, _)) => (o, true),
                None => (name, false),
            };
            if !attached {
                if COPILOT_VALUE_OPTS.contains(&opt) || COPILOT_OPTIONAL_OPTS.contains(&opt) {
                    if takes_value(args.get(i + 1)) {
                        i += 1;
                    }
                } else if COPILOT_GREEDY_OPTS.contains(&opt) {
                    while takes_value(args.get(i + 1)) {
                        i += 1;
                    }
                }
                // Anything else takes no value (`--yolo` is a plain flag; an unknown option
                // errors in GitHub, so a flags-only line carrying one is at most a flash).
            }
            i += 1;
            continue;
        }
        if a.len() > 1 && a.starts_with('-') {
            // A short cluster (`-sn foo`): booleans chain, a value short ends it.
            let rest = &a[1..];
            let bytes = rest.as_bytes();
            let mut j = 0;
            while j < bytes.len() {
                let c = bytes[j] as char;
                if COPILOT_VALUE_SHORTS.contains(&c) || COPILOT_OPTIONAL_SHORTS.contains(&c) {
                    if j + 1 == bytes.len() && takes_value(args.get(i + 1)) {
                        i += 1;
                    }
                    break;
                }
                if c != 'h' && c != 's' && c != 'v' {
                    // Unknown short: the whole token is an unknown option.
                    break;
                }
                j += 1;
            }
            i += 1;
            continue;
        }
        if first_word.is_none() {
            first_word = Some(a);
        }
        i += 1;
    }
    match first_word {
        None => true,
        Some(w) if AWS_COMMANDS.contains(&w) => false,
        Some(w) if COPILOT_SUBCOMMANDS.contains(&w) => true,
        Some(_) => false,
    }
}

/// Classifies a process from argv. `node /path/to/codex ...` counts as codex.
pub fn classify(p: &Proc) -> Option<Kind> {
    let argv0 = p.cmd.first().map(|a| basename(a)).unwrap_or(p.name.as_str());
    let wrapped = INTERPRETERS.iter().any(|i| argv0.starts_with(i));
    let (prog, rest) = if wrapped {
        (p.cmd.get(1).map(|a| script_program(a))?, p.cmd.get(2..).unwrap_or(&[]))
    } else {
        (argv0, p.cmd.get(1..).unwrap_or(&[]))
    };
    let kind = match Kind::from_program(prog) {
        // A native `copilot` may be GitHub's CLI or AWS's deploy tool: tell them apart.
        Some(Kind::Copilot) if !wrapped && !is_github_copilot_native(p) => return None,
        Some(kind) => kind,
        None if p.exe.as_ref().is_some_and(|e| e.to_string_lossy().contains("/.local/share/claude/versions/")) => Kind::Claude,
        None => return None,
    };
    if is_helper(kind, rest) { None } else { Some(kind) }
}

fn arg_after<'a>(args: &'a [String], names: &[&str]) -> Option<&'a str> {
    args.iter().position(|a| names.contains(&a.as_str())).and_then(|i| args.get(i + 1)).map(String::as_str)
}

/// On Windows, an agent with a console of its own can be typed into through it (console.rs).
fn console_route(tty: Option<&str>) -> Option<Route> {
    if !cfg!(windows) {
        return None;
    }
    let host = tty?.strip_prefix("con:")?.parse().ok()?;
    Some(Route::Console { host })
}

fn looks_like_id(s: &str) -> bool {
    s.len() >= 16 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Terminal apps on Windows, by executable name (lowercase, without `.exe`).
#[cfg(windows)]
const WINDOWS_TERMINALS: [(&str, &str); 11] = [
    ("windowsterminal", "Windows Terminal"),
    ("code", "VS Code"),
    ("code - insiders", "VS Code Insiders"),
    ("cursor", "Cursor"),
    ("windsurf", "Windsurf"),
    ("wezterm-gui", "WezTerm"),
    ("alacritty", "Alacritty"),
    ("tabby", "Tabby"),
    ("conemu64", "ConEmu"),
    ("hyper", "Hyper"),
    ("mintty", "mintty"),
];

/// The terminal app above `pid`: Windows Terminal, VS Code... found among its parents or, when
/// the console was handed to Windows Terminal, above its console host. A plain console window is
/// its console host (`conhost.exe`), which owns that window.
#[cfg(windows)]
pub(crate) fn host_app(table: &ProcTable, pid: u32) -> Option<HostApp> {
    let terminal = |p: &Proc| {
        let name = basename(&p.name).to_ascii_lowercase();
        WINDOWS_TERMINALS.iter().find(|(exe, _)| *exe == name).map(|(_, app)| *app)
    };
    let found = |chain: Vec<&Proc>| {
        let i = chain.iter().position(|p| terminal(p).is_some())?;
        // Electron apps run terminals in a helper process; the window belongs to the outermost
        // process of the same program.
        let outer = chain[i..].iter().take_while(|p| p.name == chain[i].name).last()?;
        let exe = outer.exe.as_ref().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        Some(HostApp { app: terminal(chain[i])?.to_string(), pid: outer.pid, bundle_path: exe, canvas: None, node: None })
    };
    if let Some(host) = found(table.ancestors(pid)) {
        return Some(host);
    }
    let console: u32 = table.get(pid)?.tty.as_deref()?.strip_prefix("con:")?.parse().ok()?;
    let conhost = table.get(console)?;
    let mut chain = vec![conhost];
    chain.extend(table.ancestors(console));
    found(chain).or_else(|| {
        let exe = conhost.exe.as_ref().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        Some(HostApp { app: "Console".into(), pid: console, bundle_path: exe, canvas: None, node: None })
    })
}

/// The first GUI app above `pid`, e.g. Terminal, iTerm2, Ghostty, cmux.
#[cfg(unix)]
pub(crate) fn host_app(table: &ProcTable, pid: u32) -> Option<HostApp> {
    table.ancestors(pid).into_iter().find_map(|p| {
        let exe = p.exe.as_ref()?.to_string_lossy().into_owned();
        let idx = exe.find(".app/Contents/")?;
        let bundle = &exe[..idx + 4];
        let app = basename(&bundle[..bundle.len() - 4]).to_string();
        Some(HostApp { app, pid: p.pid, bundle_path: bundle.to_string(), canvas: None, node: None })
    })
}

impl Scanner {
    pub fn new() -> Self {
        Scanner {
            sys: System::new(),
            history_cache: HashMap::new(),
            proc_cache: ProcCache::default(),
            children: HashMap::new(),
            transcripts: Transcripts::default(),
            claude_paths: HashMap::new(),
            codex_paths: HashMap::new(),
            handles: HashMap::new(),
            opencode_cache: HashMap::new(),
            started: HashMap::new(),
            started_ms: HashMap::new(),
            app_paths: HashMap::new(),
            app_handles: HashMap::new(),
            recent: None,
        }
    }

    pub fn scan(&mut self) -> Vec<Agent> {
        let mut table = ProcTable::capture(&mut self.sys, &mut self.proc_cache);
        let claude_live = crate::apps::claude_live();
        // Claude sessions the desktop app runs (its Code tab, Cowork): no terminal, and started the
        // way headless runs are, but their status file says where they came from.
        let from_app = |p: &Proc| claude_live.get(&p.pid).is_some_and(|l| crate::apps::claude_source(l.entrypoint.as_deref()).is_some());

        // Interactive agents only (they have a terminal, or they're an app's Claude session), and
        // only the outermost process of each kind (the node wrapper, not the native binary it spawns).
        let candidates: HashMap<u32, Kind> = table
            .procs
            .values()
            .filter(|p| p.tty.is_some() || from_app(p))
            .filter_map(|p| classify(p).or_else(|| from_app(p).then_some(Kind::Claude)).map(|k| (p.pid, k)))
            .collect();
        // The Codex app's threads run inside `codex app-server`.
        let app_servers: Vec<(u32, u64)> = table.procs.values().filter(|p| is_codex_app_server(p)).map(|p| (p.pid, p.start_time)).collect();
        let outermost: Vec<(u32, Kind)> = candidates
            .iter()
            .filter(|(pid, kind)| !table.ancestors(**pid).iter().any(|a| candidates.get(&a.pid) == Some(kind)))
            .map(|(pid, kind)| (*pid, *kind))
            .collect();
        let agent_pids: Vec<u32> = outermost.iter().map(|(p, _)| *p).collect();
        table.fill_cwds(&mut self.sys, &agent_pids);
        let mut agents: Vec<(&Proc, Kind)> = outermost.iter().filter_map(|(pid, kind)| table.get(*pid).map(|p| (p, *kind))).collect();
        agents.sort_by_key(|(p, k)| (*k, p.pid));

        let tmux = if agents.is_empty() { None } else { Some(tmux::discover(&table)) };
        // Lantern's own sessions (Windows).
        let sessions = if agents.is_empty() { Vec::new() } else { crate::pty::sessions() };
        let events = read_events();
        let live: Vec<u32> = agents.iter().map(|(p, _)| p.pid).collect();
        // Keep open-file lookups for agents and their children (Codex's file is held by the child).
        let children: Vec<u32> = live.iter().filter_map(|pid| table.child_of(*pid)).collect();
        self.transcripts.forget_pids(&[live.clone(), children].concat());
        self.claude_paths.retain(|pid, _| live.contains(pid));
        self.codex_paths.retain(|pid, _| live.contains(pid));
        self.handles.retain(|pid, (kind, _)| agents.iter().any(|(p, k)| p.pid == *pid && k == kind));
        self.opencode_cache.retain(|pid, _| live.contains(pid));
        self.children = live.iter().filter_map(|pid| table.child_of(*pid).map(|c| (*pid, c))).collect();
        self.started = agents.iter().filter_map(|(p, _)| p.cwd.clone().map(|c| (p.pid, (c, p.start_time)))).collect();
        self.started_ms = agents.iter().map(|(p, _)| (p.pid, (p.start_time, p.start_ms))).collect();

        let mut out = Vec::new();
        for (p, kind) in agents {
            let n = self.handle_number(p.pid, kind);
            let now = crate::procs::live(p.pid);
            let args = p.cmd.get(1..).unwrap_or(&[]);

            let hook = events.iter().filter(|e| e.source == kind.as_str()).filter(|e| e.ancestors.contains(&p.pid)).max_by_key(|e| e.at);
            let live_status = claude_live.get(&p.pid).filter(|_| kind == Kind::Claude);
            let (transcript, session_match) = self.transcript_status(p, kind, args, hook, live_status);
            let (mut status, source) = merge(hook, transcript);
            if let Some(l) = live_status {
                apply_live(&mut status, source, l);
            }

            let tmux_pane = tmux.as_ref().and_then(|t| p.tty.as_ref().and_then(|tty| t.panes_by_tty.get(tty))).cloned();
            // The agent a session started, or one running under it.
            let started_by = |pid: u32, start: u64| pid == p.pid && start == p.start_time;
            let session = sessions.iter().find(|s| {
                started_by(s.child_pid, s.child_start)
                    || table.ancestors(p.pid).iter().any(|a| a.pid == s.child_pid && a.start_time == s.child_start)
            });
            let host = match &tmux_pane {
                // Shown in a window only while attached, like a detached tmux session.
                _ if session.is_some() => None,
                Some(pane) => {
                    let session = pane.target.split(':').next().unwrap_or("").to_string();
                    tmux.as_ref().and_then(|t| t.clients.get(&(pane.socket.clone(), session))).and_then(|client| host_app(&table, *client))
                }
                None => host_app(&table, p.pid),
            };
            let mut host = host;
            if let Some(h) = host.as_mut().filter(|h| h.app == "October") {
                h.canvas = self.environ(p.pid, "OCTOBER_BUS_CANVAS").filter(|c| is_canvas_id(c));
                h.node = h.canvas.as_ref().and_then(|_| self.environ(p.pid, "OCTOBER_BUS_NODE")).filter(|n| n.len() <= 128);
            }
            let route = match session {
                Some(s) => Route::Pty { session: s.name.clone() },
                None => self.route(p.pid, p.tty.as_deref(), tmux_pane.is_some(), host.as_ref()),
            };
            let cwd = p.cwd.as_ref().map(|c| c.to_string_lossy().into_owned());
            out.push(Agent {
                id: format!("{}:{}:{}", kind.as_str(), p.pid, p.start_time),
                kind,
                handle: format!("{}-{n}", kind.as_str()),
                pid: p.pid,
                start_time: p.start_time,
                exe: now.as_ref().and_then(|l| l.exe.clone()),
                comm: now.map(|l| l.comm).unwrap_or_default(),
                tty: p.tty.clone(),
                project: p.cwd.as_ref().and_then(|c| c.file_name()).map(|f| f.to_string_lossy().into_owned()),
                cwd,
                title: status.title,
                session_id: status.session_id,
                state: status.state.unwrap_or(State::Unknown),
                state_since: status.since,
                last_message: status.last_message,
                question: status.question,
                question_kind: status.question_kind,
                question_detail: status.question_detail,
                prompt_id: status.prompt_id,
                choices: status.choices,
                transcript: (kind == Kind::Claude).then(|| self.claude_paths.get(&p.pid).map(|(path, _)| path.clone())).flatten(),
                bus_id: table.bus_id(p.pid),
                session_match,
                source: live_status.and_then(|l| crate::apps::claude_source(l.entrypoint.as_deref())).map(String::from),
                live: true,
                can_reply: route != Route::None,
                route,
                host,
                tmux: tmux_pane,
                state_source: source,
            });
        }
        self.app_paths.clear();
        let mut seen: Vec<String> = out.iter().filter_map(|a| a.session_id.clone()).collect();
        for (pid, start) in app_servers {
            let host = host_app(&table, pid);
            for path in crate::apps::codex_open_rollouts(pid) {
                let Some(status) = self.transcripts.codex_status(&path) else { continue };
                let session = status.session_id.clone().or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()));
                let id = format!("codex:{pid}:{start}:{}", session.as_deref().unwrap_or(""));
                seen.extend(session.clone());
                let cwd = crate::apps::rollout_cwd(&path);
                let a = self.app_agent(id, Kind::Codex, "Codex app", status, cwd, true, host.clone(), pid, start, &path);
                out.push(a);
            }
        }
        for r in self.recent_app_sessions() {
            if seen.contains(&r.session_id) {
                continue;
            }
            let kind = if r.source == "Codex app" { Kind::Codex } else { Kind::Claude };
            let status = match kind {
                Kind::Codex => self.transcripts.codex_status(&r.path),
                _ => self.transcripts.claude_status(&r.path),
            };
            let Some(mut status) = status else { continue };
            status.title = status.title.or(r.title.clone());
            let host = crate::apps::app_bundle(r.source).map(|b| HostApp {
                app: b.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                pid: 0,
                bundle_path: b.to_string_lossy().into_owned(),
                canvas: None,
                node: None,
            });
            let id = format!("{}:session:{}", kind.as_str(), r.session_id);
            let a = self.app_agent(id, kind, r.source, status, r.cwd.clone(), false, host, 0, 0, &r.path);
            out.push(a);
        }
        let ids: Vec<String> = out.iter().map(|a| a.id.clone()).collect();
        self.app_handles.retain(|id, _| ids.contains(id));
        self.transcripts.prune_stale();
        hide_ambiguous(&mut out);
        out
    }

    /// A variable from a process's environment.
    fn environ(&self, pid: u32, key: &str) -> Option<String> {
        let prefix = format!("{key}=");
        self.sys
            .process(Pid::from_u32(pid))?
            .environ()
            .iter()
            .find_map(|e| e.to_str().and_then(|e| e.strip_prefix(&prefix)).filter(|v| !v.is_empty()).map(String::from))
    }

    /// How Lantern can type into this agent: tmux first (it's exact), then the terminal app's own
    /// mechanism.
    fn route(&self, pid: u32, tty: Option<&str>, in_tmux: bool, host: Option<&HostApp>) -> Route {
        if in_tmux {
            return Route::Tmux;
        }
        if let Some(route) = console_route(tty) {
            return route;
        }
        let app = host.map(|h| h.app.as_str()).unwrap_or("");
        let env = |key: &str| self.environ(pid, key);
        let dev_tty = tty.map(|t| format!("/dev/{t}"));
        match app {
            "cmux" if std::path::Path::new(crate::deliver::CMUX).exists() => match (env("CMUX_WORKSPACE_ID"), env("CMUX_SURFACE_ID")) {
                (Some(workspace), Some(surface)) => Route::Cmux { workspace, surface },
                _ => Route::None,
            },
            "Terminal" => dev_tty.map(|tty| Route::Terminal { tty }).unwrap_or(Route::None),
            "iTerm" | "iTerm2" => dev_tty.map(|tty| Route::Iterm { tty }).unwrap_or(Route::None),
            _ => Route::None,
        }
    }

    /// App sessions that aren't running (Claude Desktop, Cowork, Codex app) from the last three
    /// days; the list is re-read every 30 seconds.
    fn recent_app_sessions(&mut self) -> Vec<crate::apps::Recent> {
        if let Some((at, list)) = &self.recent
            && at.elapsed() < std::time::Duration::from_secs(30)
        {
            return list.clone();
        }
        let now = crate::hooks::now_ms();
        let mut list = crate::apps::recent_claude(now);
        list.extend(crate::apps::recent_codex(now));
        list.sort_by_key(|r| std::cmp::Reverse(r.updated_ms));
        self.recent = Some((std::time::Instant::now(), list.clone()));
        list
    }

    /// An agent for a session that isn't in a terminal (Codex app thread, or a recent app session).
    #[allow(clippy::too_many_arguments)]
    fn app_agent(
        &mut self,
        id: String,
        kind: Kind,
        source: &str,
        status: SessionStatus,
        cwd: Option<String>,
        live: bool,
        host: Option<HostApp>,
        pid: u32,
        start_time: u64,
        path: &std::path::Path,
    ) -> Agent {
        let prefix = match source {
            "Cowork" => "cowork".to_string(),
            _ => format!("{}-app", kind.as_str()),
        };
        let n = match self.app_handles.get(&id) {
            Some((_, n)) => *n,
            None => {
                let n = (1..).find(|n| !self.app_handles.values().any(|(p, used)| *p == prefix && used == n)).unwrap_or(1);
                self.app_handles.insert(id.clone(), (prefix.clone(), n));
                n
            }
        };
        self.app_paths.insert(id.clone(), (kind, path.to_path_buf()));
        Agent {
            id,
            kind,
            handle: format!("{prefix}-{n}"),
            pid,
            start_time,
            exe: None,
            comm: String::new(),
            tty: None,
            project: cwd.as_deref().and_then(|c| std::path::Path::new(c).file_name()).map(|f| f.to_string_lossy().into_owned()),
            cwd,
            title: status.title,
            session_id: status.session_id,
            state: status.state.unwrap_or(State::Unknown),
            state_since: status.since,
            last_message: status.last_message,
            // Apps aren't answered by keys: say what it's asking, without buttons.
            question_kind: status.question_kind.map(|_| crate::model::QuestionKind::Other),
            question: status.question,
            question_detail: None,
            prompt_id: None,
            choices: None,
            transcript: None,
            bus_id: None,
            session_match: SessionMatch::Exact,
            source: Some(source.to_string()),
            live,
            host,
            tmux: None,
            can_reply: false,
            route: Route::None,
            state_source: StateSource::Transcript,
        }
    }

    /// The conversation for the chat view. `None` when Lantern can't read this harness's sessions;
    /// `Err` when it couldn't read this one just now (keep what's shown, and try again).
    pub fn history(&mut self, agent: &Agent) -> Option<Result<crate::history::History, crate::history::ReadFailed>> {
        let source = if let Some((kind, path)) = self.app_paths.get(&agent.id) {
            Some((*kind, path.clone()))
        } else {
            if agent.session_match == SessionMatch::Ambiguous {
                return None;
            }
            match agent.kind {
                Kind::Claude => self.claude_paths.get(&agent.pid).map(|(p, _)| (Kind::Claude, p.clone())),
                Kind::Codex => self
                    .transcripts
                    .codex_path_for_pid(agent.pid)
                    .or_else(|| self.children.get(&agent.pid).copied().and_then(|c| self.transcripts.codex_path_for_pid(c)))
                    .or_else(|| self.codex_paths.get(&agent.pid).map(|(p, _)| p.clone()))
                    .map(|p| (Kind::Codex, p)),
                Kind::Muse => self.transcripts.muse_path_for_pid(agent.pid).map(|p| (Kind::Muse, p)),
                Kind::Copilot => {
                    let home = self.environ(agent.pid, "COPILOT_HOME");
                    let known = self.started.get(&agent.pid);
                    let root = known.map(|(cwd, _)| crate::readers::copilot::root_for_env(cwd, home.as_deref()));
                    let open = root.as_ref().and_then(|r| {
                        let start = known.map(|(_, s)| *s).unwrap_or(0);
                        self.transcripts.copilot_path_for_pid(r, agent.pid, start).or_else(|| {
                            self.children.get(&agent.pid).copied().and_then(|c| self.transcripts.copilot_path_for_pid(r, c, start))
                        })
                    });
                    let guessed = if open.is_none() {
                        known.and_then(|(cwd, start)| root.as_ref().and_then(|r| crate::readers::copilot::session_file_in(r, cwd, *start)))
                    } else {
                        None
                    };
                    open.or(guessed).map(|p| (Kind::Copilot, p))
                }
                Kind::Pi | Kind::October | Kind::Gemini => {
                    let (cwd, start) = self.started.get(&agent.pid)?;
                    let path = if agent.kind == Kind::Gemini {
                        crate::readers::gemini::session_file(cwd, *start)
                    } else {
                        crate::readers::pi::session_file(cwd, *start, agent.kind == Kind::October)
                    };
                    path.map(|p| (agent.kind, p))
                }
                Kind::Opencode => {
                    let (cwd, start) = self.started.get(&agent.pid)?.clone();
                    let Some(stamp) = crate::readers::opencode::stamp() else { return Some(Ok(Default::default())) };
                    let stamp = HistoryStamp::OpenCode(cwd.clone(), start, stamp);
                    if let Some((old, messages)) = self.history_cache.get(&agent.id)
                        && *old == stamp
                    {
                        return Some(Ok(messages.clone()));
                    }
                    let Some(history) = crate::readers::opencode::history(&cwd, start) else {
                        return Some(Err(crate::history::ReadFailed));
                    };
                    self.remember_history(&agent.id, stamp, &history);
                    return Some(Ok(history));
                }
                _ => return None,
            }
        };
        let exact = self.app_paths.contains_key(&agent.id) || agent.session_match == SessionMatch::Exact;
        let Some((kind, path)) = source else { return Some(Ok(Default::default())) };
        // A session file that's gone (or not written yet) is an empty conversation, not an
        // unsupported agent.
        let Ok(metadata) = std::fs::metadata(&path) else { return Some(Ok(Default::default())) };
        let Ok(modified) = metadata.modified() else { return Some(Ok(Default::default())) };
        // A guessed session (picked by folder and time) is this process's conversation only once
        // it has written to it since it started: until then the newest file there may be an
        // earlier process's, which isn't this agent's history. (A session it resumed shows from
        // its first write.) Status keeps its own minute of tolerance.
        let modified_ms = modified.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        if !exact && modified_ms < self.started_at_ms(agent) {
            return Some(Ok(Default::default()));
        }
        let stamp = HistoryStamp::File(kind, path.clone(), crate::platform::file_id(&metadata), metadata.len(), modified);
        if let Some((old, messages)) = self.history_cache.get(&agent.id)
            && *old == stamp
        {
            return Some(Ok(messages.clone()));
        }
        let chat = match kind {
            Kind::Codex => crate::history::codex(&path),
            Kind::Pi | Kind::October => crate::readers::pi::history(&path),
            Kind::Gemini => crate::readers::gemini::history(&path),
            Kind::Muse => crate::readers::muse::history(&path),
            Kind::Copilot => crate::readers::copilot::history(&path),
            _ => crate::history::claude(&path),
        };
        // A file that couldn't be read is no read, and isn't cached.
        let Some(chat) = chat else { return Some(Err(crate::history::ReadFailed)) };
        // Named by its file: `/clear` or `/new` in the same process is another source.
        let history = crate::history::History::of(&path.to_string_lossy(), chat);
        self.remember_history(&agent.id, stamp, &history);
        Some(Ok(history))
    }

    /// When `agent`'s process started, in milliseconds: to the millisecond where the platform
    /// says (a file an earlier process wrote in the same second as this one started is still
    /// before it), else its whole second.
    fn started_at_ms(&self, agent: &Agent) -> u64 {
        match self.started_ms.get(&agent.pid) {
            // The same process (its whole-second start matches), and a millisecond time that agrees
            // with it (Linux works out boot time two ways, which can differ by a second).
            Some(&(secs, ms)) if secs == agent.start_time && ms > 0 && (ms / 1000).abs_diff(secs) <= 1 => ms,
            _ => agent.start_time.saturating_mul(1000),
        }
    }

    fn remember_history(&mut self, id: &str, stamp: HistoryStamp, history: &crate::history::History) {
        if self.history_cache.len() >= 64 {
            self.history_cache.clear();
        }
        self.history_cache.insert(id.to_string(), (stamp, history.clone()));
    }

    fn handle_number(&mut self, pid: u32, kind: Kind) -> usize {
        if let Some((_, n)) = self.handles.get(&pid) {
            return *n;
        }
        let n = (1..).find(|n| !self.handles.values().any(|(k, used)| *k == kind && used == n)).unwrap_or(1);
        self.handles.insert(pid, (kind, n));
        n
    }

    /// The session's state, and how sure Lantern is that the session belongs to this process.
    fn transcript_status(
        &mut self,
        p: &Proc,
        kind: Kind,
        args: &[String],
        hook: Option<&HookEvent>,
        live: Option<&crate::apps::ClaudeLive>,
    ) -> (Option<SessionStatus>, SessionMatch) {
        let found = |status: Option<SessionStatus>, m: SessionMatch| match status {
            Some(s) => (Some(s), m),
            None => (None, SessionMatch::None),
        };
        match kind {
            Kind::Claude => {
                let from_hook = hook.and_then(|h| h.transcript_path.as_ref()).map(PathBuf::from);
                // Claude's own status file names the session exactly.
                let from_live = live
                    .and_then(|l| l.session_id.as_deref())
                    .filter(|id| looks_like_id(id))
                    .and_then(|id| self.transcripts.claude_path_for_session(id));
                let from_args = arg_after(args, &["session-id", "resume", "-r"])
                    .filter(|id| looks_like_id(id))
                    .and_then(|id| self.transcripts.claude_path_for_session(id));
                let found_path = match from_hook.or(from_live).or(from_args) {
                    Some(path) => Some((path, true)),
                    None => match self.claude_paths.get(&p.pid).filter(|(p, _)| p.exists()) {
                        Some(known) => Some(known.clone()),
                        None => {
                            let claimed: Vec<PathBuf> = self.claude_paths.values().map(|(p, _)| p.clone()).collect();
                            p.cwd
                                .as_deref()
                                .and_then(|cwd| self.transcripts.claude_guess_path(cwd, p.start_time, &claimed))
                                .map(|p| (p, false))
                        }
                    },
                };
                let Some((path, exact)) = found_path else { return (None, SessionMatch::None) };
                self.claude_paths.insert(p.pid, (path.clone(), exact));
                found(self.transcripts.claude_status(&path), if exact { SessionMatch::Exact } else { SessionMatch::Guessed })
            }
            Kind::Codex => {
                // The node wrapper doesn't hold the file; its native child does.
                let child = self.children.get(&p.pid).copied();
                let open =
                    self.transcripts.codex_path_for_pid(p.pid).or_else(|| child.and_then(|c| self.transcripts.codex_path_for_pid(c)));
                if let Some(path) = open {
                    return found(self.transcripts.codex_status(&path), SessionMatch::Exact);
                }
                // Where open files can't be read (Windows): the session its hook named, the one
                // found before, or the newest unclaimed rollout for its folder. Elsewhere the open
                // file is the truth, and no file yet means nothing has happened.
                if !cfg!(windows) {
                    return (None, SessionMatch::None);
                }
                let from_hook = hook
                    .and_then(|h| h.session_id.as_deref())
                    .filter(|id| looks_like_id(id))
                    .and_then(|id| self.transcripts.codex_path_for_session(id, p.start_time));
                let found_path = match from_hook {
                    Some(path) => Some((path, true)),
                    None => match self.codex_paths.get(&p.pid).filter(|(p, _)| p.exists()) {
                        Some(known) => Some(known.clone()),
                        None => {
                            let claimed: Vec<PathBuf> = self.codex_paths.values().map(|(p, _)| p.clone()).collect();
                            p.cwd
                                .as_deref()
                                .and_then(|cwd| self.transcripts.codex_guess_path(cwd, p.start_time, &claimed))
                                .map(|p| (p, false))
                        }
                    },
                };
                let Some((path, exact)) = found_path else { return (None, SessionMatch::None) };
                self.codex_paths.insert(p.pid, (path.clone(), exact));
                found(self.transcripts.codex_status(&path), if exact { SessionMatch::Exact } else { SessionMatch::Guessed })
            }
            Kind::Muse => match self.transcripts.muse_path_for_pid(p.pid) {
                Some(path) => found(self.transcripts.cached(&path, crate::readers::muse::parse), SessionMatch::Exact),
                None => (None, SessionMatch::None),
            },
            Kind::Copilot => {
                // The npm wrapper doesn't hold the marker; its native child does. The
                // parent's start time stands in for the child's where open files can't be
                // read: the parent is always older, so a genuinely stale marker still fails.
                let child = self.children.get(&p.pid).copied();
                let home = self.environ(p.pid, "COPILOT_HOME");
                let root = p.cwd.as_deref().map(|c| crate::readers::copilot::root_for_env(c, home.as_deref()));
                let open = root.as_ref().and_then(|r| {
                    self.transcripts
                        .copilot_path_for_pid(r, p.pid, p.start_time)
                        .or_else(|| child.and_then(|c| self.transcripts.copilot_path_for_pid(r, c, p.start_time)))
                });
                if let Some(path) = open {
                    // A marker with no log yet (`/clear` before its first write, resume
                    // before the first prompt): an empty session, still exact.
                    if !path.is_file() {
                        let mut status = idle();
                        status.session_id = path.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned());
                        return (Some(status), SessionMatch::Exact);
                    }
                    return found(self.transcripts.cached(&path, crate::readers::copilot::parse), SessionMatch::Exact);
                }
                let (Some(cwd), Some(root)) = (p.cwd.as_deref(), root) else { return (None, SessionMatch::None) };
                match crate::readers::copilot::session_file_in(&root, cwd, p.start_time) {
                    Some(path) => found(self.transcripts.cached(&path, crate::readers::copilot::parse), SessionMatch::Guessed),
                    None => (Some(idle()), SessionMatch::None),
                }
            }
            // No session file since the process started means nothing has happened yet.
            Kind::Pi | Kind::October => {
                let Some(cwd) = p.cwd.as_deref() else { return (None, SessionMatch::None) };
                match crate::readers::pi::session_file(cwd, p.start_time, kind == Kind::October) {
                    Some(path) => found(self.transcripts.cached(&path, crate::readers::pi::parse), SessionMatch::Guessed),
                    None => (Some(idle()), SessionMatch::None),
                }
            }
            Kind::Gemini => {
                let Some(cwd) = p.cwd.as_deref() else { return (None, SessionMatch::None) };
                match crate::readers::gemini::session_file(cwd, p.start_time) {
                    Some(path) => found(self.transcripts.cached(&path, crate::readers::gemini::parse), SessionMatch::Guessed),
                    None => (Some(idle()), SessionMatch::None),
                }
            }
            Kind::Opencode => {
                // One database for every session: cache per (process, database change).
                let (Some(stamp), Some(cwd)) = (crate::readers::opencode::stamp(), p.cwd.as_deref()) else {
                    return (None, SessionMatch::None);
                };
                if let Some((s, status)) = self.opencode_cache.get(&p.pid)
                    && *s == stamp
                {
                    let m = if status.session_id.is_some() { SessionMatch::Guessed } else { SessionMatch::None };
                    return (Some(status.clone()), m);
                }
                let status = crate::readers::opencode::status(cwd, p.start_time).unwrap_or_else(idle);
                self.opencode_cache.insert(p.pid, (stamp, status.clone()));
                let m = if status.session_id.is_some() { SessionMatch::Guessed } else { SessionMatch::None };
                (Some(status), m)
            }
            _ => (None, SessionMatch::None),
        }
    }
}

/// Two agents of one kind in one folder whose sessions were only guessed may each be showing the
/// other's conversation. Show neither: no state, message or history, until a hook or the command
/// line says which session is whose. Replies still work: they go to the process's own terminal.
pub(crate) fn hide_ambiguous(agents: &mut [Agent]) {
    let mut shared: HashMap<(Kind, String), usize> = HashMap::new();
    for a in agents.iter().filter(|a| a.session_match != SessionMatch::Exact) {
        if let Some(cwd) = &a.cwd {
            *shared.entry((a.kind, cwd.clone())).or_default() += 1;
        }
    }
    for a in agents.iter_mut() {
        let crowded = a.cwd.as_ref().is_some_and(|c| shared.get(&(a.kind, c.clone())).copied().unwrap_or(0) > 1);
        if crowded && a.session_match == SessionMatch::Guessed && a.state_source != StateSource::Hook {
            a.session_match = SessionMatch::Ambiguous;
            a.state = State::Unknown;
            a.state_since = None;
            a.last_message = None;
            a.question = None;
            a.question_kind = None;
            a.question_detail = None;
            a.prompt_id = None;
            a.choices = None;
            a.title = None;
            a.session_id = None;
        }
    }
}

fn idle() -> SessionStatus {
    SessionStatus { state: Some(State::Idle), ..Default::default() }
}

/// `codex app-server`: the process the Codex app runs its threads in.
fn is_codex_app_server(p: &Proc) -> bool {
    let program = |a: &String| basename(a) == "codex";
    let is_codex = p.cmd.first().is_some_and(program) || p.cmd.get(1).is_some_and(program) || p.name == "codex";
    is_codex && p.cmd.iter().any(|a| a == "app-server")
}

/// Claude's status file says busy or idle the moment it changes. Busy is working; idle after a
/// transcript that still looks mid-turn means the turn ended (or was stopped). A hook's question
/// (a permission prompt), or one in the transcript (a question for you), is kept.
pub(crate) fn apply_live(status: &mut SessionStatus, source: StateSource, live: &crate::apps::ClaudeLive) {
    if status.state == Some(State::NeedsInput) && (source == StateSource::Hook || status.prompt_id.is_some()) {
        return;
    }
    match live.busy() {
        Some(true) if status.state != Some(State::Working) => {
            status.state = Some(State::Working);
            status.since = live.status_updated_at.or(status.since);
            status.question = None;
            status.question_kind = None;
        }
        Some(false) if status.state == Some(State::Working) => {
            status.state = Some(if status.last_message.is_some() { State::Waiting } else { State::Idle });
            status.since = live.status_updated_at.or(status.since);
        }
        _ => {}
    }
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
            // Claude's own question or plan, open in the transcript, is the prompt a hook is
            // announcing (an older hook calls it a permission to run AskUserQuestion).
            if hook_newer && s.state == Some(State::NeedsInput) && t.prompt_id.is_some() && h.question_kind != t.question_kind {
                s.question = t.question.clone();
                s.question_kind = t.question_kind;
                s.question_detail = t.question_detail.clone();
                s.prompt_id = t.prompt_id.clone();
                s.choices = t.choices.clone();
            }
            (s, if hook_newer { StateSource::Hook } else { StateSource::Transcript })
        }
        (Some(h), None) => (h, StateSource::Hook),
        (None, Some(t)) => (t, StateSource::Transcript),
        (None, None) => (SessionStatus::default(), StateSource::None),
    }
}

/// An October canvas id: a UUID (what `october://canvas/<id>` accepts).
pub(crate) fn is_canvas_id(s: &str) -> bool {
    s.len() == 36 && s.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

#[cfg(test)]
mod history_cache_tests {
    use super::*;
    #[test]
    fn cached_history_is_invalidated_by_file_changes_and_session_switches() {
        let root = std::env::temp_dir().join(format!("lantern-history-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("first.jsonl");
        let write = |path: &std::path::Path, text: &str| {
            std::fs::write(path, format!("{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{text}\"}}}}\n")).unwrap();
        };
        write(&path, "first");
        let mut scanner = Scanner::new();
        let a = crate::tests::agent_for(123, 1, Some("ttys001"));
        scanner.app_paths.insert(a.id.clone(), (Kind::Claude, path.clone()));
        assert_eq!(scanner.history(&a).unwrap().unwrap().messages[0].text, "first");
        assert_eq!(scanner.history(&a).unwrap().unwrap().messages[0].text, "first");
        assert_eq!(scanner.history_cache.len(), 1);
        let first_source = scanner.history(&a).unwrap().unwrap().source;
        write(&path, "changed message");
        assert_eq!(scanner.history(&a).unwrap().unwrap().messages[0].text, "changed message");
        assert_eq!(scanner.history(&a).unwrap().unwrap().source, first_source, "the same session file");
        let second = root.join("second.jsonl");
        write(&second, "new session");
        scanner.app_paths.insert(a.id.clone(), (Kind::Claude, second));
        assert_eq!(scanner.history(&a).unwrap().unwrap().messages[0].text, "new session");
        assert_ne!(scanner.history(&a).unwrap().unwrap().source, first_source, "a new session in the same process is another source");
        assert!(scanner.history(&a).unwrap().unwrap().complete, "a short file is read from its start");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A guessed session written last before the process started is an earlier process's: no
    /// history (and no source) until this process writes to a session.
    #[test]
    fn a_guessed_session_counts_only_once_this_process_writes_it() {
        let root = std::env::temp_dir().join(format!("lantern-owned-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let previous = root.join("previous.jsonl");
        std::fs::write(&previous, "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"old task\"}}\n").unwrap();
        let mut scanner = Scanner::new();
        let mut a = crate::tests::agent_for(321, 1, Some("ttys002"));
        a.session_match = SessionMatch::Guessed;
        a.start_time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 5;
        scanner.claude_paths.insert(a.pid, (previous.clone(), false));
        let h = scanner.history(&a).unwrap().unwrap();
        assert!(h.messages.is_empty() && h.source.is_none(), "the earlier process's conversation isn't shown");
        // Once written after the process started, it's this agent's (a resumed session).
        a.start_time = 0;
        assert_eq!(scanner.history(&a).unwrap().unwrap().messages[0].text, "old task");
        // An exact match needs no such evidence.
        let mut exact = crate::tests::agent_for(322, 1, Some("ttys003"));
        exact.start_time = a.start_time + u64::MAX / 2;
        scanner.claude_paths.insert(exact.pid, (previous, true));
        assert_eq!(scanner.history(&exact).unwrap().unwrap().messages.len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Within one second: a file the previous run wrote 200 ms before this process started is
    /// still before it, and one written 200 ms after is this process's.
    #[test]
    fn the_ownership_check_uses_the_start_to_the_millisecond() {
        let root = std::env::temp_dir().join(format!("lantern-owned-ms-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("s.jsonl");
        std::fs::write(&path, "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"same words\"}}\n").unwrap();
        let written = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_790_000_000_400);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(written).unwrap();
        let mut scanner = Scanner::new();
        let mut a = crate::tests::agent_for(323, 1, Some("ttys004"));
        a.session_match = SessionMatch::Guessed;
        a.start_time = 1_790_000_000;
        scanner.claude_paths.insert(a.pid, (path.clone(), false));
        scanner.started_ms.insert(a.pid, (a.start_time, 1_790_000_000_600));
        let before = scanner.history(&a).unwrap().unwrap();
        assert!(before.messages.is_empty() && before.source.is_none(), "written before it started, in the same second");
        scanner.started_ms.insert(a.pid, (a.start_time, 1_790_000_000_200));
        assert_eq!(scanner.history(&a).unwrap().unwrap().messages.len(), 1, "written after it started");
        // Without a millisecond start, the whole second it started in.
        scanner.started_ms.remove(&a.pid);
        assert_eq!(scanner.history(&a).unwrap().unwrap().messages.len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
