//! Tests for the parts that touch other programs' files and processes: session parsing, hook
//! installation, and the checks made before typing into a terminal.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::model::{Agent, Kind, QuestionKind, Route, State, StateSource};
use crate::procs::Proc;
use crate::{deliver, history, hooks, scanner, transcripts};

/// A folder under the system temp dir, removed when the test ends (even when it fails): tests
/// that point HOME here install hooks, which copies the engine binary in.
struct TempDir(PathBuf);
impl std::ops::Deref for TempDir {
    type Target = PathBuf;
    fn deref(&self) -> &PathBuf {
        &self.0
    }
}
impl AsRef<std::path::Path> for TempDir {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(name: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!("lantern-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    TempDir(dir)
}

#[cfg(windows)]
#[test]
fn classifies_windows_command_lines() {
    let claude = r"C:\Users\x\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\cli.js";
    assert_eq!(scanner::classify(&proc(&[r"C:\Program Files\nodejs\node.exe", claude])), Some(Kind::Claude));
    assert_eq!(scanner::classify(&proc(&[r"C:\Users\x\.local\bin\claude.exe"])), Some(Kind::Claude));
    assert_eq!(scanner::classify(&proc(&[r"C:\Users\x\.local\bin\CLAUDE.EXE", "-p", "hi"])), None);
    let codex = r"C:\Users\x\AppData\Roaming\npm\node_modules\@openai\codex\bin\codex.js";
    assert_eq!(scanner::classify(&proc(&["node.exe", codex, "resume"])), Some(Kind::Codex));
    assert_eq!(scanner::classify(&proc(&["node.exe", r"C:\work\cli.js"])), None);
}

fn proc(cmd: &[&str]) -> Proc {
    Proc {
        pid: 1000,
        ppid: Some(1),
        name: cmd[0].rsplit('/').next().unwrap().into(),
        exe: Some(PathBuf::from(cmd[0])),
        cmd: cmd.iter().map(|s| s.to_string()).collect(),
        cwd: None,
        start_time: 0,
        tty: Some("ttys001".into()),
    }
}

#[test]
fn classifies_agents_and_skips_helpers() {
    assert_eq!(scanner::classify(&proc(&["/Users/x/.local/bin/claude", "--session-id", "abc"])), Some(Kind::Claude));
    assert_eq!(scanner::classify(&proc(&["node", "/opt/homebrew/bin/codex", "resume", "x"])), Some(Kind::Codex));
    assert_eq!(scanner::classify(&proc(&["/opt/homebrew/bin/opencode"])), Some(Kind::Opencode));
    assert_eq!(scanner::classify(&proc(&["node", "/usr/local/bin/pi"])), Some(Kind::Pi));
    assert_eq!(scanner::classify(&proc(&["/Users/x/.local/bin/cursor-agent"])), Some(Kind::Cursor));
    assert_eq!(scanner::classify(&proc(&["/Users/x/.local/bin/muse-bin-1.4.6-R6000.1"])), Some(Kind::Muse));
    assert_eq!(scanner::classify(&proc(&["/Users/x/.local/bin/muse-bin-1.4.6-R6000.1", "exec", "hi"])), None);
    // Helpers and headless runs aren't agents you can answer.
    assert_eq!(scanner::classify(&proc(&["/Users/x/.local/bin/claude", "daemon", "run"])), None);
    assert_eq!(scanner::classify(&proc(&["/Users/x/.local/bin/claude", "-p", "hi"])), None);
    assert_eq!(scanner::classify(&proc(&["/opt/homebrew/bin/codex", "exec", "hi"])), None);
    // A native `copilot` is AWS's deploy tool, not GitHub Copilot CLI.
    assert_eq!(scanner::classify(&proc(&["/usr/local/bin/copilot", "deploy"])), None);
    assert_eq!(scanner::classify(&proc(&["/bin/zsh"])), None);
}

#[test]
fn claude_transcript_states() {
    let dir = temp_dir("claude");
    let path = dir.join("s.jsonl");
    let user = r#"{"type":"user","timestamp":"2026-09-23T16:51:00.000Z","message":{"role":"user","content":"fix it"}}"#;
    // One API message, written as two entries that share its final stop_reason.
    let text_part = r#"{"type":"assistant","timestamp":"2026-09-23T16:51:05.000Z","message":{"id":"m1","stop_reason":"tool_use","content":[{"type":"text","text":"Running the tests."}]}}"#;
    let tool = r#"{"type":"assistant","timestamp":"2026-09-23T16:51:05.100Z","message":{"id":"m1","stop_reason":"tool_use","content":[{"type":"tool_use","name":"Bash","input":{"command":"npm test"}}]}}"#;
    let result = r#"{"type":"user","timestamp":"2026-09-23T16:51:06.000Z","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#;
    let done = r#"{"type":"assistant","timestamp":"2026-09-23T16:51:09.000Z","message":{"id":"m2","stop_reason":"end_turn","content":[{"type":"text","text":"All fixed."}]}}"#;
    let title = r#"{"type":"ai-title","aiTitle":"Fix the test","sessionId":"s1"}"#;

    // A text-only entry of a message that goes on to use tools is not the end of the turn.
    fs::write(&path, format!("{user}\n{text_part}\n")).unwrap();
    assert_eq!(transcripts::parse_claude(&path, 0).state, Some(State::Working));
    fs::write(&path, format!("{user}\n{text_part}\n{tool}\n")).unwrap();
    assert_eq!(transcripts::parse_claude(&path, 0).state, Some(State::Working));

    fs::write(&path, format!("{user}\n{text_part}\n{tool}\n{result}\n{done}\n{title}\n")).unwrap();
    let s = transcripts::parse_claude(&path, 99);
    assert_eq!(s.state, Some(State::Waiting));
    assert_eq!(s.last_message.as_deref(), Some("All fixed."));
    assert_eq!(s.title.as_deref(), Some("Fix the test"));
    // `since` is the entry's own time, so a title written later doesn't start a new turn.
    assert_eq!(s.since, Some(1_790_182_269_000));

    let chat = history::claude(&path);
    let roles: Vec<String> = chat.iter().map(|m| format!("{:?}", m.role)).collect();
    assert_eq!(roles, ["User", "Agent", "Tool", "Agent"]);
    assert_eq!(chat[2].text, "Bash · npm test");

    fs::write(&path, "").unwrap();
    assert_eq!(transcripts::parse_claude(&path, 0).state, Some(State::Idle));
}

#[test]
fn codex_rollout_states() {
    let dir = temp_dir("codex");
    let path = dir.join("rollout.jsonl");
    let meta = r#"{"type":"session_meta","payload":{"id":"t1"}}"#;
    let env = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>x</environment_context>"}]}}"#;
    let ask =
        r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"write tests"}]}}"#;
    let started = r#"{"type":"event_msg","timestamp":"2026-09-23T16:51:06.609Z","payload":{"type":"task_started","turn_id":"u1","started_at":1790182266}}"#;
    let done = r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"u1","last_agent_message":"Tests written.","completed_at":1790183062}}"#;

    fs::write(&path, format!("{meta}\n{env}\n{ask}\n{started}\n")).unwrap();
    let s = transcripts::parse_codex(&path, 0);
    assert_eq!(s.state, Some(State::Working));
    assert_eq!(s.title.as_deref(), Some("write tests"));
    assert_eq!(s.session_id.as_deref(), Some("t1"));
    assert_eq!(s.since, Some(1_790_182_266_000));

    fs::write(&path, format!("{meta}\n{env}\n{ask}\n{started}\n{done}\n")).unwrap();
    let s = transcripts::parse_codex(&path, 0);
    assert_eq!(s.state, Some(State::Waiting));
    assert_eq!(s.last_message.as_deref(), Some("Tests written."));
    assert_eq!(s.since, Some(1_790_183_062_000));

    // A turn longer than the tail: the boundary event is out of reach, so the state is unknown
    // rather than a confident "idle".
    let filler = format!("{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"item_completed\",\"pad\":\"{}\"}}}}\n", "x".repeat(4000));
    let mut long = format!("{meta}\n{ask}\n{started}\n");
    for _ in 0..(transcripts::TAIL_BYTES / 4000 + 8) {
        long.push_str(&filler);
    }
    fs::write(&path, long).unwrap();
    assert_eq!(transcripts::parse_codex(&path, 0).state, Some(State::Unknown));
}

#[test]
fn codex_code_mode_tool_calls() {
    assert_eq!(
        history::describe_code_call("exec", r#"text(await tools.exec_command({cmd:"npm test -- --watch=false",max_output_tokens:2000}));"#),
        "Run · npm test -- --watch=false"
    );
    assert_eq!(history::describe_code_call("exec", r#"text(await tools.apply_patch("*** Begin"));"#), "apply patch");
}

/// One test for everything that changes HOME, since tests share the process environment.
#[test]
fn hooks_install_uninstall_and_missing_app() {
    let home = temp_dir("home");
    unsafe { std::env::set_var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, &*home) };
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::create_dir_all(home.join(".codex")).unwrap();
    let theirs = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]},"model":"opus"}"#;
    fs::write(home.join(".claude/settings.json"), theirs).unwrap();

    // A broken Codex config stops the whole install before Claude's file changes.
    fs::write(home.join(".codex/config.toml"), "this = = is not toml\n").unwrap();
    assert!(hooks::install().is_err());
    assert_eq!(fs::read_to_string(home.join(".claude/settings.json")).unwrap(), theirs);
    assert!(fs::read_dir(home.join(".claude")).unwrap().count() == 1, "no backup or temp file left behind");

    fs::write(home.join(".codex/config.toml"), "model = \"gpt\"\nnotify = [\"their-notifier\"]\n\n[tui]\nx = 1\n").unwrap();
    hooks::install().unwrap();
    let settings = fs::read_to_string(home.join(".claude/settings.json")).unwrap();
    assert!(settings.contains("say done"), "keeps the user's own hooks");
    assert!(settings.contains("\"model\": \"opus\""));
    assert_eq!(settings.matches("hook claude").count(), 6);
    assert!(settings.contains("PermissionRequest"));
    let config = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(config.contains("lantern-engine") && config.contains("[tui]"));
    assert!(hooks::support_dir().join("bin").join(format!("lantern-engine{}", std::env::consts::EXE_SUFFIX)).exists());

    // Installing twice doesn't duplicate anything, and the second backup doesn't overwrite the first.
    hooks::install().unwrap();
    assert_eq!(fs::read_to_string(home.join(".claude/settings.json")).unwrap().matches("hook claude").count(), 6);
    let backups = fs::read_dir(home.join(".claude"))
        .unwrap()
        .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains("lantern-backup"))
        .count();
    assert_eq!(backups, 2);

    // If Lantern is gone, the hook command still exits 0 and prints nothing.
    #[cfg(unix)]
    {
        let gone = home.join("nowhere/lantern-engine");
        let out = Command::new("/bin/sh").args(["-c", &hooks::claude_hook_command(&gone)]).output().unwrap();
        assert!(out.status.success() && out.stdout.is_empty());
        let codex = hooks::codex_notify_command(&gone);
        let out = Command::new(&codex[0]).args(&codex[1..]).arg("{\"type\":\"agent-turn-complete\"}").output().unwrap();
        assert!(out.status.success());
    }

    hooks::uninstall_everything().unwrap();
    let settings: serde_json::Value = serde_json::from_str(&fs::read_to_string(home.join(".claude/settings.json")).unwrap()).unwrap();
    let original: serde_json::Value = serde_json::from_str(theirs).unwrap();
    assert_eq!(settings, original, "settings are back to what the user had");
    let config = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(config.contains("their-notifier") && !config.contains("lantern-engine"));
    assert!(!hooks::support_dir().exists());

    // Claude's file is written, then Codex's write fails: Claude's file gets its old contents back.
    fs::remove_file(home.join(".codex/config.toml")).unwrap();
    fs::create_dir_all(home.join(".codex/config.toml/in-the-way")).unwrap();
    let before = fs::read_to_string(home.join(".claude/settings.json")).unwrap();
    assert!(hooks::install().is_err());
    assert_eq!(fs::read_to_string(home.join(".claude/settings.json")).unwrap(), before);

    // Codex's folder can't even be created (a file is in the way): same rollback.
    fs::remove_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".codex"), "not a folder").unwrap();
    assert!(hooks::install().is_err());
    assert_eq!(fs::read_to_string(home.join(".claude/settings.json")).unwrap(), before);
}

#[test]
fn pi_session_states() {
    use crate::readers::pi;
    let dir = temp_dir("pi");
    let path = dir.join("s.jsonl");
    let header = r#"{"type":"session","version":3,"id":"p1","cwd":"/x"}"#;
    let user = r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"add a flag"}]}}"#;
    let call = r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"read","arguments":{"path":"a.ts"}}],"stopReason":"toolUse"}}"#;
    let result = r#"{"type":"message","message":{"role":"toolResult","toolName":"read","content":[]}}"#;
    let done = r#"{"type":"message","timestamp":"2026-09-21T14:13:20.123Z","message":{"role":"assistant","content":[{"type":"text","text":"Added --flag."}],"stopReason":"stop"}}"#;

    fs::write(&path, format!("{header}\n{user}\n{call}\n")).unwrap();
    assert_eq!(pi::parse(&path, 0).state, Some(State::Working));
    fs::write(&path, format!("{header}\n{user}\n{call}\n{result}\n{done}\n")).unwrap();
    let s = pi::parse(&path, 0);
    assert_eq!(s.state, Some(State::Waiting));
    assert_eq!(s.last_message.as_deref(), Some("Added --flag."));
    assert_eq!(s.title.as_deref(), Some("add a flag"));
    assert_eq!(s.session_id.as_deref(), Some("p1"));
    assert_eq!(s.since, Some(1_790_000_000_123));
    let chat = pi::history(&path);
    assert_eq!(chat.len(), 3);
    assert_eq!(chat[1].text, "read · a.ts");
}

#[test]
fn gemini_replay_and_states() {
    use crate::readers::gemini;
    let dir = temp_dir("gemini");
    let path = dir.join("session.jsonl");
    let header = r#"{"sessionId":"g1","startTime":"2026-01-01T00:00:00Z","kind":"main"}"#;
    let context = r#"{"$set":{"messages":[{"id":"c","type":"user","content":[{"text":"<session_context>x"}]}]}}"#;
    let ask = r#"{"id":"u1","type":"user","content":[{"text":"rename the file"}]}"#;
    let partial = r#"{"id":"g1","type":"gemini","content":"","toolCalls":[{"name":"run_shell_command","args":{"command":"mv a b"}}]}"#;
    let tool_result = r#"{"id":"u2","type":"user","content":[{"functionResponse":{}}]}"#;
    let reply = r#"{"id":"g2","type":"gemini","timestamp":"2026-09-21T14:13:20.123Z","content":"Renamed."}"#;

    fs::write(&path, format!("{header}\n{context}\n")).unwrap();
    assert_eq!(gemini::parse(&path, 0).state, Some(State::Idle));
    fs::write(&path, format!("{header}\n{context}\n{ask}\n{partial}\n{tool_result}\n")).unwrap();
    assert_eq!(gemini::parse(&path, 0).state, Some(State::Working));
    fs::write(&path, format!("{header}\n{context}\n{ask}\n{partial}\n{tool_result}\n{reply}\n")).unwrap();
    let s = gemini::parse(&path, 0);
    assert_eq!(s.state, Some(State::Waiting));
    assert_eq!(s.last_message.as_deref(), Some("Renamed."));
    assert_eq!(s.title.as_deref(), Some("rename the file"));
    assert_eq!(s.since, Some(1_790_000_000_123));
    // A rewind drops everything from that message on.
    let rewind = r#"{"$rewindTo":"g2"}"#;
    fs::write(&path, format!("{header}\n{context}\n{ask}\n{partial}\n{tool_result}\n{reply}\n{rewind}\n")).unwrap();
    assert_eq!(gemini::parse(&path, 0).state, Some(State::Working));
}

pub(crate) fn agent_for(pid: u32, start_time: u64, tty: Option<&str>) -> Agent {
    Agent {
        id: format!("claude:{pid}:{start_time}"),
        kind: Kind::Claude,
        handle: "claude-1".into(),
        pid,
        start_time,
        exe: crate::procs::live(pid).and_then(|l| l.exe),
        comm: crate::procs::live(pid).map(|l| l.comm).unwrap_or_default(),
        tty: tty.map(String::from),
        cwd: None,
        project: None,
        title: None,
        session_id: None,
        state: State::Waiting,
        state_since: None,
        last_message: None,
        question: None,
        question_kind: None,
        question_detail: None,
        prompt_id: None,
        session_match: crate::model::SessionMatch::Exact,
        source: None,
        live: true,
        host: None,
        tmux: None,
        can_reply: true,
        route: Route::Tmux,
        state_source: StateSource::None,
    }
}

/// Typing goes to a process, not a terminal: an agent that exited (or a reused pid, or a
/// different terminal) is refused before anything is sent.
#[cfg(unix)]
#[test]
fn delivery_refuses_targets_that_are_not_the_agent_any_more() {
    // A child that exits at once, so its pid is gone (or reused by something with another start).
    let child = Command::new("/usr/bin/true").spawn().unwrap();
    let pid = child.id();
    let _ = child.wait_with_output();
    let gone = deliver::verify(&agent_for(pid, 1, None)).unwrap_err().to_string();
    assert!(gone.contains("exited"), "{gone}");

    // This test process is alive, but not with that start time or on that tty.
    let me = std::process::id();
    let live = crate::procs::live(me).unwrap();
    assert!(deliver::verify(&agent_for(me, live.start + 1, None)).unwrap_err().to_string().contains("another process"));
    assert!(deliver::verify(&agent_for(me, live.start, Some("ttys999"))).unwrap_err().to_string().contains("terminal"));
    // No terminal to check against: refused, whatever the foreground.
    assert!(deliver::verify(&agent_for(me, live.start, None)).unwrap_err().to_string().contains("no terminal"));
}

fn live_like(agent: &Agent) -> crate::procs::Live {
    crate::procs::Live {
        pgid: 7,
        tpgid: 7,
        tty: agent.tty.clone(),
        start: agent.start_time,
        exe: agent.exe.clone(),
        comm: agent.comm.clone(),
    }
}

/// Everything the check refuses, one rule at a time, against a reading that otherwise passes.
#[test]
fn delivery_refuses_unknown_owners_and_changed_endpoints() {
    let mut a = agent_for(1, 10, Some("ttys004"));
    a.exe = Some("/usr/local/bin/claude".into());
    a.comm = "claude".into();
    a.route = Route::Terminal { tty: "/dev/ttys004".into() };
    assert!(deliver::check(&a, &live_like(&a)).is_ok());

    let unknown_fg = crate::procs::Live { tpgid: 0, ..live_like(&a) };
    assert!(deliver::check(&a, &unknown_fg).unwrap_err().to_string().contains("can't tell"));
    let background = crate::procs::Live { tpgid: 8, ..live_like(&a) };
    assert!(deliver::check(&a, &background).unwrap_err().to_string().contains("foreground"));
    let shell = crate::procs::Live { exe: Some("/bin/zsh".into()), comm: "zsh".into(), ..live_like(&a) };
    assert!(deliver::check(&a, &shell).unwrap_err().to_string().contains("no longer running its agent"));
    // The executable can't be read (deleted by an update): the kernel's name must still match.
    let unreadable = crate::procs::Live { exe: None, ..live_like(&a) };
    assert!(deliver::check(&a, &unreadable).is_ok());
    let unreadable_shell = crate::procs::Live { exe: None, comm: "zsh".into(), ..live_like(&a) };
    assert!(deliver::check(&a, &unreadable_shell).is_err());
    let mut moved = a.clone();
    moved.route = Route::Terminal { tty: "/dev/ttys009".into() };
    assert!(deliver::check(&moved, &live_like(&moved)).unwrap_err().to_string().contains("tab changed"));
}

/// A Lantern session (Windows) has no foreground to check; the agent must be the session's own
/// process, alive, on the session's console.
#[test]
fn session_delivery_requires_the_sessions_own_agent() {
    use crate::pty::Info;
    let mut a = agent_for(40, 10, Some("con:900"));
    a.exe = Some(r"C:\Program Files\nodejs\node.exe".into());
    a.comm = "node.exe".into();
    a.route = Route::Pty { session: "claude-app-1".into() };
    // On Windows the foreground is never known; that must not matter here.
    let live = crate::procs::Live { tpgid: 0, pgid: 0, ..live_like(&a) };
    let info = Info { child_pid: 40, child_start: 10, child_tty: Some("con:900".into()), alive: true };
    assert!(deliver::check_session(&a, &live, &info, &[]).is_ok());
    // Started through a wrapper the session ran: still the session's agent.
    let wrapper = Info { child_pid: 39, ..info.clone() };
    assert!(deliver::check_session(&a, &live, &wrapper, &[39, 4]).is_ok());
    assert!(deliver::check_session(&a, &live, &wrapper, &[4]).unwrap_err().to_string().contains("isn't the program"));
    let ended = Info { alive: false, ..info.clone() };
    assert!(deliver::check_session(&a, &live, &ended, &[]).unwrap_err().to_string().contains("ended"));
    let other_console = Info { child_tty: Some("con:901".into()), ..info.clone() };
    assert!(deliver::check_session(&a, &live, &other_console, &[]).unwrap_err().to_string().contains("console"));
    let reused = crate::procs::Live { start: 11, ..live_like(&a) };
    assert!(deliver::check_session(&a, &reused, &info, &[]).unwrap_err().to_string().contains("another process"));
    // The ordinary terminal check still refuses an unknown foreground.
    a.route = Route::None;
    assert!(deliver::check(&a, &live).is_err());
}

/// The process audit's counterexample: same pid, same start time, same terminal, but the agent
/// `exec`ed a shell. The kernel reports the new program, and delivery refuses.
#[cfg(unix)]
#[test]
fn delivery_refuses_a_process_that_execed_something_else() {
    let mut child = Command::new("/bin/sh").args(["-c", "/bin/sleep 0.4; exec /bin/sleep 5"]).spawn().unwrap();
    let pid = child.id();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let before = crate::procs::live(pid).unwrap();
    let mut a = agent_for(pid, before.start, Some("ttys004"));
    a.exe = before.exe.clone();
    a.comm = before.comm.clone();
    std::thread::sleep(std::time::Duration::from_millis(900));
    let after = crate::procs::live(pid).unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!((after.start, after.pgid), (before.start, before.pgid), "same process");
    assert_ne!(after.exe, before.exe, "the kernel reports the new program");
    let reading = crate::procs::Live { tty: Some("ttys004".into()), tpgid: after.pgid, ..after };
    assert!(deliver::check(&a, &reading).unwrap_err().to_string().contains("no longer running its agent"));
}

/// A phone reply that couldn't start in time is canceled: when the stalled worker gets to it,
/// it types nothing. One that started is reported as uncertain, never as "not accepted".
#[test]
fn stalled_deliveries_are_canceled_not_typed_later() {
    use crate::actions::{Op, Outcome, Ticket, perform};
    use crate::serve::Incoming;
    let (tx, rx) = std::sync::mpsc::channel();
    let a = agent_for(std::process::id(), 0, Some("ttys004"));
    // Nobody serves the queue: the host's wait runs out (at the phone's 300 ms deadline).
    let outcome = crate::mobile::host::deliver_and_wait(&tx, &a, "rm -rf build", Some(hooks::now_ms() + 300));
    assert!(matches!(outcome, Outcome::Failed { code: "expired", .. }), "{outcome:?}");
    // The worker resumes and finds the queued reply: canceled, nothing typed.
    let Ok(Incoming::Deliver(d)) = rx.try_recv() else { panic!("the reply was queued") };
    let link = crate::october_link::detached();
    let far = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let o = perform(&a, Op::Text(d.text), far, &d.ticket, &link);
    assert!(matches!(o, Outcome::Failed { code: "canceled", .. }), "{o:?}");
    // Past its deadline: expired, whatever the ticket says.
    let o = perform(&a, Op::Text("x".into()), std::time::Instant::now(), &Ticket::new(), &link);
    assert!(matches!(o, Outcome::Failed { code: "expired", .. }), "{o:?}");
    // A started action can't be canceled any more; the waiter must wait for its outcome.
    let t = Ticket::new();
    assert!(t.start());
    assert!(!t.cancel());
}

#[test]
fn permission_keys_must_name_the_current_prompt() {
    use crate::deliver::Key;
    let mut a = agent_for(1, 1, Some("ttys004"));
    a.question_kind = Some(QuestionKind::Permission);
    a.prompt_id = Some("p2".into());
    assert!(crate::serve::keys_for(&a, vec![Key::Char('1')], None).is_err());
    assert!(crate::serve::keys_for(&a, vec![Key::Char('1')], Some("p1".into())).is_err());
    assert!(crate::serve::keys_for(&a, vec![Key::Char('1')], Some("p2".into())).is_ok());
}

#[test]
fn permission_prompts_keep_the_whole_command() {
    let v = serde_json::json!({
        "hook_event_name": "PermissionRequest", "session_id": "s", "tool_name": "Bash",
        "tool_input": {"command": "echo harmless\nrm -rf ~/important"}
    });
    let ev = hooks::claude_event(&v).unwrap();
    let q = ev.question.unwrap();
    assert!(q.contains("echo harmless") && q.contains("+1 more line"), "{q}");
    assert!(ev.question_detail.unwrap().contains("rm -rf ~/important"));
    assert!(ev.prompt_id.is_some());
}

/// Stop, then Claude's idle reminder (twice): one finished turn, one time. A new turn after
/// work starts again gets a new time.
#[test]
fn repeated_hook_events_keep_the_turn() {
    let ev = |state: State, message: Option<&str>, at: u64| hooks::HookEvent {
        source: "claude".into(),
        state,
        session_id: Some("s".into()),
        cwd: None,
        message: message.map(String::from),
        question: None,
        question_kind: None,
        question_detail: None,
        prompt_id: None,
        transcript_path: None,
        ancestors: vec![1],
        at,
    };
    let stop = ev(State::Waiting, Some("Done."), 100);
    let idle = hooks::continue_turn(Some(&stop), ev(State::Waiting, None, 60_100));
    assert_eq!((idle.at, idle.message.as_deref()), (100, Some("Done.")));
    let again = hooks::continue_turn(Some(&idle), ev(State::Waiting, None, 120_100));
    assert_eq!(again.at, 100);
    let working = hooks::continue_turn(Some(&again), ev(State::Working, None, 130_000));
    let next = hooks::continue_turn(Some(&working), ev(State::Waiting, Some("Done."), 140_000));
    assert_eq!(next.at, 140_000);
}

#[test]
fn backups_and_temporary_files_never_collide() {
    let dir = temp_dir("backups");
    let file = dir.join("settings.json");
    fs::write(&file, "{}").unwrap();
    for _ in 0..100 {
        hooks::backup(&file).unwrap();
    }
    let backups = fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains("lantern-backup")).count();
    assert_eq!(backups, 100);

    let target = dir.join("config.toml");
    let writers: Vec<_> = (0..8)
        .map(|i| {
            let target = target.clone();
            std::thread::spawn(move || {
                (0..25).map(|_| hooks::write_atomic(&target, format!("writer {i}").as_bytes())).collect::<Result<Vec<_>, _>>()
            })
        })
        .collect();
    for w in writers {
        w.join().unwrap().unwrap();
    }
    assert!(fs::read_to_string(&target).unwrap().starts_with("writer "));
    let leftovers = fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains("lantern-tmp")).count();
    assert_eq!(leftovers, 0);
}

#[test]
fn same_folder_guesses_are_hidden() {
    let mut a = agent_for(1, 1, Some("ttys001"));
    a.kind = Kind::Pi;
    a.cwd = Some("/p".into());
    a.session_match = crate::model::SessionMatch::Guessed;
    a.last_message = Some("theirs?".into());
    let mut b = a.clone();
    b.pid = 2;
    let mut c = a.clone();
    c.pid = 3;
    c.cwd = Some("/other".into());
    let mut agents = vec![a, b, c];
    scanner::hide_ambiguous(&mut agents);
    assert_eq!(agents[0].session_match, crate::model::SessionMatch::Ambiguous);
    assert_eq!((agents[1].last_message.as_deref(), agents[1].state), (None, State::Unknown));
    assert_eq!(agents[2].session_match, crate::model::SessionMatch::Guessed);
}

#[test]
fn hook_events_carry_typed_questions() {
    let ev = hooks::HookEvent {
        source: "claude".into(),
        state: State::NeedsInput,
        session_id: Some("s".into()),
        cwd: None,
        message: None,
        question: Some("Permission to run Bash · rm -rf node_modules".into()),
        question_kind: Some(QuestionKind::Permission),
        question_detail: None,
        prompt_id: Some("p1".into()),
        transcript_path: None,
        ancestors: vec![1],
        at: 5,
    };
    let s = ev.status();
    assert_eq!((s.state, s.question_kind, s.since), (Some(State::NeedsInput), Some(QuestionKind::Permission), Some(5)));
    let json = serde_json::to_string(&ev).unwrap();
    assert!(json.contains("\"questionKind\":\"permission\""));
}

/// Two prompts whose one-line summaries match but whose commands differ are two prompts.
#[test]
fn every_permission_request_gets_its_own_prompt_id() {
    let ask = |command: &str| {
        let v = serde_json::json!({
            "hook_event_name": "PermissionRequest", "session_id": "s", "tool_name": "Bash",
            "tool_input": {"command": command}
        });
        hooks::claude_event(&v).unwrap()
    };
    let first = ask("cd build\nmake clean");
    let second = hooks::continue_turn(Some(&first), ask("cd build\nrm -rf ~"));
    assert_eq!(first.question, second.question, "same summary");
    assert_ne!(first.prompt_id, second.prompt_id);
    assert!(second.question_detail.unwrap().contains("rm -rf ~"));
}

#[cfg(unix)]
#[test]
fn helpers_are_bounded_and_drained() {
    use std::time::{Duration, Instant};
    // Plenty of output: read while it runs, no false time-out.
    let out = crate::run::output(Command::new("/bin/sh").args(["-c", "head -c 300000 /dev/zero"]), Duration::from_secs(5)).unwrap();
    assert_eq!(out.stdout.len(), 300_000);
    // Too slow: killed at the limit.
    let t = Instant::now();
    let e = crate::run::output(Command::new("/bin/sleep").arg("5"), Duration::from_millis(100)).unwrap_err();
    assert!(e.downcast_ref::<crate::run::TimedOut>().is_some());
    assert!(t.elapsed() < Duration::from_millis(600), "{:?}", t.elapsed());
    // Exits at once, but leaves a child holding its output open: still ends at the limit.
    let t = Instant::now();
    let out =
        crate::run::output(Command::new("/bin/sh").args(["-c", "echo hi; /bin/sleep 5 & exit 0"]), Duration::from_millis(100)).unwrap();
    assert!(t.elapsed() < Duration::from_millis(600), "{:?}", t.elapsed());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
}

/// The same limits on Windows, where the helper and what it starts share a job object.
#[cfg(windows)]
#[test]
fn helpers_are_bounded_and_drained() {
    use std::time::{Duration, Instant};
    let out = crate::run::output(Command::new("cmd").args(["/d", "/c", "echo hi"]), Duration::from_secs(10)).unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    // Too slow: killed at the limit.
    let t = Instant::now();
    let e = crate::run::output(Command::new("ping").args(["-n", "6", "127.0.0.1"]), Duration::from_millis(200)).unwrap_err();
    assert!(e.downcast_ref::<crate::run::TimedOut>().is_some());
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    // Exits at once, but leaves a child holding its output open: still ends at the limit.
    let t = Instant::now();
    let out =
        crate::run::output(Command::new("cmd").args(["/d", "/c", "start /b ping -n 6 127.0.0.1 & echo hi"]), Duration::from_millis(500))
            .unwrap();
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert!(String::from_utf8_lossy(&out.stdout).contains("hi"));
}

/// A screenshot reaches each agent in a way it can use: attached (Codex), or readable without a
/// permission prompt (Claude Code, Gemini), and named in the first message for everyone.
#[test]
fn new_sessions_carry_the_screenshot() {
    use crate::launch::{Extras, agent_command, first_message};
    use std::path::Path;
    let shot = Path::new("/Users/me/Library/Application Support/October Lantern/screenshots/s1.png");
    let with_shot = Extras { screenshot: Some(shot), ..Default::default() };
    let msg = first_message(Some("fix the layout"), with_shot).unwrap();
    assert!(msg.starts_with("fix the layout") && msg.contains("s1.png"));
    assert!(first_message(None, with_shot).unwrap().contains("wait for my instructions"));
    assert_eq!(first_message(Some("hi"), Extras::default()).as_deref(), Some("hi"));
    // A task: your words, then the app context, the screenshot and the toolkit, in that order.
    let task = Extras { context: Some("Working in DaVinci Resolve"), toolkit: Some("- Media: ffmpeg"), ..with_shot };
    let full = first_message(Some("normalize the voice"), task).unwrap();
    let at = |s: &str| full.find(s).unwrap();
    assert!(at("normalize") < at("DaVinci") && at("DaVinci") < at("s1.png") && at("s1.png") < at("ffmpeg"), "{full}");
    assert_eq!(first_message(None, Extras { toolkit: Some("- x"), ..Default::default() }), None);

    let codex = agent_command(Kind::Codex, Path::new("/p"), Some(&msg), Some(shot), None, None);
    let (flags, prompt) = codex.split_once(" -- ").unwrap();
    assert!(flags.contains("--image") && flags.contains("s1.png"), "{codex}");
    assert!(prompt.contains("fix the layout"));
    let claude = agent_command(Kind::Claude, Path::new("/p"), Some(&msg), Some(shot), None, None);
    assert!(claude.split_once(" -- ").unwrap().0.contains("--add-dir"), "{claude}");
    let gemini = agent_command(Kind::Gemini, Path::new("/p"), Some(&msg), Some(shot), None, None);
    assert!(gemini.contains("--include-directories"));
    assert!(!agent_command(Kind::Claude, Path::new("/p"), Some("hi"), None, None, None).contains("--add-dir"));
}

#[test]
fn models_are_listed_and_passed() {
    use crate::models::{parse_bullets, parse_codex, parse_slashed, parse_table, valid};
    let codex = r#"{"models":[{"slug":"gpt-b","display_name":"GPT B","visibility":"list","priority":2},
        {"slug":"hidden","visibility":"hide","priority":0},{"slug":"gpt-a","display_name":"GPT A","visibility":"list","priority":1}]}"#;
    let ids: Vec<String> = parse_codex(codex).into_iter().map(|m| m.id).collect();
    assert_eq!(ids, ["gpt-a", "gpt-b"]);
    let grok = "Default model: grok-4.7\n\nAvailable models:\n  * grok-4.7 (default)\n  - grok-4.6\n";
    assert_eq!(parse_bullets(grok).len(), 2);
    let table = "provider    model     context  max-out\nopenrouter  ~anthropic/claude-opus-latest  1M  128K\n";
    let t = parse_table(table);
    assert_eq!((t.len(), t[0].id.as_str(), t[0].group.as_deref()), (1, "openrouter/~anthropic/claude-opus-latest", Some("openrouter")));
    assert_eq!(parse_slashed("opencode/big-pickle\nnoise line\n").len(), 1);
    assert!(valid("claude-opus-5-5[1m]") && valid("sonnet") && !valid("--dangerous") && !valid("a b") && !valid("x;rm"));

    let cmd = crate::launch::agent_command(Kind::Claude, std::path::Path::new("/p"), Some("hi"), None, Some("opus"), None);
    let (flags, _) = cmd.split_once(" -- ").unwrap();
    assert!(flags.contains("--model") && flags.contains("opus"), "{cmd}");
}

/// Agents that take a first message on the command line get it there (nothing to type later),
/// and Claude Code gets a session id so its conversation is matched from the start.
#[test]
fn first_messages_go_on_the_command_line() {
    use crate::launch::agent_command;
    use std::path::Path;
    for kind in [Kind::Grok, Kind::October, Kind::Pi, Kind::Gemini] {
        let cmd = agent_command(kind, Path::new("/p"), Some("fix it"), None, None, None);
        assert!(cmd.contains(" -- ") && cmd.contains("fix it"), "{cmd}");
    }
    let opencode = agent_command(Kind::Opencode, Path::new("/p"), Some("fix it"), None, None, None);
    assert!(opencode.contains("--prompt") && !opencode.contains(" -- "), "{opencode}");
    let claude = agent_command(Kind::Claude, Path::new("/p"), Some("fix it"), None, None, None);
    assert!(claude.contains("--session-id"), "{claude}");
}

#[test]
fn toolkit_list_keeps_the_users_notes() {
    use crate::toolkit::{Found, NOTES_HEADING, render};
    let found = Found {
        machine: Some("Apple M3, 16 GB memory".into()),
        tools: vec![("Media", vec!["ffmpeg".into()]), ("Cloud", vec![])],
        models: vec!["Ollama: llama3.2".into()],
        apps: vec!["DaVinci Resolve (scripting API in /Library/...)".into()],
    };
    let text = render(&found, "use my LUTs in ~/Grades\n");
    assert!(text.contains("- Media: ffmpeg") && !text.contains("Cloud"), "{text}");
    assert!(text.contains("Ollama: llama3.2") && text.contains("DaVinci Resolve"));
    assert_eq!(crate::toolkit::notes_of(&text).map(str::trim), Some("use my LUTs in ~/Grades"));
    // Rendering again from its own notes doesn't grow the file.
    let again = render(&found, crate::toolkit::notes_of(&text).unwrap());
    assert_eq!(again.matches(NOTES_HEADING).count(), 1);
    assert_eq!(again.len(), text.len());
}

/// Claude's status file: busy is working at once; idle ends a turn the transcript still shows as
/// running; a permission prompt from a hook stays.
#[test]
fn claude_status_files_give_live_state_and_app_labels() {
    use crate::apps::{ClaudeLive, claude_source};
    use crate::model::SessionStatus;
    let live = |status: &str| ClaudeLive {
        pid: 1,
        session_id: Some("s".into()),
        entrypoint: Some("cli".into()),
        status: Some(status.into()),
        status_updated_at: Some(500),
    };
    let working = SessionStatus { state: Some(State::Working), since: Some(100), last_message: Some("done".into()), ..Default::default() };
    let mut s = working.clone();
    crate::scanner::apply_live(&mut s, StateSource::Transcript, &live("idle"));
    assert_eq!((s.state, s.since), (Some(State::Waiting), Some(500)));
    let mut s = SessionStatus { state: Some(State::Waiting), since: Some(100), ..Default::default() };
    crate::scanner::apply_live(&mut s, StateSource::Transcript, &live("busy"));
    assert_eq!((s.state, s.since), (Some(State::Working), Some(500)));
    let mut s = SessionStatus { state: Some(State::NeedsInput), ..Default::default() };
    crate::scanner::apply_live(&mut s, StateSource::Hook, &live("busy"));
    assert_eq!(s.state, Some(State::NeedsInput));

    assert_eq!(claude_source(Some("claude-desktop")), Some("Claude Desktop"));
    assert_eq!(claude_source(Some("local-agent")), Some("Cowork"));
    assert_eq!(claude_source(Some("cli")), None);
}

#[test]
fn app_sessions_are_found_in_session_files_and_codex_index() {
    let dir = temp_dir("apps");
    let desktop = dir.join("d.jsonl");
    fs::write(&desktop, "{\"type\":\"summary\"}\n{\"entrypoint\":\"claude-desktop\",\"sessionId\":\"abc\",\"cwd\":\"/p\"}\n").unwrap();
    let r = crate::apps::claude_app_session(&desktop, 7).unwrap();
    assert_eq!((r.source, r.session_id.as_str(), r.cwd.as_deref()), ("Claude Desktop", "abc", Some("/p")));
    let cli = dir.join("c.jsonl");
    fs::write(&cli, "{\"entrypoint\":\"cli\",\"sessionId\":\"x\"}\n").unwrap();
    assert!(crate::apps::claude_app_session(&cli, 7).is_none());
    let rows = r#"[{"id":"t1","rollout_path":"/r.jsonl","cwd":"/w","title":"Fix it","updated":1790000000000}]"#;
    let t = crate::apps::parse_codex_threads(rows);
    assert_eq!((t[0].source, t[0].title.as_deref(), t[0].updated_ms), ("Codex app", Some("Fix it"), 1_790_000_000_000));
}

/// On October Bus: Claude gets an MCP file, OpenCode its config through the environment, the
/// October harness runs under the Bus's launcher, and the first message says so.
#[test]
fn bus_launches_reach_each_agent() {
    use crate::bus::Attach;
    use crate::launch::{Extras, agent_command, first_message};
    use std::path::Path;
    let claude = Attach {
        id: "lantern-claude-1".into(),
        name: "Claude".into(),
        args: vec!["--mcp-config".into(), "'/b/x.json'".into()],
        ..Default::default()
    };
    let cmd = agent_command(Kind::Claude, Path::new("/p"), Some("hi"), None, None, Some(&claude));
    let (flags, _) = cmd.split_once(" -- ").unwrap();
    assert!(flags.contains("--mcp-config"), "{cmd}");
    let opencode = Attach { env: vec![("OPENCODE_CONFIG".into(), "/b/o.json".into())], ..Default::default() };
    let cmd = agent_command(Kind::Opencode, Path::new("/p"), None, None, None, Some(&opencode));
    assert!(cmd.contains("OPENCODE_CONFIG=") && cmd.contains("exec opencode"), "{cmd}");
    let october = Attach { wrap: vec!["'/b/october-bus'".into(), "agent".into(), "run".into(), "--".into()], ..Default::default() };
    let cmd = agent_command(Kind::October, Path::new("/p"), None, None, None, Some(&october));
    assert!(cmd.contains("october-bus'\\'' agent run -- october"), "{cmd}");
    let note = crate::bus::note("Claude · web");
    let msg = first_message(Some("fix it"), Extras { bus_note: Some(&note), ..Default::default() }).unwrap();
    assert!(msg.starts_with("fix it") && msg.contains("October Bus"));
}

/// Gemini CLI and Qwen Code get a system settings file of Lantern's: the Mac's own system settings,
/// kept as they are, plus the Bus.
#[test]
fn bus_settings_keep_existing_system_settings() {
    let bus = br#"{"mcpServers":{"october_bus":{"command":"/b","args":["mcp","stdio"]}}}"#;
    let base = br#"{"general":{"vimMode":true},"mcpServers":{"theirs":{"command":"x"}}}"#;
    let merged: serde_json::Value = serde_json::from_slice(&crate::bus::merge_settings(Some(base), bus).unwrap()).unwrap();
    assert_eq!(merged["general"]["vimMode"], true);
    assert_eq!(merged["mcpServers"]["theirs"]["command"], "x");
    assert_eq!(merged["mcpServers"]["october_bus"]["command"], "/b");
    let alone: serde_json::Value = serde_json::from_slice(&crate::bus::merge_settings(None, bus).unwrap()).unwrap();
    assert_eq!(alone["mcpServers"]["october_bus"]["args"][0], "mcp");
    for kind in [Kind::Gemini, Kind::Qwen, Kind::Copilot, Kind::Goose, Kind::Claude, Kind::Codex, Kind::Opencode, Kind::October] {
        assert!(crate::bus::supported(kind), "{kind:?}");
    }
    assert!(!crate::bus::supported(Kind::Grok) && !crate::bus::supported(Kind::Cursor) && !crate::bus::supported(Kind::Pi));
}

/// Goose takes the Bus as one quoted `--with-extension` command; Copilot as an `@file`.
#[test]
fn bus_reaches_goose_and_copilot() {
    use crate::bus::Attach;
    use crate::launch::agent_command;
    use std::path::Path;
    let goose = Attach { args: vec!["session".into(), "--with-extension".into(), "'/b mcp stdio'".into()], ..Default::default() };
    let cmd = agent_command(Kind::Goose, Path::new("/p"), None, None, None, Some(&goose));
    assert!(cmd.contains("goose session --with-extension"), "{cmd}");
    let copilot = Attach { args: vec!["--additional-mcp-config".into(), "'@/b/c.json'".into()], ..Default::default() };
    let cmd = agent_command(Kind::Copilot, Path::new("/p"), Some("hi"), None, None, Some(&copilot));
    assert!(cmd.contains("--additional-mcp-config"), "{cmd}");
}

/// Open on an agent in October uses its canvas id from the environment, only when it's a UUID.
#[test]
fn october_canvas_ids() {
    use crate::scanner::is_canvas_id;
    assert!(is_canvas_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
    assert!(!is_canvas_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa"));
    assert!(!is_canvas_id("not-a-canvas"));
    assert!(!is_canvas_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa/"));
}

#[test]
fn audit_october_must_not_route_across_canvas_identity() {
    use crate::model::HostApp;
    use crate::october_core::OctoberAgent;
    use crate::october_link::{LinkState, merge};
    let mut a = agent_for(123, 1, Some("ttys001"));
    a.cwd = Some("/project".into());
    a.host = Some(HostApp {
        app: "October".into(),
        pid: 1,
        bundle_path: "/Applications/October.app".into(),
        canvas: Some("canvas-A".into()),
        node: None,
    });
    a.route = Route::None;
    a.can_reply = false;
    let link = LinkState {
        status: "connected".into(),
        paired: true,
        agents: vec![OctoberAgent {
            canvas_id: "canvas-B".into(),
            node_id: "other-agent".into(),
            kind: "terminal".into(),
            name: None,
            harness: Some("claude".into()),
            cwd: Some("/project".into()),
            state: "working".into(),
            attention: None,
        }],
        ..Default::default()
    };
    let mut agents = vec![a];
    merge(&mut agents, &link);
    assert_eq!(agents[0].route, Route::None, "A process on canvas A must never be sent to the same harness/folder on canvas B");
}

#[test]
fn october_messages_are_rejected_without_truncation() {
    let accepted = "界".repeat(8000);
    assert!(crate::october_core::validate_message(&accepted).is_ok());
    let oversized = accepted + "tail instructions";
    assert!(crate::october_core::validate_message(&oversized).is_err());
    assert!(oversized.ends_with("tail instructions"));
}

#[test]
fn expired_preparation_cannot_start_a_session() {
    use std::time::{Duration, Instant};
    assert!(crate::launch::check_launch_deadline(Instant::now() - Duration::from_secs(1)).is_err());
    assert!(crate::launch::check_launch_deadline(Instant::now() + Duration::from_secs(1)).is_ok());
}

/// Paired with October: an agent is matched to its own node exactly when its terminal is on the
/// Bus, and to the one node in its folder when October keeps it off the Bus.
#[test]
fn october_agents_match_their_node() {
    use crate::model::HostApp;
    use crate::october_core::OctoberAgent;
    use crate::october_link::{LinkState, merge};
    let node = |canvas: &str, id: &str| OctoberAgent {
        canvas_id: canvas.into(),
        node_id: id.into(),
        kind: "terminal".into(),
        name: Some(format!("@{id}")),
        harness: Some("claude".into()),
        cwd: Some("/project".into()),
        state: "working".into(),
        attention: None,
    };
    let link = LinkState {
        status: "connected".into(),
        paired: true,
        agents: vec![node("canvas-A", "one"), node("canvas-A", "two")],
        ..Default::default()
    };
    let agent = |canvas: Option<&str>, id: Option<&str>| {
        let mut a = agent_for(123, 1, Some("ttys001"));
        a.cwd = Some("/project".into());
        a.route = Route::None;
        a.host = Some(HostApp {
            app: "October".into(),
            pid: 1,
            bundle_path: "/Applications/October.app".into(),
            canvas: canvas.map(String::from),
            node: id.map(String::from),
        });
        a
    };
    // On the Bus: exact, even with two nodes in the same folder.
    let mut agents = vec![agent(Some("canvas-A"), Some("two"))];
    merge(&mut agents, &link);
    assert_eq!(agents[0].route, Route::October { canvas_id: "canvas-A".into(), node_id: "two".into() });
    // A node id that isn't listed routes nowhere.
    let mut agents = vec![agent(Some("canvas-A"), Some("gone"))];
    merge(&mut agents, &link);
    assert_eq!(agents[0].route, Route::None);
    // Off the Bus: two candidates is ambiguous; one is a match.
    let mut agents = vec![agent(None, None)];
    merge(&mut agents, &link);
    assert_eq!(agents[0].route, Route::None);
    let single = LinkState { agents: vec![node("canvas-B", "solo")], ..link.clone() };
    let mut agents = vec![agent(None, None)];
    merge(&mut agents, &single);
    assert_eq!(agents[0].route, Route::October { canvas_id: "canvas-B".into(), node_id: "solo".into() });
}

#[test]
fn harness_pin_is_read_and_checked() {
    use crate::harness::{parse_node_version, parse_pin, pin};
    let shipped = pin().expect("engine/harness.json must have a pin for this system");
    assert_eq!(shipped.package, "@october-dev/october");
    assert!(shipped.node_minimum >= (22, 19, 0));
    assert!(parse_node_version(&shipped.node_version).unwrap() >= shipped.node_minimum, "the pinned Node.js must satisfy the harness");
    let json = include_str!("../harness.json");
    for (key, archive) in [
        ("darwin-arm64", "tar.gz"),
        ("darwin-x64", "tar.gz"),
        ("linux-x64", "tar.xz"),
        ("linux-arm64", "tar.xz"),
        ("win-x64", "zip"),
        ("win-arm64", "zip"),
    ] {
        let p = parse_pin(json, key).unwrap();
        assert!(p.node_url.ends_with(&format!("-{key}.{archive}")), "{}", p.node_url);
        assert_eq!(p.node_sha256.len(), 64);
    }
    // Anything off is no pin at all.
    assert!(parse_pin(&json.replace("https://nodejs.org/dist/", "http://example.com/"), "darwin-arm64").is_none());
    assert!(parse_pin(&json.replace("bed7eea5", "zzzzzzzz"), "darwin-arm64").is_none());
    assert!(parse_pin(json, "riscv").is_none());
    assert_eq!(parse_node_version("v22.19.0\n"), Some((22, 19, 0)));
    assert_eq!(parse_node_version("24.21"), Some((24, 21, 0)));
    assert!(parse_node_version("v22.18.9").unwrap() < (22, 19, 0));
    assert_eq!(parse_node_version("node"), None);
    // Other platforms' optional packages are pruned after npm install.
    use crate::harness::allows;
    use serde_json::json;
    assert!(allows(&json!(null), "darwin"));
    assert!(allows(&json!(["darwin"]), "darwin"));
    assert!(!allows(&json!(["linux"]), "darwin"));
    assert!(!allows(&json!(["!darwin"]), "darwin"));
    assert!(allows(&json!(["!win32"]), "darwin"));
    assert!(!allows(&json!(["x64"]), "arm64"));
}

#[test]
fn harness_targets_name_each_systems_files() {
    use crate::harness::Target;
    use std::path::Path;
    let mac = Target { os: "darwin", arch: "arm64" };
    let linux = Target { os: "linux", arch: "x64" };
    let windows = Target { os: "win", arch: "x64" };
    assert_eq!((mac.key(), mac.archive(), mac.npm_os()), ("darwin-arm64".into(), "tar.gz", "darwin"));
    assert_eq!((linux.key(), linux.archive(), linux.npm_os()), ("linux-x64".into(), "tar.xz", "linux"));
    assert_eq!((windows.key(), windows.archive(), windows.npm_os()), ("win-x64".into(), "zip", "win32"));
    let node = Path::new("/r/node-v24.21.0-linux-x64");
    assert_eq!(linux.node_bin(node), node.join("bin"));
    assert_eq!(linux.node_exe(&linux.node_bin(node)), node.join("bin").join("node"));
    assert_eq!(linux.npm_cli(node), node.join("lib/node_modules/npm/bin/npm-cli.js"));
    assert_eq!(linux.node_dir(&linux.node_bin(node)).as_deref(), Some(node));
    let win = Path::new(r"C:\r\node-v24.21.0-win-x64");
    assert_eq!(windows.node_bin(win), win);
    assert_eq!(windows.node_exe(win), win.join("node.exe"));
    assert_eq!(windows.npm_cli(win), win.join("node_modules").join("npm").join("bin").join("npm-cli.js"));
    assert_eq!(windows.harness_bin(Path::new("h")), Path::new("h").join("node_modules").join(".bin").join("october.cmd"));
    assert_eq!((linux.path_separator(), windows.path_separator()), (':', ';'));
    assert!(crate::harness::pin().is_some(), "this system has a pin");
}

#[test]
fn the_token_command_is_quoted_for_each_shell() {
    use crate::harness::token_command;
    assert_eq!(token_command("/A B/lantern-engine", false), "'/A B/lantern-engine' october-token");
    assert_eq!(token_command("/it's/lantern-engine", false), r"'/it'\''s/lantern-engine' october-token");
    assert_eq!(
        token_command(r"C:\Program Files\Lantern\lantern-engine.exe", true),
        r#""C:\Program Files\Lantern\lantern-engine.exe" october-token"#
    );
}

#[test]
fn bundled_october_runs_with_its_node_and_sign_in() {
    use crate::harness::{Bundled, env_for};
    use crate::launch::{LaunchEnv, agent_command_with, october_env};
    use std::path::Path;
    let bundled = Bundled {
        bin: "/S/October Lantern/harness/1.0/node_modules/.bin/october".into(),
        node_bin: "/S/October Lantern/runtime/node-v24.21.0-darwin-arm64/bin".into(),
        version: "1.0".into(),
    };
    let session = env_for(Some(&("tok.en".to_string(), 2_000)), 1_000);
    let extra = october_env(false, Some(&bundled), session.clone());
    let cmd = agent_command_with(Kind::October, Path::new("/p"), Some("hi"), None, None, None, &extra);
    // The command is single-quoted for `$SHELL -lic`; look inside it.
    let inner = cmd.replace(r"'\''", "'");
    assert!(inner.contains("PATH='/S/October Lantern/runtime/node-v24.21.0-darwin-arm64/bin':\"$PATH\" "), "{inner}");
    assert!(inner.contains("exec '/S/October Lantern/harness/1.0/node_modules/.bin/october' -- 'hi'"), "{inner}");
    for var in [
        "OCTOBER_SUPABASE_URL=",
        "OCTOBER_SUPABASE_ANON_KEY=",
        "OCTOBER_SUPABASE_ACCESS_TOKEN='tok.en'",
        "OCTOBER_SUPABASE_EXPIRES_AT='2000'",
    ] {
        assert!(inner.contains(var), "{var} missing: {inner}");
    }
    assert!(!inner.contains("REFRESH"), "the refresh token is never passed");
    // With October Bus, the Bus wraps the bundled program and the environment still comes first.
    let bus = crate::bus::Attach { wrap: vec!["'/bus'".into(), "agent".into(), "run".into(), "--".into()], ..Default::default() };
    let wrapped = agent_command_with(Kind::October, Path::new("/p"), None, None, None, Some(&bus), &extra).replace(r"'\''", "'");
    assert!(wrapped.contains("OCTOBER_SUPABASE_EXPIRES_AT='2000' exec '/bus' agent run -- '/S/October Lantern/harness/1.0/"), "{wrapped}");
    // `october` on the user's PATH is used as it is, still signed in.
    let own = october_env(true, Some(&bundled), session);
    assert_eq!((own.program.as_deref(), own.path_prefix.as_deref()), (None, None));
    let plain = agent_command_with(Kind::October, Path::new("/p"), None, None, None, None, &own);
    assert!(plain.contains("exec october") && !plain.contains("PATH=") && plain.contains("OCTOBER_SUPABASE_ACCESS_TOKEN"), "{plain}");
    // Signed out, or an expired token: no sign-in at all.
    assert!(env_for(None, 1_000).is_empty());
    assert!(env_for(Some(&("old".to_string(), 999)), 1_000).is_empty());
    assert_eq!(LaunchEnv::default(), october_env(true, None, Vec::new()));
}

#[test]
fn the_bundled_harness_counts_as_installed() {
    use crate::harness::bundled_in;
    use crate::launch::{Installed, with_bundled};
    let shell = Installed { kinds: vec![Kind::Claude], tmux: true, signed_in: vec![] };
    assert!(with_bundled(&shell, true).kinds.contains(&Kind::October));
    assert!(!with_bundled(&shell, false).kinds.contains(&Kind::October));
    let both = Installed { kinds: vec![Kind::October], tmux: false, signed_in: vec![] };
    assert_eq!(with_bundled(&both, true).kinds, vec![Kind::October]);

    // An install is complete only with its program and the Node.js it was installed with.
    let root = temp_dir("harness");
    assert!(bundled_in(&root).is_none());
    let version = crate::harness::pin().unwrap().version;
    let target = crate::harness::Target::this();
    let dir = root.join("harness").join(&version);
    fs::create_dir_all(dir.join("node_modules/.bin")).unwrap();
    fs::write(target.harness_bin(&dir), "#!/usr/bin/env node\n").unwrap();
    assert!(bundled_in(&root).is_none(), "no Node.js recorded yet");
    let node_bin = root.join("node/bin");
    fs::write(dir.join("node-bin"), node_bin.to_string_lossy().as_bytes()).unwrap();
    assert!(bundled_in(&root).is_none(), "its Node.js is gone");
    fs::create_dir_all(&node_bin).unwrap();
    fs::write(target.node_exe(&node_bin), "").unwrap();
    let b = bundled_in(&root).unwrap();
    assert_eq!(b.node_bin, node_bin);
    assert_eq!(b.bin, target.harness_bin(&dir), "the system's own program (october.cmd on Windows)");
    let _ = fs::remove_dir_all(&root);
}

/// Every agent Lantern starts is told where it is and to give full paths.
#[test]
fn launched_agents_are_asked_for_full_paths() {
    let m = crate::launch::with_guidance("find my passport".into(), std::path::Path::new("/Users/me"));
    assert!(m.starts_with("find my passport"));
    assert!(m.contains("You're in /Users/me") && m.contains("full absolute path"));
}

/// The note names the computer the agent is on, never "the Mac" elsewhere, and keeps the prefix
/// the apps hide in the conversation.
#[test]
fn launch_guidance_fits_each_platform() {
    use crate::launch::guidance_for;
    let mac = guidance_for("macos", "hi".into(), std::path::Path::new("/Users/me"));
    assert!(mac.contains("on macOS.") && mac.contains("/Users/name/") && mac.contains("mdfind"));
    let linux = guidance_for("linux", "hi".into(), std::path::Path::new("/home/me"));
    assert!(linux.contains("on Linux.") && linux.contains("/home/name/") && linux.contains("locate"));
    assert!(!linux.contains("Mac") && !linux.contains("mdfind"));
    let windows = guidance_for("windows", "hi".into(), std::path::Path::new(r"C:\Users\me"));
    assert!(windows.contains("on Windows.") && windows.contains(r"C:\Users\name\Documents") && windows.contains("Get-ChildItem"));
    assert!(!windows.contains("Mac"));
    for note in [mac, linux, windows] {
        assert!(note.starts_with("hi\n\n(Started from October Lantern"));
    }
}

#[test]
fn muse_sessions_give_state_question_and_history() {
    use crate::readers::muse;
    let dir = std::env::temp_dir().join(format!("lantern-muse-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("session.jsonl");
    let ev = |seq: u64, kind: &str, event: serde_json::Value| {
        serde_json::json!({"stream": {"kind": "session", "id": "s1"}, "sequence": seq, "recorded_at": 1_791_834_142_000_000u64 + seq,
            "record_type": "event", "payload_type": "runtime.session", "payload": {"kind": kind, "event": event}})
        .to_string()
    };
    let mut log = vec![
        ev(1, "run", serde_json::json!({"kind": "started", "prompt": "check PR 215\nplease"})),
        ev(
            2,
            "run",
            serde_json::json!({"kind": "assistant_tool_calls_committed", "tool_calls": [{"name": "bash", "args": "{\"command\":\"gh pr view 215\",\"description\":\"View PR 215\"}"}]}),
        ),
        ev(
            3,
            "approval",
            serde_json::json!({"kind": "requested", "pending_action_id": "a1", "tool_name": "network", "raw_args": "https api.github.com:443"}),
        ),
        ev(4, "approval", serde_json::json!({"kind": "automated_review_started", "pending_action_id": "a1"})),
    ];
    let write = |log: &[String]| std::fs::write(&path, log.join("\n") + "\n").unwrap();
    write(&log);
    // A model is reviewing the request: still working.
    let s = muse::parse(&path, 0);
    assert_eq!((s.state, s.title.as_deref(), s.session_id.as_deref()), (Some(State::Working), Some("check PR 215"), Some("s1")));
    log.push(ev(
        5,
        "approval",
        serde_json::json!({"kind": "automated_review_completed", "pending_action_id": "a1", "status": "needs_user"}),
    ));
    write(&log);
    let s = muse::parse(&path, 0);
    assert_eq!(s.state, Some(State::NeedsInput));
    assert_eq!(s.question.as_deref(), Some("Allow network: https api.github.com:443?"));
    log.push(ev(6, "approval", serde_json::json!({"kind": "decision_applied", "pending_action_id": "a1", "decision": "approved"})));
    log.push(ev(7, "run", serde_json::json!({"kind": "assistant_message_committed", "text": "PR 215 adds chat mode."})));
    log.push(ev(8, "run", serde_json::json!({"kind": "terminal", "terminal": "completed"})));
    write(&log);
    let s = muse::parse(&path, 0);
    assert_eq!(
        (s.state, s.last_message.as_deref(), s.since),
        (Some(State::Waiting), Some("PR 215 adds chat mode."), Some(1_791_834_142_000))
    );
    let h = muse::history(&path);
    let roles: Vec<_> = h.iter().map(|m| (format!("{:?}", m.role), m.text.as_str())).collect();
    assert_eq!(
        roles,
        [("User".into(), "check PR 215\nplease"), ("Tool".into(), "bash · View PR 215"), ("Agent".into(), "PR 215 adds chat mode.")]
    );
    let root = std::path::Path::new("/h/.local/share/muse/sessions");
    assert!(muse::is_main_lock(root, &root.join("2026/10/12/s1/.session.lock")));
    assert!(!muse::is_main_lock(root, &root.join("2026/10/12/s1/subagent/x/.session.lock")));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Codex 0.16x lists `--no-daemon` among its options; older ones (and prose that mentions it)
/// don't count, so Windows only adds the flag to a Codex that takes it.
#[test]
fn codex_help_is_read_for_the_no_daemon_option() {
    use crate::launch::help_lists_no_daemon;
    let new = "Options:\n      --no-alt-screen\n          Disable alternate screen mode\n\n      --no-daemon\n          Run without the shared background server, even if it is already running\n\n  -h, --help\n";
    assert!(help_lists_no_daemon(new));
    assert!(help_lists_no_daemon(&new.replace('\n', "\r\n")), "Windows line endings");
    assert!(!help_lists_no_daemon("Options:\n      --no-alt-screen\n  -h, --help\n"));
    assert!(!help_lists_no_daemon("Error: rerun the same command with --no-daemon\n"));
    assert!(!help_lists_no_daemon(""));
}

/// When Codex's `--help` can't be read in time, its version decides: 0.160.0 is the first with the
/// daemon (and `--no-daemon`).
#[test]
fn codex_version_decides_the_no_daemon_option_when_help_cannot_be_read() {
    use crate::launch::{codex_version, version_has_no_daemon};
    assert_eq!(codex_version("codex-cli 0.160.0\n"), Some((0, 160, 0)));
    assert_eq!(codex_version("codex-cli 0.161.2\r\n"), Some((0, 161, 2)));
    assert_eq!(codex_version("codex v1.2.3-alpha.4"), Some((1, 2, 3)));
    assert_eq!(codex_version("codex-cli 0.160.0+build7"), Some((0, 160, 0)));
    assert_eq!(codex_version("codex-cli 0.160"), None);
    assert_eq!(codex_version("codex-cli 1.2.3.4"), None);
    assert_eq!(codex_version(""), None);
    assert!(version_has_no_daemon("codex-cli 0.160.0"));
    assert!(version_has_no_daemon("codex-cli 0.172.1"));
    assert!(version_has_no_daemon("codex-cli 1.0.0"));
    assert!(version_has_no_daemon("codex-cli 0.160.0-alpha.2"));
    assert!(!version_has_no_daemon("codex-cli 0.159.9"));
    assert!(!version_has_no_daemon("codex-cli 0.98.0"));
    assert!(!version_has_no_daemon("not a version"));
}
