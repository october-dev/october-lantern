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
    proc_cache: ProcCache,
    /// Agent pid → the child process that holds its session file open (Codex's native binary).
    children: HashMap<u32, u32>,
    transcripts: Transcripts,
    /// Remember the transcript each Claude process was matched to (and whether exactly), so
    /// guesses stay stable.
    claude_paths: HashMap<u32, (PathBuf, bool)>,
    /// Handle numbers stay with an agent for its whole life; a new agent takes the lowest free one.
    handles: HashMap<u32, (Kind, usize)>,
    opencode_cache: HashMap<u32, ((u64, u64), SessionStatus)>,
    /// Working directory and start time per agent, for readers that match sessions by them.
    started: HashMap<u32, (PathBuf, u64)>,
    /// App sessions (Codex app threads, recent Claude Desktop/Cowork/Codex app sessions) by agent
    /// id: their session file, for history.
    app_paths: HashMap<String, (Kind, PathBuf)>,
    /// Handle numbers for app sessions, by agent id ("claude-app-2", "codex-app-1").
    app_handles: HashMap<String, (String, usize)>,
    /// App sessions that aren't running, re-read every 30 s.
    recent: Option<(std::time::Instant, Vec<crate::apps::Recent>)>,
}

/// Arguments that mean the process is a helper or a headless run, not an interactive agent.
fn is_helper(kind: Kind, args: &[String]) -> bool {
    let (subcommands, flags): (&[&str], &[&str]) = match kind {
        Kind::Claude => (&["daemon", "mcp"], &["--bg-pty-host", "--bg-spare", "--chrome-native-host", "-p", "--print"]),
        Kind::Codex => (&["exec", "e", "mcp", "mcp-server", "app-server", "login", "logout", "proto"], &[]),
        Kind::Opencode => (&["serve", "run", "mcp"], &[]),
        _ => (&["mcp", "serve", "exec", "run", "login"], &["-p", "--print", "--prompt"]),
    };
    args.first().is_some_and(|a| subcommands.contains(&a.as_str())) || args.iter().any(|a| flags.contains(&a.as_str()))
}

const INTERPRETERS: [&str; 5] = ["node", "bun", "deno", "python", "uv"];

/// Classifies a process from argv. `node /path/to/codex ...` counts as codex.
pub fn classify(p: &Proc) -> Option<Kind> {
    let argv0 = p.cmd.first().map(|a| basename(a)).unwrap_or(p.name.as_str());
    let wrapped = INTERPRETERS.iter().any(|i| argv0.starts_with(i));
    let (prog, rest) =
        if wrapped { (p.cmd.get(1).map(|a| basename(a))?, p.cmd.get(2..).unwrap_or(&[])) } else { (argv0, p.cmd.get(1..).unwrap_or(&[])) };
    let kind = match Kind::from_program(prog) {
        // GitHub's Copilot CLI is a node program; a native `copilot` is AWS's deploy tool.
        Some(Kind::Copilot) if !wrapped => return None,
        Some(kind) => kind,
        None if p.exe.as_ref().is_some_and(|e| e.to_string_lossy().contains("/.local/share/claude/versions/")) => Kind::Claude,
        None => return None,
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
            proc_cache: ProcCache::default(),
            children: HashMap::new(),
            transcripts: Transcripts::default(),
            claude_paths: HashMap::new(),
            handles: HashMap::new(),
            opencode_cache: HashMap::new(),
            started: HashMap::new(),
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
        let events = read_events();
        let live: Vec<u32> = agents.iter().map(|(p, _)| p.pid).collect();
        // Keep open-file lookups for agents and their children (Codex's file is held by the child).
        let children: Vec<u32> = live.iter().filter_map(|pid| table.child_of(*pid)).collect();
        self.transcripts.forget_pids(&[live.clone(), children].concat());
        self.claude_paths.retain(|pid, _| live.contains(pid));
        self.handles.retain(|pid, (kind, _)| agents.iter().any(|(p, k)| p.pid == *pid && k == kind));
        self.opencode_cache.retain(|pid, _| live.contains(pid));
        self.children = live.iter().filter_map(|pid| table.child_of(*pid).map(|c| (*pid, c))).collect();
        self.started = agents.iter().filter_map(|(p, _)| p.cwd.clone().map(|c| (p.pid, (c, p.start_time)))).collect();

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
            let host = match &tmux_pane {
                Some(pane) => {
                    let session = pane.target.split(':').next().unwrap_or("").to_string();
                    tmux.as_ref().and_then(|t| t.clients.get(&(pane.socket.clone(), session))).and_then(|client| host_app(&table, *client))
                }
                None => host_app(&table, p.pid),
            };
            let route = self.route(p.pid, p.tty.as_deref(), tmux_pane.is_some(), host.as_ref());
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

    /// How Lantern can type into this agent: tmux first (it's exact), then the terminal app's own
    /// mechanism.
    fn route(&self, pid: u32, tty: Option<&str>, in_tmux: bool, host: Option<&HostApp>) -> Route {
        if in_tmux {
            return Route::Tmux;
        }
        let app = host.map(|h| h.app.as_str()).unwrap_or("");
        let env = |key: &str| -> Option<String> {
            let prefix = format!("{key}=");
            self.sys
                .process(Pid::from_u32(pid))?
                .environ()
                .iter()
                .find_map(|e| e.to_str().and_then(|e| e.strip_prefix(&prefix)).filter(|v| !v.is_empty()).map(String::from))
        };
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
            question: None,
            question_kind: None,
            question_detail: None,
            prompt_id: None,
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

    /// The conversation for the chat view. `None` when Lantern can't read this harness's sessions.
    pub fn history(&mut self, agent: &Agent) -> Option<Vec<crate::history::ChatMessage>> {
        if let Some((kind, path)) = self.app_paths.get(&agent.id) {
            return Some(match kind {
                Kind::Codex => crate::history::codex(path),
                _ => crate::history::claude(path),
            });
        }
        match agent.kind {
            _ if agent.session_match == SessionMatch::Ambiguous => None,
            Kind::Claude => Some(self.claude_paths.get(&agent.pid).map(|(p, _)| crate::history::claude(p)).unwrap_or_default()),
            Kind::Codex => {
                let path = self
                    .transcripts
                    .codex_path_for_pid(agent.pid)
                    .or_else(|| self.children.get(&agent.pid).copied().and_then(|c| self.transcripts.codex_path_for_pid(c)));
                Some(path.map(|p| crate::history::codex(&p)).unwrap_or_default())
            }
            Kind::Pi | Kind::October | Kind::Gemini | Kind::Opencode => {
                let (cwd, start) = self.started.get(&agent.pid).cloned()?;
                Some(match agent.kind {
                    Kind::Opencode => crate::readers::opencode::history(&cwd, start),
                    Kind::Gemini => {
                        crate::readers::gemini::session_file(&cwd, start).map(|p| crate::readers::gemini::history(&p)).unwrap_or_default()
                    }
                    k => crate::readers::pi::session_file(&cwd, start, k == Kind::October)
                        .map(|p| crate::readers::pi::history(&p))
                        .unwrap_or_default(),
                })
            }
            _ => None,
        }
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
                let from_args = arg_after(args, &["--session-id", "--resume", "-r"])
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
                let path =
                    self.transcripts.codex_path_for_pid(p.pid).or_else(|| child.and_then(|c| self.transcripts.codex_path_for_pid(c)));
                match path {
                    Some(path) => found(self.transcripts.codex_status(&path), SessionMatch::Exact),
                    None => (None, SessionMatch::None),
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
/// (a permission prompt) is kept.
pub(crate) fn apply_live(status: &mut SessionStatus, source: StateSource, live: &crate::apps::ClaudeLive) {
    if source == StateSource::Hook && status.state == Some(State::NeedsInput) {
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
            (s, if hook_newer { StateSource::Hook } else { StateSource::Transcript })
        }
        (Some(h), None) => (h, StateSource::Hook),
        (None, Some(t)) => (t, StateSource::Transcript),
        (None, None) => (SessionStatus::default(), StateSource::None),
    }
}
