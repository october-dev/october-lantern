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
        start_ms: 0,
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

    let chat = history::claude(&path).unwrap().messages;
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
    let chat = pi::history(&path).unwrap().messages;
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

#[test]
fn grok_records_and_states() {
    use crate::history::Role;
    use crate::readers::grok;
    let dir = temp_dir("grok");
    let path = dir.join("updates.jsonl");
    // Redacted copies of real `updates.jsonl` lines: the `{timestamp, method, params}` envelope,
    // `turn_completed` on `_x.ai/session/update` with `prompt_id` and `stop_reason`.
    let ask = r#"{"timestamp":1787811120,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"rename the file"},"_meta":{"modelId":"grok-4.5","promptIndex":0}},"_meta":{"eventId":"g1-1","agentTimestampMs":1787811120000}}}"#;
    let thought = r#"{"timestamp":1787811130,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"planning the rename"}},"_meta":{"eventId":"g1-2","agentTimestampMs":1787811130000}}}"#;
    let narrate = r#"{"timestamp":1787811135,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Let me look."}},"_meta":{"eventId":"g1-3","agentTimestampMs":1787811135000}}}"#;
    let call = r#"{"timestamp":1787811140,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"tool_call","toolCallId":"call-1","title":"run_terminal_command","rawInput":{"command":"mv a b","description":"rename the file"}},"_meta":{"eventId":"g1-4","agentTimestampMs":1787811140000}}}"#;
    let call_update = r#"{"timestamp":1787811150,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"tool_call_update","toolCallId":"call-1","status":"completed"},"_meta":{"eventId":"g1-5","agentTimestampMs":1787811150000}}}"#;
    let chunk1 = r#"{"timestamp":1787811160,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Renamed "}},"_meta":{"eventId":"g1-6","agentTimestampMs":1787811160000}}}"#;
    let chunk2 = r#"{"timestamp":1787811170,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"the file."}},"_meta":{"eventId":"g1-7","agentTimestampMs":1787811170000}}}"#;
    // Trailing bookkeeping after the final text: real turns run hooks before `turn_completed`.
    let hook = r#"{"timestamp":1787811175,"method":"_x.ai/session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"hook_execution","event_name":"session_start","runs":[{"name":"global/settings:session_start[0].hooks[0]","status":{"status":"success","elapsed_ms":4}}]},"_meta":{"eventId":"g1-8","agentTimestampMs":1787811175000}}}"#;
    let done = r#"{"timestamp":1787811180,"method":"_x.ai/session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"turn_completed","prompt_id":"p1","stop_reason":"end_turn","elapsed_ms":1000},"_meta":{"eventId":"g1-9","agentTimestampMs":1787811180000}}}"#;
    let failed = r#"{"timestamp":1787811190,"method":"_x.ai/session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"turn_completed","prompt_id":"p2","stop_reason":"error","agent_result":"API error (status 402): usage balance exhausted","elapsed_ms":621},"_meta":{"eventId":"g1-9","agentTimestampMs":1787811190000}}}"#;
    // A background-task completion: a `user_message_chunk` for the model only, which still
    // starts a turn (real sessions show the model's thoughts following it).
    let hidden = r#"{"timestamp":1787811200,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"<system-reminder>Background task \"watch\" completed (exit code: 0).</system-reminder>"},"_meta":{"modelId":"grok-4.5","promptIndex":8,"hideFromScrollback":true}},"_meta":{"eventId":"g1-10","agentTimestampMs":1787811200000}}}"#;
    // A second prompt, stopped before Grok says anything: a completion with no text and no result.
    let ask_again = r#"{"timestamp":1787811210,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"try again"},"_meta":{"modelId":"grok-4.5","promptIndex":1}},"_meta":{"eventId":"g1-11","agentTimestampMs":1787811210000}}}"#;
    let cancelled = r#"{"timestamp":1787811220,"method":"_x.ai/session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"turn_completed","prompt_id":"p3","stop_reason":"cancelled","elapsed_ms":300},"_meta":{"eventId":"g1-12","agentTimestampMs":1787811220000}}}"#;
    // The reader skips a line that isn't JSON, which would hide a broken fixture.
    for line in [ask, thought, narrate, call, call_update, chunk1, chunk2, hook, done, failed, hidden, ask_again, cancelled] {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
    let hook_json: serde_json::Value = serde_json::from_str(hook).unwrap();
    assert_eq!(hook_json["params"]["update"]["sessionUpdate"], "hook_execution");

    // A reminder alone starts a turn: the model answers next.
    fs::write(&path, format!("{hidden}\n")).unwrap();
    assert_eq!(grok::parse(&path, 0).state, Some(State::Working));
    fs::write(&path, format!("{ask}\n{thought}\n{call}\n{call_update}\n")).unwrap();
    assert_eq!(grok::parse(&path, 0).state, Some(State::Working));
    // Text → tool call → text → hook → done: only the last text is the message.
    fs::write(&path, format!("{ask}\n{thought}\n{narrate}\n{call}\n{call_update}\n{chunk1}\n{chunk2}\n{hook}\n{done}\n")).unwrap();
    let s = grok::parse(&path, 0);
    assert_eq!(s.state, Some(State::Waiting));
    assert_eq!(s.last_message.as_deref(), Some("Renamed the file."));
    assert_eq!(s.title.as_deref(), Some("rename the file"));
    assert_eq!(s.session_id.as_deref(), Some("g1"));
    assert_eq!(s.since, Some(1_787_811_180_000));
    // An error turn shows the error Grok recorded.
    fs::write(&path, format!("{ask}\n{thought}\n{failed}\n")).unwrap();
    let s = grok::parse(&path, 0);
    assert_eq!(s.state, Some(State::Waiting));
    assert_eq!(s.last_message.as_deref(), Some("API error (status 402): usage balance exhausted"));
    // The chat shows the error too, after the prompt, as its own entry.
    let chat = grok::history(&path).expect("the fixture reads").messages;
    assert_eq!(chat.len(), 2, "the prompt and the error: {chat:?}");
    assert!(matches!(chat[0].role, Role::User));
    assert!(matches!(chat[1].role, Role::Agent));
    assert_eq!(chat[1].text, "API error (status 402): usage balance exhausted");
    assert_eq!(chat[1].at, Some(crate::model::iso(1_787_811_190_000)));
    assert_eq!(chat[1].id, Some(format!("{}.result", ask.len() + thought.len() + 2)));
    // An error turn's own text never becomes a later turn's message: the next turn ends with
    // nothing to say, so there is no last message.
    fs::write(&path, format!("{ask}\n{narrate}\n{failed}\n{ask_again}\n{cancelled}\n")).unwrap();
    let s = grok::parse(&path, 0);
    assert_eq!(s.state, Some(State::Waiting));
    assert_eq!(s.last_message, None, "not the earlier turn's narration");
    // A reminder after the turn starts the next one; the title stays the user's words.
    fs::write(&path, format!("{ask}\n{done}\n{hidden}\n")).unwrap();
    let s = grok::parse(&path, 0);
    assert_eq!(s.state, Some(State::Working));
    assert_eq!(s.title.as_deref(), Some("rename the file"));
}

#[test]
fn grok_history_joins_chunks_and_skips_hidden() {
    use crate::history::Role;
    use crate::readers::grok;
    let dir = temp_dir("grok-history");
    let path = dir.join("updates.jsonl");
    let ask = r#"{"timestamp":1787811120,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"rename the file"},"_meta":{"modelId":"grok-4.5","promptIndex":0}},"_meta":{"eventId":"g1-1","agentTimestampMs":1787811120000}}}"#;
    let thought = r#"{"timestamp":1787811130,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"planning"}},"_meta":{"eventId":"g1-2","agentTimestampMs":1787811130000}}}"#;
    let call = r#"{"timestamp":1787811140,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"tool_call","toolCallId":"call-1","title":"run_terminal_command","rawInput":{"command":"mv a b","description":"rename the file"}},"_meta":{"eventId":"g1-3","agentTimestampMs":1787811140000}}}"#;
    let read = r#"{"timestamp":1787811145,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"tool_call","toolCallId":"call-2","title":"read_file","rawInput":{"target_file":"/tmp/notes.md"}},"_meta":{"eventId":"g1-4","agentTimestampMs":1787811145000}}}"#;
    let call_update = r#"{"timestamp":1787811150,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"tool_call_update","toolCallId":"call-1","status":"completed"},"_meta":{"eventId":"g1-5","agentTimestampMs":1787811150000}}}"#;
    let chunk1 = r#"{"timestamp":1787811160,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Renamed "}},"_meta":{"eventId":"g1-6","agentTimestampMs":1787811160000}}}"#;
    let chunk2 = r#"{"timestamp":1787811170,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"the file."}},"_meta":{"eventId":"g1-7","agentTimestampMs":1787811170000}}}"#;
    let hidden = r#"{"timestamp":1787811190,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"<system-reminder>done</system-reminder>"},"_meta":{"hideFromScrollback":true}},"_meta":{"eventId":"g1-8","agentTimestampMs":1787811190000}}}"#;
    for line in [ask, thought, call, read, call_update, chunk1, chunk2, hidden] {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
    fs::write(&path, format!("{ask}\n{thought}\n{call}\n{read}\n{call_update}\n{chunk1}\n{chunk2}\n{hidden}\n")).unwrap();
    let chat = grok::history(&path).expect("the fixture reads");
    assert!(chat.whole);
    assert_eq!(chat.messages.len(), 4, "prompt, two tool calls and one joined reply: {:?}", chat.messages);
    assert!(matches!(chat.messages[0].role, Role::User));
    assert_eq!(chat.messages[0].text, "rename the file");
    assert_eq!(chat.messages[0].id.as_deref(), Some("0.0"));
    assert_eq!(chat.messages[0].at, Some(crate::model::iso(1_787_811_120_000)));
    assert!(matches!(chat.messages[1].role, Role::Tool));
    assert!(chat.messages[1].text.contains("run_terminal_command"), "the tool call is described: {}", chat.messages[1].text);
    assert!(chat.messages[1].text.contains("rename the file"), "with its description: {}", chat.messages[1].text);
    assert!(matches!(chat.messages[2].role, Role::Tool));
    assert!(chat.messages[2].text.contains("read_file"), "the read is described: {}", chat.messages[2].text);
    assert!(chat.messages[2].text.contains("notes.md"), "with its file: {}", chat.messages[2].text);
    assert!(matches!(chat.messages[3].role, Role::Agent));
    assert_eq!(chat.messages[3].text, "Renamed the file.");
}

#[test]
fn grok_session_file_skips_hidden_and_matches_cwd() {
    use crate::readers::grok;
    use std::time::{Duration, SystemTime};
    assert_eq!(grok::encode("/a b"), "%2Fa%20b");
    let dir = temp_dir("grok-sessions");
    let work = dir.join("work");
    fs::create_dir_all(&work).unwrap();
    let cwd = work.canonicalize().unwrap().to_string_lossy().into_owned();
    let sessions = dir.join("sessions");
    // The search index at the root is not a session directory.
    fs::create_dir_all(&sessions).unwrap();
    fs::write(sessions.join("session_search.sqlite"), b"not a directory").unwrap();
    let bucket = sessions.join(grok::encode(&cwd));
    let line = r#"{"timestamp":1787811120,"method":"session/update","params":{"sessionId":"g1","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"hi"}}}}"#;
    serde_json::from_str::<serde_json::Value>(line).unwrap();
    // Serialized, not interpolated: a Windows cwd's backslashes are JSON escapes.
    let summary = |id: &str, cwd: &str| serde_json::json!({"info": {"id": id, "cwd": cwd}}).to_string();
    let child_summary = |id: &str, cwd: &str| serde_json::json!({"info": {"id": id, "cwd": cwd}, "session_kind": "subagent"}).to_string();
    let hidden_summary = |id: &str, cwd: &str| serde_json::json!({"info": {"id": id, "cwd": cwd}, "hidden": true}).to_string();
    let other_summary = serde_json::json!({"info": {"id": "44444444-4444-4444-4444-444444444444", "cwd": "/elsewhere"}}).to_string();
    let make = |id: &str, summary: &str| {
        let session = bucket.join(id);
        fs::create_dir_all(&session).unwrap();
        let log = session.join("updates.jsonl");
        fs::write(&log, format!("{line}\n")).unwrap();
        fs::write(session.join("summary.json"), summary).unwrap();
        log
    };
    let live = make("11111111-1111-1111-1111-111111111111", &summary("11111111-1111-1111-1111-111111111111", &cwd));
    // A newer subagent child in the same bucket is never picked...
    let child = make("22222222-2222-2222-2222-222222222222", &child_summary("22222222-2222-2222-2222-222222222222", &cwd));
    // ...nor an explicitly hidden session, however new...
    let hid = make("33333333-3333-3333-3333-333333333333", &hidden_summary("33333333-3333-3333-3333-333333333333", &cwd));
    // ...nor a session whose summary names another cwd, however new.
    let other = make("44444444-4444-4444-4444-444444444444", &other_summary);
    let stamp = |path: &PathBuf, secs_ago: u64| {
        let t = SystemTime::now() - Duration::from_secs(secs_ago);
        fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
    };
    stamp(&live, 100);
    stamp(&child, 10);
    stamp(&hid, 5);
    stamp(&other, 1);
    let found = grok::session_file_in(&sessions, &work, 0);
    assert_eq!(found.as_deref(), Some(live.as_path()), "only the live session on this cwd matches");
    // The encoded folder gone (a cwd Grok saw before a symlink changed): a session elsewhere
    // still matches by its own cwd.
    fs::remove_dir_all(&bucket).unwrap();
    let stray_dir = sessions.join("stale-bucket").join("55555555-5555-5555-5555-555555555555");
    fs::create_dir_all(&stray_dir).unwrap();
    let stray = stray_dir.join("updates.jsonl");
    fs::write(&stray, format!("{line}\n")).unwrap();
    fs::write(stray_dir.join("summary.json"), summary("55555555-5555-5555-5555-555555555555", &cwd)).unwrap();
    let fallback = grok::session_file_in(&sessions, &work, 0);
    assert_eq!(fallback.as_deref(), Some(stray.as_path()), "the fallback scan matches by the summary's cwd");
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
        choices: None,
        transcript: None,
        bus_id: None,
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

/// Claude's prompt for a risky `rm` (2026-11-06): its PermissionRequest event wasn't the latest
/// when the reminder came, so Lantern said "Needs your answer" with no card. The reminder now
/// gives a card that answers it, with the command from the transcript.
#[test]
fn a_permission_reminder_without_its_request_still_gets_a_card() {
    let dir = std::env::temp_dir().join(format!("lantern-perm-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = dir.join("t.jsonl");
    let command = "cd ~; for d in BraveSoftware ms-playwright; do rm -rf \"$C/$d\"/* \"$C/$d\"/.[!.]* 2>/dev/null; done";
    let lines = [
        serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"a","name":"AskUserQuestion","input":{}}]}}),
        serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"a","content":"Clear all the safe caches"}]}}),
        serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"Clearing those now."}]}}),
        serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"b","name":"Bash","input":{"command":command}}]}}),
    ];
    std::fs::write(&transcript, lines.iter().map(|l| l.to_string() + "\n").collect::<String>()).unwrap();
    let v = serde_json::json!({
        "hook_event_name": "Notification", "notification_type": "permission_prompt",
        "session_id": format!("s-{}", uuid::Uuid::new_v4()), "message": "Claude needs your permission to use Bash",
        "transcript_path": transcript.to_string_lossy(),
    });
    let ev = hooks::claude_event(&v).unwrap();
    assert_eq!((ev.state, ev.question_kind), (State::NeedsInput, Some(QuestionKind::Permission)));
    assert!(ev.question.as_deref().unwrap().starts_with("Permission to run Bash · cd ~;"), "{:?}", ev.question);
    assert!(ev.question_detail.unwrap().contains("rm -rf"));
    assert!(ev.prompt_id.is_some());

    // Without a readable transcript, Claude's own words, still answerable.
    let bare = serde_json::json!({
        "hook_event_name": "Notification", "notification_type": "permission_prompt",
        "session_id": format!("s-{}", uuid::Uuid::new_v4()), "message": "Claude needs your permission to use Bash",
    });
    let ev = hooks::claude_event(&bare).unwrap();
    assert_eq!(ev.question.as_deref(), Some("Claude needs your permission to use Bash"));
    assert!(ev.prompt_id.is_some());
    let _ = std::fs::remove_dir_all(&dir);
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
        choices: None,
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
        choices: None,
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

/// Images added to a task are named in the first message; Codex gets each attached, Claude Code
/// their folder once.
#[test]
fn new_sessions_carry_added_images() {
    use crate::launch::{Extras, agent_command_with, first_message};
    use std::path::{Path, PathBuf};
    let dir = "/Users/me/Library/Application Support/October Lantern/screenshots";
    let images = [PathBuf::from(format!("{dir}/a.png")), PathBuf::from(format!("{dir}/b.png"))];
    let msg = first_message(Some("make it match"), Extras { images: &images, ..Default::default() }).unwrap();
    assert!(msg.starts_with("make it match") && msg.contains("- ") && msg.contains("a.png") && msg.contains("b.png"), "{msg}");
    let one = first_message(None, Extras { images: &images[..1], ..Default::default() }).unwrap();
    assert!(one.contains("wait for my instructions") && one.contains("a.png") && !one.contains("b.png"), "{one}");
    let files: Vec<&Path> = images.iter().map(PathBuf::as_path).collect();
    let env = crate::launch::LaunchEnv::default();
    let codex = agent_command_with(Kind::Codex, Path::new("/p"), Some(&msg), &files, None, None, &env);
    assert_eq!(codex.split_once(" -- ").unwrap().0.matches("--image").count(), 2, "{codex}");
    let claude = agent_command_with(Kind::Claude, Path::new("/p"), Some(&msg), &files, None, None, &env);
    assert_eq!(claude.split_once(" -- ").unwrap().0.matches("--add-dir").count(), 1, "{claude}");
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
    assert!(cmd.contains("october-bus' agent run -- october"), "{cmd}");
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
    let cmd = agent_command_with(Kind::October, Path::new("/p"), Some("hi"), &[], None, None, &extra);
    // The private launch script contains the environment, never the terminal command arguments.
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
    let wrapped = agent_command_with(Kind::October, Path::new("/p"), None, &[], None, Some(&bus), &extra).replace(r"'\''", "'");
    assert!(wrapped.contains("OCTOBER_SUPABASE_EXPIRES_AT='2000' exec '/bus' agent run -- '/S/October Lantern/harness/1.0/"), "{wrapped}");
    // `october` on the user's PATH is used as it is, still signed in.
    let own = october_env(true, Some(&bundled), session);
    assert_eq!((own.program.as_deref(), own.path_prefix.as_deref()), (None, None));
    let plain = agent_command_with(Kind::October, Path::new("/p"), None, &[], None, None, &own);
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
    let shell = Installed { kinds: vec![Kind::Claude], tmux: true, signed_in: vec![], never_set_up: vec![] };
    assert!(with_bundled(&shell, true).kinds.contains(&Kind::October));
    assert!(!with_bundled(&shell, false).kinds.contains(&Kind::October));
    let both = Installed { kinds: vec![Kind::October], tmux: false, signed_in: vec![], never_set_up: vec![] };
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
    // Administrator steps go through the system's own prompt, for the person to allow.
    assert!(windows.contains("-Verb RunAs") && mac.contains("with administrator privileges") && linux.contains("pkexec"));
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
    let h = muse::history(&path).unwrap().messages;
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

fn color_and_toppings() -> serde_json::Value {
    serde_json::json!({"questions": [
        {"question": "Which color?", "header": "Color", "multiSelect": false,
         "options": [{"label": "Red", "description": "The color red"}, {"label": "Green"}, {"label": "Blue"}]},
        {"question": "Which toppings?", "header": "Toppings", "multiSelect": true,
         "options": [{"label": "Cheese"}, {"label": "Olives"}, {"label": "Onion"}]}
    ]})
}

#[test]
fn claudes_own_questions_are_choices_not_permissions() {
    // AskUserQuestion comes through the permission hook; Allow (1) would quietly pick the first option.
    let v = serde_json::json!({"hook_event_name": "PermissionRequest", "session_id": "s", "tool_name": "AskUserQuestion",
        "tool_input": color_and_toppings()});
    let ev = hooks::claude_event(&v).unwrap();
    assert_eq!(ev.question_kind, Some(QuestionKind::Choice));
    assert_eq!(ev.question.as_deref(), Some("Which color? (+1 more)"));
    assert!(ev.prompt_id.is_some());
    let questions = ev.choices.unwrap().questions;
    assert_eq!((questions.len(), questions[1].multi_select, questions[0].options[0].label.as_str()), (2, true, "Red"));

    let plan = serde_json::json!({"hook_event_name": "PermissionRequest", "session_id": "s", "tool_name": "ExitPlanMode",
        "tool_input": {"plan": "1. Make hello.txt"}});
    let ev = hooks::claude_event(&plan).unwrap();
    assert_eq!((ev.question_kind, ev.question_detail.as_deref()), (Some(QuestionKind::Plan), Some("1. Make hello.txt")));

    // A permission offers "always" only when Claude's terminal shows it (it has suggestions).
    let bash = serde_json::json!({"hook_event_name": "PermissionRequest", "session_id": "s", "tool_name": "Bash",
        "tool_input": {"command": "npm install stripe"},
        "permission_suggestions": [{"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "npm install:*"}]}]});
    assert_eq!(hooks::claude_event(&bash).unwrap().choices.unwrap().always.as_deref(), Some("Always allow npm install:*"));
    let bare = serde_json::json!({"hook_event_name": "PermissionRequest", "session_id": "s", "tool_name": "Bash",
        "tool_input": {"command": "ls"}});
    assert_eq!(hooks::claude_event(&bare).unwrap().choices, None);
}

#[test]
fn an_open_question_in_the_transcript_needs_you_without_hooks() {
    let dir = temp_dir("claude-question");
    let path = dir.join("s.jsonl");
    let user = r#"{"type":"user","timestamp":"2026-09-23T16:51:00.000Z","message":{"role":"user","content":"pick"}}"#;
    let ask = serde_json::json!({"type": "assistant", "timestamp": "2026-09-23T16:51:05.000Z", "message": {"id": "m1",
        "stop_reason": "tool_use", "content": [{"type": "tool_use", "id": "toolu_9", "name": "AskUserQuestion", "input": color_and_toppings()}]}});
    fs::write(&path, format!("{user}\n{ask}\n")).unwrap();
    let s = transcripts::parse_claude(&path, 0);
    assert_eq!(
        (s.state, s.question_kind, s.prompt_id.as_deref()),
        (Some(State::NeedsInput), Some(QuestionKind::Choice), Some("t-toolu_9"))
    );
    assert_eq!(s.choices.unwrap().questions.len(), 2);
    // Claude's status file says busy while it waits: the question stays.
    let mut status = transcripts::parse_claude(&path, 0);
    let busy = crate::apps::ClaudeLive { pid: 1, session_id: None, entrypoint: None, status: Some("busy".into()), status_updated_at: None };
    scanner::apply_live(&mut status, StateSource::Transcript, &busy);
    assert_eq!(status.state, Some(State::NeedsInput));
    // Answered: back to work.
    let answer = r#"{"type":"user","timestamp":"2026-09-23T16:51:09.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_9","content":"Red"}]}}"#;
    fs::write(&path, format!("{user}\n{ask}\n{answer}\n")).unwrap();
    assert_eq!(transcripts::parse_claude(&path, 0).state, Some(State::Working));
}

#[test]
fn answers_become_the_terminals_keys() {
    use crate::deliver::Key::{self, Char, Escape, Tab};
    use crate::serve::{Answer, answer_keys};
    let mut a = agent_for(1, 1, Some("ttys004"));
    let keys = |a: &Agent, answer: Answer| answer_keys(a, &answer).ok();
    let picks = |p: &[&[usize]]| Answer::Picks { picks: p.iter().map(|q| q.to_vec()).collect() };

    a.question_kind = Some(QuestionKind::Permission);
    assert_eq!(keys(&a, Answer::Yes), Some(vec![Char('1')]));
    assert_eq!(keys(&a, Answer::No), Some(vec![Escape]));
    assert_eq!(keys(&a, Answer::Always), None, "only offered when the terminal shows it");
    a.choices = Some(crate::model::Choices { questions: Vec::new(), always: Some("Always allow this folder".into()) });
    assert_eq!(keys(&a, Answer::Always), Some(vec![Char('2')]));

    a.question_kind = Some(QuestionKind::Plan);
    a.choices = None;
    assert_eq!(keys(&a, Answer::Yes), Some(vec![Char('1')]));
    assert_eq!(keys(&a, Answer::No), Some(vec![Escape]));

    // As tested against Claude Code 2.1.310: one single-choice question is answered by its number;
    // otherwise numbers (Tab after a pick-many question), then 1 submits.
    a.question_kind = Some(QuestionKind::Choice);
    let questions = hooks::choice_questions(&color_and_toppings()).unwrap();
    a.choices = Some(crate::model::Choices { questions: questions[..1].to_vec(), always: None });
    assert_eq!(keys(&a, picks(&[&[1]])), Some(vec![Char('2')]));
    assert_eq!(keys(&a, picks(&[&[0, 1]])), None, "one pick for a single-choice question");
    assert_eq!(keys(&a, picks(&[&[3]])), None);
    assert_eq!(keys(&a, Answer::Yes), None);
    a.choices = Some(crate::model::Choices { questions: questions.clone(), always: None });
    let both: Vec<Key> = vec![Char('3'), Char('1'), Char('3'), Tab, Char('1')];
    assert_eq!(keys(&a, picks(&[&[2], &[2, 0, 2]])), Some(both));
    assert_eq!(keys(&a, picks(&[&[2]])), None, "every question needs an answer");
    a.choices = Some(crate::model::Choices { questions: questions[1..].to_vec(), always: None });
    assert_eq!(keys(&a, picks(&[&[1]])), Some(vec![Char('2'), Tab, Char('1')]));

    // As tested against Claude Code 2.1.317: a question showing a draft takes the number as a
    // move, so Enter picks; a plain question after it still picks by number, and 1 submits.
    let mut drafted = questions[..1].to_vec();
    for option in &mut drafted[0].options {
        option.preview = Some("0–3s  hook".into());
    }
    a.choices = Some(crate::model::Choices { questions: [drafted.clone(), drafted].concat(), always: None });
    assert_eq!(keys(&a, picks(&[&[1], &[0]])), Some(vec![Char('2'), Key::Enter, Char('1'), Key::Enter, Char('1')]));

    // Not a question Lantern answers: nothing is pressed.
    a.question_kind = Some(QuestionKind::Other);
    assert_eq!(keys(&a, Answer::No), None);
    // A question needs its prompt id, like a permission.
    a.question_kind = Some(QuestionKind::Choice);
    a.prompt_id = Some("t-1".into());
    assert!(crate::serve::keys_for(&a, vec![Char('1')], None).is_err());
    assert!(crate::serve::keys_for(&a, vec![Char('1')], Some("t-1".into())).is_ok());
}

/// By hand, against a running Bus: `LANTERN_BUS_E2E_TO=<agent's Bus id>
/// LANTERN_ENGINE_BIN=target/debug/lantern-engine cargo test -- --ignored
/// a_message_reaches_an_agent_through_a_real_bus` (with the Bus's environment).
#[test]
#[ignore]
fn a_message_reaches_an_agent_through_a_real_bus() {
    let to = std::env::var("LANTERN_BUS_E2E_TO").expect("LANTERN_BUS_E2E_TO");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let text = "Create a file called from-lantern.txt containing the word hello.";
    assert!(matches!(crate::bus::send_as_you(&to, text, "e2e-1", deadline), crate::bus::Posted::Accepted));
    // The same send again is the same message: the Bus keeps one.
    assert!(matches!(crate::bus::send_as_you(&to, text, "e2e-1", deadline), crate::bus::Posted::Accepted));
}

#[test]
fn a_send_that_waited_too_long_posts_nothing() {
    let past = std::time::Instant::now();
    assert!(matches!(crate::bus::send_as_you("lantern-x", "hi", "k", past), crate::bus::Posted::Expired));
}

#[test]
fn an_agents_bus_id_comes_from_its_bus_connection() {
    let at = |pid: u32, ppid: u32, cmd: &[&str]| Proc { pid, ppid: Some(ppid), ..proc(cmd) };
    let bus = "/Users/me/Library/Application Support/October Lantern/bin/october-bus-20b7456";
    let table = crate::procs::ProcTable {
        procs: [
            at(10, 1, &["claude"]),
            at(11, 10, &[bus, "mcp", "stdio", "--scope", "lantern", "--agent", "lantern-claude-1a2b3c4d", "--name", "Claude Code · x"]),
            // Codex's node wrapper: the Bus connection is under its native binary.
            at(20, 1, &["node", "/usr/local/bin/codex"]),
            at(21, 20, &["codex"]),
            at(22, 21, &[bus, "mcp", "stdio", "--agent", "lantern-codex-99aa00bb"]),
            // The October harness runs under `october-bus agent run`.
            at(30, 1, &[bus, "agent", "run", "--scope", "lantern", "--id", "lantern-october-0011aa22", "--", "october"]),
            at(31, 30, &["october"]),
            // Someone else's MCP server with an --agent flag isn't the Bus.
            at(40, 1, &["claude"]),
            at(41, 40, &["other-mcp", "--agent", "lantern-not-ours"]),
        ]
        .into_iter()
        .map(|p| (p.pid, p))
        .collect(),
    };
    assert_eq!(table.bus_id(10).as_deref(), Some("lantern-claude-1a2b3c4d"));
    assert_eq!(table.bus_id(20).as_deref(), Some("lantern-codex-99aa00bb"));
    assert_eq!(table.bus_id(31).as_deref(), Some("lantern-october-0011aa22"));
    assert_eq!(table.bus_id(40), None);
}

#[test]
fn a_question_keeps_the_drafts_its_options_preview() {
    let input = serde_json::json!({"questions": [{"question": "OK to build?", "options": [
        {"label": "Looks good", "preview": "0–1.5s  HOOK\n1.5–4s  chaos\n"},
        {"label": "Change something"},
    ]}]});
    let questions = hooks::choice_questions(&input).unwrap();
    assert_eq!(questions[0].options[0].preview.as_deref(), Some("0–1.5s  HOOK\n1.5–4s  chaos"));
    assert_eq!(questions[0].options[1].preview, None);
    let sent = serde_json::to_value(&questions[0].options).unwrap();
    assert_eq!(sent[1], serde_json::json!({"label": "Change something"}));
}

#[test]
fn october_runs_on_pro_for_a_paid_lantern_and_lite_otherwise() {
    crate::harness::set_pro(false);
    assert_eq!(crate::harness::default_model(), "october/claude/lite");
    crate::harness::set_pro(true);
    assert_eq!(crate::harness::default_model(), "october/claude/pro");
    crate::harness::set_pro(false);
    assert!(crate::models::valid(crate::harness::PRO_MODEL) && crate::models::valid(crate::harness::LITE_MODEL));
    let plan: serde_json::Value = serde_json::from_str(r#"{"type":"october.plan","pro":true}"#).unwrap();
    assert_eq!(plan["pro"], true);
}

/// History entries keep the same id on every read: identical-looking steps stay distinct,
/// appending doesn't renumber earlier ones, and a window cut from the end of the file names its
/// lines as a whole read does.
#[test]
fn history_ids_name_entries_for_good() {
    let dir = temp_dir("history-ids");
    let path = dir.join("s.jsonl");
    let step = r#"{"type":"assistant","timestamp":"2026-09-23T16:51:05.000Z","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#;
    let answer = r#"{"type":"assistant","timestamp":"2026-09-23T16:51:05.000Z","message":{"content":[{"type":"text","text":"Done."},{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#;
    fs::write(&path, format!("{step}\n{step}\n{answer}\n")).unwrap();
    let first = history::claude(&path).unwrap().messages;
    let ids: Vec<String> = first.iter().map(|m| m.id.clone().unwrap()).collect();
    assert_eq!(first.len(), 4);
    assert_eq!(first[0].text, first[1].text);
    let unique: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), 4, "identical steps are still distinct entries: {ids:?}");

    fs::write(&path, format!("{step}\n{step}\n{answer}\n{step}\n")).unwrap();
    let more = history::claude(&path).unwrap().messages;
    assert_eq!(more[..4].iter().map(|m| m.id.clone().unwrap()).collect::<Vec<_>>(), ids, "appending keeps earlier ids");

    let (whole, from_start) = transcripts::tail_lines(&path, u64::MAX).unwrap();
    assert!(from_start);
    let len = fs::metadata(&path).unwrap().len();
    let (tail, cut) = transcripts::tail_lines(&path, len - 10).unwrap();
    assert!(!cut, "a read cut from the end doesn't start at the beginning");
    assert_eq!(tail.len(), whole.len() - 1, "the cut first line is dropped");
    assert_eq!(tail.iter().map(|(o, _)| *o).collect::<Vec<_>>(), whole[1..].iter().map(|(o, _)| *o).collect::<Vec<_>>());
}

/// The source names the session read, the same each time and different for another one.
#[test]
fn history_sources_are_stable_names() {
    assert_eq!(history::source_id("/a/s.jsonl"), history::source_id("/a/s.jsonl"));
    assert_ne!(history::source_id("/a/s.jsonl"), history::source_id("/a/t.jsonl"));
    assert_eq!(history::source_id("").len(), 16);
}

/// A Gemini record written again is the same entry updated: its id doesn't change.
#[test]
fn gemini_history_keeps_a_records_id_when_it_is_rewritten() {
    use crate::readers::gemini;
    let dir = temp_dir("gemini-ids");
    let path = dir.join("session.jsonl");
    let header = r#"{"sessionId":"g1","startTime":"2026-01-01T00:00:00Z","kind":"main"}"#;
    let pending = r#"{"id":"g1","type":"gemini","content":"","toolCalls":[{"name":"run_shell_command","args":{}}]}"#;
    let filled = r#"{"id":"g1","type":"gemini","content":"","toolCalls":[{"name":"run_shell_command","args":{"command":"pwd"}}]}"#;
    fs::write(&path, format!("{header}\n{pending}\n")).unwrap();
    let before = gemini::history(&path).unwrap().messages;
    fs::write(&path, format!("{header}\n{pending}\n{filled}\n")).unwrap();
    let after = gemini::history(&path).unwrap().messages;
    assert_eq!(after.len(), 1);
    assert_eq!(before[0].id, after[0].id);
    assert_ne!(before[0].text, after[0].text);
}

/// OpenCode's coverage comes from the session's own message count: a long session whose newest
/// message has no parts yet isn't whole. A database that can't be read is no read, not an empty
/// conversation; a short session is whole, and its entries carry their message's time.
#[test]
fn opencode_history_coverage_and_failure() {
    use crate::readers::opencode;
    let dir = temp_dir("opencode-coverage");
    let db = dir.join("opencode.db");
    let cwd = dir.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE TABLE session(id TEXT, title TEXT, directory TEXT, parent_id TEXT, time_created INTEGER);
         CREATE TABLE message(id TEXT, session_id TEXT, time_updated INTEGER, data TEXT);
         CREATE TABLE part(id TEXT, message_id TEXT, data TEXT);",
    )
    .unwrap();
    let dir_s = cwd.to_string_lossy().into_owned();
    c.execute("INSERT INTO session VALUES('ses_1','t',?1,NULL,1000)", [&dir_s]).unwrap();
    let message = |n: usize, parts: bool| {
        let id = format!("msg_{n:04}");
        let role = if n.is_multiple_of(2) { "user" } else { "assistant" };
        let data = format!("{{\"role\":\"{role}\",\"time\":{{\"created\":{}}}}}", 1_790_000_000_000u64 + n as u64);
        c.execute("INSERT INTO message VALUES(?1,'ses_1',?2,?3)", rusqlite::params![id, 1_790_000_000_000i64 + n as i64, data]).unwrap();
        if parts {
            let part = format!("{{\"type\":\"text\",\"text\":\"m{n}\"}}");
            c.execute("INSERT INTO part VALUES(?1,?2,?3)", rusqlite::params![format!("prt_{n:04}"), id, part]).unwrap();
        }
    };
    for n in 0..10 {
        message(n, true);
    }
    let short = opencode::history_in(&db, &cwd, 0).unwrap();
    assert!(short.complete, "ten messages: the whole session");
    assert_eq!(short.messages.len(), 10);
    assert_eq!(short.messages[0].id.as_deref(), Some("prt_0000"));
    assert!(short.messages[0].at.is_some(), "entries carry their message's time");

    for n in 10..100 {
        message(n, n != 99);
    }
    let long = opencode::history_in(&db, &cwd, 0).unwrap();
    assert!(!long.complete, "100 messages, the newest without parts yet: not the whole session");

    drop(c);
    fs::write(&db, b"not a database").unwrap();
    assert!(opencode::history_in(&db, &cwd, 0).is_none(), "an unreadable database is a failed read");
}

/// OpenCode writing while Lantern reads, at the 80-message edge: the coverage and the messages
/// come from the same moment, so a session growing past 80 is never read as a whole one (which
/// Ask would take as a rewind and drop what it kept).
#[test]
fn opencode_history_coverage_holds_while_opencode_writes() {
    use crate::readers::opencode;
    let dir = temp_dir("opencode-interleave");
    let db = dir.join("opencode.db");
    let cwd = dir.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE session(id TEXT, title TEXT, directory TEXT, parent_id TEXT, time_created INTEGER);
         CREATE TABLE message(id TEXT, session_id TEXT, time_updated INTEGER, data TEXT);
         CREATE TABLE part(id TEXT, message_id TEXT, data TEXT);",
    )
    .unwrap();
    c.execute("INSERT INTO session VALUES('ses_1','t',?1,NULL,1000)", [cwd.to_string_lossy().into_owned()]).unwrap();
    let message = |n: usize, parts: bool| {
        let id = format!("msg_{n:04}");
        let role = if n.is_multiple_of(2) { "user" } else { "assistant" };
        let data = format!("{{\"role\":\"{role}\",\"time\":{{\"created\":{}}}}}", 1_790_000_000_000u64 + n as u64);
        c.execute("INSERT INTO message VALUES(?1,'ses_1',?2,?3)", rusqlite::params![id, 1_790_000_000_000i64 + n as i64, data]).unwrap();
        if parts {
            let part = format!("{{\"type\":\"text\",\"text\":\"m{n}\"}}");
            c.execute("INSERT INTO part VALUES(?1,?2,?3)", rusqlite::params![format!("prt_{n:04}"), id, part]).unwrap();
        }
    };
    for n in 0..80 {
        message(n, true);
    }
    let whole = opencode::history_with(&db, &cwd, 0, || {}).unwrap();
    assert!(whole.complete, "80 messages: the whole session");
    assert_eq!(whole.messages.first().map(|m| m.text.as_str()), Some("m0"));

    // The 81st arrives after the session is chosen, before its messages are read.
    let grown = opencode::history_with(&db, &cwd, 0, || message(80, true)).unwrap();
    assert!(!grown.complete, "81 messages read as 80 of them: not the whole session");
    assert_eq!(grown.messages.first().map(|m| m.text.as_str()), Some("m1"), "the window moved on");
    assert_eq!(grown.messages.last().map(|m| m.text.as_str()), Some("m80"));

    // The 82nd, started but without its first part yet.
    let partial = opencode::history_with(&db, &cwd, 0, || message(81, false)).unwrap();
    assert!(!partial.complete);
    assert_eq!(partial.messages.last().map(|m| m.text.as_str()), Some("m80"));
}

/// Goose status and history from a current database: the session is matched by its own
/// `working_dir`, archived, scheduled and child sessions are excluded even when newer, an
/// assistant reply is your turn, and anything after it is working again.
#[test]
fn goose_status_and_history_from_a_current_database() {
    use crate::readers::goose;
    let dir = temp_dir("goose-current");
    let db = dir.join("sessions.db");
    let cwd = dir.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE TABLE schema_version(version INTEGER PRIMARY KEY);
         INSERT INTO schema_version VALUES(18);
         CREATE TABLE sessions(id TEXT PRIMARY KEY, name TEXT, description TEXT, working_dir TEXT,
           created_at TIMESTAMP, session_type TEXT DEFAULT 'user', archived_at TIMESTAMP, parent_session_id TEXT);
         CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT, session_id TEXT,
           role TEXT, content_json TEXT, created_timestamp INTEGER, metadata_json TEXT);",
    )
    .unwrap();
    let dir_s = cwd.to_string_lossy().into_owned();
    let session = |id: &str, dir: &str, kind: &str, archived: Option<&str>, parent: Option<&str>| {
        c.execute(
            "INSERT INTO sessions(id, name, description, working_dir, created_at, session_type, archived_at, parent_session_id) \
             VALUES(?1, 'main', '', ?2, '2026-11-01 00:00:00', ?3, ?4, ?5)",
            rusqlite::params![id, dir, kind, archived, parent],
        )
        .unwrap();
    };
    // Each decoy is newer than the real session, and each is excluded for its own reason.
    session("20261101_1", &dir_s, "user", None, None);
    session("20261101_2", &dir_s, "user", Some("2026-11-02 00:00:00"), None);
    session("20261101_3", &dir_s, "scheduled", None, None);
    session("20261101_4", &dir_s, "user", None, Some("20261101_1"));
    session("20261101_5", "/elsewhere", "user", None, None);
    let message = |session: &str, mid: &str, role: &str, content: &str, at: i64, meta: &str| {
        c.execute(
            "INSERT INTO messages(message_id, session_id, role, content_json, created_timestamp, metadata_json) VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![mid, session, role, content, at, meta],
        )
        .unwrap();
    };
    let t = 1_791_000_000i64;
    message("20261101_1", "msg_a_1", "user", r#"[{"type":"text","text":"list the files"}]"#, t, "{}");
    message(
        "20261101_1",
        "msg_a_2",
        "assistant",
        r#"[{"type":"toolRequest","id":"r1","toolCall":{"status":"success","value":{"name":"shell","arguments":{"command":"ls"}}}}]"#,
        t + 1,
        "{}",
    );
    message("20261101_1", "msg_a_3", "assistant", r#"[{"type":"thinking","thinking":"hmm"}]"#, t + 2, "{}");
    message("20261101_1", "msg_a_4", "assistant", r#"[{"type":"text","text":"three files"}]"#, t + 3, "{}");
    message("20261101_2", "msg_x_1", "user", r#"[{"type":"text","text":"archived"}]"#, t + 100, "{}");
    message("20261101_3", "msg_y_1", "user", r#"[{"type":"text","text":"scheduled"}]"#, t + 100, "{}");
    message("20261101_4", "msg_z_1", "user", r#"[{"type":"text","text":"child"}]"#, t + 100, "{}");
    message("20261101_5", "msg_w_1", "user", r#"[{"type":"text","text":"elsewhere"}]"#, t + 100, "{}");

    let status = goose::status_in(&db, &cwd, 0).unwrap();
    assert_eq!(status.session_id.as_deref(), Some("20261101_1"));
    assert_eq!(status.title.as_deref(), Some("main"));
    assert_eq!(status.state, Some(State::Waiting));
    assert_eq!(status.last_message.as_deref(), Some("three files"));

    let chat = goose::history_in(&db, &cwd, 0).unwrap();
    assert!(chat.complete, "four messages: the whole session");
    assert_eq!(chat.messages.len(), 3, "thinking is not conversation");
    assert!(matches!(chat.messages[0].role, history::Role::User));
    assert_eq!(chat.messages[0].text, "list the files");
    assert_eq!(chat.messages[0].id.as_deref(), Some("msg_a_1.0"));
    assert!(chat.messages[0].at.is_some(), "entries carry their message's time");
    assert!(matches!(chat.messages[1].role, history::Role::Tool));
    assert!(chat.messages[1].text.contains("shell"), "{}", chat.messages[1].text);
    assert_eq!(chat.messages[2].text, "three files");

    // A new prompt is working again; a tool call mid-turn is working too.
    message("20261101_1", "msg_a_5", "user", r#"[{"type":"text","text":"and again"}]"#, t + 4, "{}");
    assert_eq!(goose::status_in(&db, &cwd, 0).unwrap().state, Some(State::Working));
    // Built as JSON, so a stray brace can't silently void the tool call again.
    let tool_reply = serde_json::json!([
        {"type": "text", "text": "on it"},
        {"type": "toolRequest", "id": "r2", "toolCall": {"status": "success", "value": {"name": "shell", "arguments": {}}}},
    ])
    .to_string();
    message("20261101_1", "msg_a_6", "assistant", &tool_reply, t + 5, "{}");
    let status = goose::status_in(&db, &cwd, 0).unwrap();
    assert_eq!(status.state, Some(State::Working), "a tool call means mid-turn, even with text");
    assert_eq!(status.last_message, None);
    // The tool call is parsed, not dropped as malformed JSON: it shows in the chat.
    let chat = goose::history_in(&db, &cwd, 0).unwrap();
    assert!(
        chat.messages
            .iter()
            .any(|m| m.id.as_deref() == Some("msg_a_6.1") && matches!(m.role, history::Role::Tool) && m.text.contains("shell")),
        "the tool call is parsed and appears in history: {:?}",
        chat.messages.iter().map(|m| &m.id).collect::<Vec<_>>()
    );

    // A message goose itself hides stays out of the chat.
    message("20261101_1", "msg_a_7", "user", r#"[{"type":"text","text":"hidden context"}]"#, t + 6, r#"{"userVisible":false}"#);
    let chat = goose::history_in(&db, &cwd, 0).unwrap();
    assert!(!chat.messages.iter().any(|m| m.text.contains("hidden context")), "userVisible:false is not conversation");
}

/// An old Goose database: no `schema_version`, none of the newer columns, millisecond clocks.
/// The title falls back to the first user text, and ids fall back to the row.
#[test]
fn goose_reads_an_old_database_without_the_new_columns() {
    use crate::readers::goose;
    let dir = temp_dir("goose-old");
    let db = dir.join("sessions.db");
    let cwd = dir.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE TABLE sessions(id TEXT PRIMARY KEY, description TEXT, working_dir TEXT, created_at TIMESTAMP);
         CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT, role TEXT,
           content_json TEXT, created_timestamp INTEGER);",
    )
    .unwrap();
    c.execute("INSERT INTO sessions VALUES('20260101_1','',?1,'2026-01-01 00:00:00')", [cwd.to_string_lossy().into_owned()]).unwrap();
    c.execute(
        "INSERT INTO messages(session_id, role, content_json, created_timestamp) VALUES('20260101_1','user',?1,1767225600000)",
        [r#"[{"type":"text","text":"first line\nsecond line"}]"#],
    )
    .unwrap();
    c.execute(
        "INSERT INTO messages(session_id, role, content_json, created_timestamp) VALUES('20260101_1','assistant',?1,1767225660000)",
        [r#"[{"type":"text","text":"old reply"}]"#],
    )
    .unwrap();

    let status = goose::status_in(&db, &cwd, 0).unwrap();
    assert_eq!(status.session_id.as_deref(), Some("20260101_1"));
    assert_eq!(status.title.as_deref(), Some("first line"), "no name column: the first prompt names it");
    assert_eq!(status.state, Some(State::Waiting));
    assert_eq!(status.since, Some(1_767_225_660_000), "millisecond clocks are normalized, not divided twice");
    assert_eq!(status.last_message.as_deref(), Some("old reply"));

    let chat = goose::history_in(&db, &cwd, 0).unwrap();
    assert!(chat.complete);
    assert_eq!(chat.messages.len(), 2);
    assert_eq!(chat.messages[0].id.as_deref(), Some("1.0"), "no message_id column: the row is the id");
}

/// A database from a newer Goose is read as long as the tables and columns the reader needs are
/// there; an unreadable one, or one without them, is a failed read.
#[test]
fn goose_reads_newer_versions_and_fails_on_unreadable_databases() {
    use crate::readers::goose;
    let dir = temp_dir("goose-unknown");
    let cwd = dir.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.canonicalize().unwrap();

    let db = dir.join("sessions.db");
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE TABLE schema_version(version INTEGER PRIMARY KEY);
         INSERT INTO schema_version VALUES(99);
         CREATE TABLE sessions(id TEXT PRIMARY KEY, name TEXT, working_dir TEXT, created_at TIMESTAMP, session_type TEXT);
         CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT, session_id TEXT,
           role TEXT, content_json TEXT, created_timestamp INTEGER, metadata_json TEXT);",
    )
    .unwrap();
    c.execute("INSERT INTO sessions VALUES('20261101_1','new',?1,'2026-11-01 00:00:00','user')", [cwd.to_string_lossy().into_owned()])
        .unwrap();
    let status = goose::status_in(&db, &cwd, 0).unwrap();
    assert_eq!(status.session_id.as_deref(), Some("20261101_1"), "schema 99 with the columns the reader needs is read");
    assert_eq!(status.state, Some(State::Idle), "no messages yet");
    assert!(goose::history_in(&db, &cwd, 0).is_some());
    drop(c);

    fs::write(&db, b"not a database").unwrap();
    assert!(goose::status_in(&db, &cwd, 0).is_none(), "an unreadable database is a failed read");
    assert!(goose::history_in(&db, &cwd, 0).is_none());

    let db = dir.join("other.db");
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY, name TEXT);").unwrap();
    drop(c);
    assert!(goose::status_in(&db, &cwd, 0).is_none(), "no messages table: not a sessions database");
    assert!(goose::history_in(&db, &cwd, 0).is_none());
}

/// A session with messages only from before the process started isn't its conversation; one with
/// no messages yet is idle.
#[test]
fn goose_matches_only_sessions_active_since_the_process_started() {
    use crate::readers::goose;
    let dir = temp_dir("goose-stale");
    let db = dir.join("sessions.db");
    let cwd = dir.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE TABLE sessions(id TEXT PRIMARY KEY, name TEXT, working_dir TEXT, created_at TIMESTAMP, session_type TEXT);
         CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT, role TEXT,
           content_json TEXT, created_timestamp INTEGER);",
    )
    .unwrap();
    c.execute("INSERT INTO sessions VALUES('old','',?1,'2026-01-01 00:00:00','user')", [cwd.to_string_lossy().into_owned()]).unwrap();
    c.execute(
        "INSERT INTO messages(session_id, role, content_json, created_timestamp) VALUES('old','assistant',?1,1767225600)",
        [r#"[{"type":"text","text":"yesterday"}]"#],
    )
    .unwrap();
    // A clean lookup with no session for this process is Idle, not a failed read.
    let status = goose::status_in(&db, &cwd, 1_800_000_000).unwrap();
    assert_eq!(status.session_id, None, "nothing since the process started: no session");
    assert_eq!(status.state, Some(State::Idle));
    assert!(goose::history_in(&db, &cwd, 1_800_000_000).unwrap().messages.is_empty());

    c.execute("INSERT INTO sessions VALUES('new','',?1,'2026-11-12 00:00:01','user')", [cwd.to_string_lossy().into_owned()]).unwrap();
    let status = goose::status_in(&db, &cwd, 1_794_441_600).unwrap();
    assert_eq!(status.session_id.as_deref(), Some("new"));
    assert_eq!(status.state, Some(State::Idle), "created but nothing written yet");
}

/// A failed turn is your turn, with the error as its last message: a provider error, the
/// turn-limit error, and exhausted credits. Ordinary notifications stay as they are.
#[test]
fn goose_failed_turns_are_waiting_with_the_error() {
    use crate::readers::goose;
    let dir = temp_dir("goose-errors");
    let db = dir.join("sessions.db");
    let cwd = dir.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE TABLE schema_version(version INTEGER PRIMARY KEY);
         INSERT INTO schema_version VALUES(18);
         CREATE TABLE sessions(id TEXT PRIMARY KEY, name TEXT, working_dir TEXT, created_at TIMESTAMP, session_type TEXT);
         CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT, session_id TEXT,
           role TEXT, content_json TEXT, created_timestamp INTEGER, metadata_json TEXT);",
    )
    .unwrap();
    c.execute("INSERT INTO sessions VALUES('s1','main',?1,'2026-11-01 00:00:00','user')", [cwd.to_string_lossy().into_owned()]).unwrap();
    let message = |mid: &str, role: &str, content: &str, at: i64| {
        c.execute(
            "INSERT INTO messages(message_id, session_id, role, content_json, created_timestamp, metadata_json) \
             VALUES(?1,'s1',?2,?3,?4,'{}')",
            rusqlite::params![mid, role, content, at],
        )
        .unwrap();
    };
    let t = 1_791_000_000i64;
    message("e1", "user", r#"[{"type":"text","text":"go"}]"#, t);

    // A provider failure ends the turn.
    message("e2", "assistant", r#"[{"type":"error","message":"Provider error: connection reset"}]"#, t + 1);
    let status = goose::status_in(&db, &cwd, 0).unwrap();
    assert_eq!(status.state, Some(State::Waiting));
    assert_eq!(status.last_message.as_deref(), Some("Provider error: connection reset"));

    // So does the turn limit.
    message("e3", "user", r#"[{"type":"text","text":"again"}]"#, t + 2);
    message("e4", "assistant", r#"[{"type":"error","message":"Maximum turns reached"}]"#, t + 3);
    let status = goose::status_in(&db, &cwd, 0).unwrap();
    assert_eq!(status.state, Some(State::Waiting));
    assert_eq!(status.last_message.as_deref(), Some("Maximum turns reached"));

    // And exhausted credits, which goose saves as a system notification.
    message("e5", "user", r#"[{"type":"text","text":"once more"}]"#, t + 4);
    message(
        "e6",
        "assistant",
        r#"[{"type":"systemNotification","notificationType":"creditsExhausted","msg":"Out of credits for today"}]"#,
        t + 5,
    );
    let status = goose::status_in(&db, &cwd, 0).unwrap();
    assert_eq!(status.state, Some(State::Waiting));
    assert_eq!(status.last_message.as_deref(), Some("Out of credits for today"));

    // The errors are conversation: they show in the chat.
    let chat = goose::history_in(&db, &cwd, 0).unwrap();
    let agent_texts: Vec<&str> = chat.messages.iter().filter(|m| matches!(m.role, history::Role::Agent)).map(|m| m.text.as_str()).collect();
    assert!(agent_texts.contains(&"Provider error: connection reset"), "{agent_texts:?}");
    assert!(agent_texts.contains(&"Maximum turns reached"), "{agent_texts:?}");
    assert!(agent_texts.contains(&"Out of credits for today"), "{agent_texts:?}");

    // An ordinary notification is not a finished turn: on its own it leaves the card Working,
    // as before, and never becomes the last message.
    message("e7", "assistant", r#"[{"type":"systemNotification","notificationType":"inlineMessage","msg":"compacting"}]"#, t + 6);
    let status = goose::status_in(&db, &cwd, 0).unwrap();
    assert_eq!(status.state, Some(State::Working));
    assert_eq!(status.last_message, None);
}

/// Ten older millisecond-dated sessions must not crowd out a current seconds-dated one: the
/// clock is normalized before the pick, in both status and history.
#[test]
fn goose_picks_the_active_session_across_mixed_clocks() {
    use crate::readers::goose;
    let dir = temp_dir("goose-clocks");
    let db = dir.join("sessions.db");
    let cwd = dir.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE TABLE schema_version(version INTEGER PRIMARY KEY);
         INSERT INTO schema_version VALUES(18);
         CREATE TABLE sessions(id TEXT PRIMARY KEY, name TEXT, working_dir TEXT, created_at TIMESTAMP, session_type TEXT);
         CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT, session_id TEXT,
           role TEXT, content_json TEXT, created_timestamp INTEGER, metadata_json TEXT);",
    )
    .unwrap();
    let dir_s = cwd.to_string_lossy().into_owned();
    // Ten older sessions dated in milliseconds: raw, each dwarfs any seconds clock.
    for i in 0..10 {
        c.execute(
            "INSERT INTO sessions(id, name, working_dir, created_at, session_type) VALUES(?1,'old',?2,'2026-01-01 00:00:00','user')",
            rusqlite::params![format!("old_{i}"), dir_s],
        )
        .unwrap();
        c.execute(
            "INSERT INTO messages(message_id, session_id, role, content_json, created_timestamp, metadata_json) \
             VALUES(?1,?2,'assistant',?3,1767225600000,'{}')",
            rusqlite::params![format!("old_{i}_m"), format!("old_{i}"), r#"[{"type":"text","text":"old"}]"#],
        )
        .unwrap();
    }
    // One current session dated in seconds.
    c.execute("INSERT INTO sessions VALUES('now','current',?1,'2026-11-12 00:00:00','user')", [&dir_s]).unwrap();
    c.execute(
        "INSERT INTO messages(message_id, session_id, role, content_json, created_timestamp, metadata_json) \
         VALUES('now_u','now','user',?1,1795000000,'{}')",
        [r#"[{"type":"text","text":"current prompt"}]"#],
    )
    .unwrap();
    c.execute(
        "INSERT INTO messages(message_id, session_id, role, content_json, created_timestamp, metadata_json) \
         VALUES('now_a','now','assistant',?1,1795000001,'{}')",
        [r#"[{"type":"text","text":"current reply"}]"#],
    )
    .unwrap();

    let status = goose::status_in(&db, &cwd, 0).unwrap();
    assert_eq!(status.session_id.as_deref(), Some("now"));
    assert_eq!(status.state, Some(State::Waiting));
    assert_eq!(status.last_message.as_deref(), Some("current reply"));

    let chat = goose::history_in(&db, &cwd, 0).unwrap();
    assert_eq!(chat.messages.len(), 2);
    assert_eq!(chat.messages[0].text, "current prompt");
    assert_eq!(chat.messages[1].text, "current reply");
}

/// `CLI Session` is goose's placeholder name: the card shows the first prompt you can see
/// instead, never a hidden message.
#[test]
fn goose_titles_an_unnamed_session_from_its_first_visible_prompt() {
    use crate::readers::goose;
    let dir = temp_dir("goose-title");
    let db = dir.join("sessions.db");
    let cwd = dir.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE TABLE schema_version(version INTEGER PRIMARY KEY);
         INSERT INTO schema_version VALUES(18);
         CREATE TABLE sessions(id TEXT PRIMARY KEY, name TEXT, working_dir TEXT, created_at TIMESTAMP, session_type TEXT);
         CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT, session_id TEXT,
           role TEXT, content_json TEXT, created_timestamp INTEGER, metadata_json TEXT);",
    )
    .unwrap();
    c.execute("INSERT INTO sessions VALUES('s1','CLI Session',?1,'2026-11-01 00:00:00','user')", [cwd.to_string_lossy().into_owned()])
        .unwrap();
    let message = |mid: &str, role: &str, content: &str, at: i64, meta: &str| {
        c.execute(
            "INSERT INTO messages(message_id, session_id, role, content_json, created_timestamp, metadata_json) \
             VALUES(?1,'s1',?2,?3,?4,?5)",
            rusqlite::params![mid, role, content, at, meta],
        )
        .unwrap();
    };
    let t = 1_791_000_000i64;
    message("t1", "user", r#"[{"type":"text","text":"hidden context"}]"#, t, r#"{"userVisible":false}"#);
    message("t2", "user", r#"[{"type":"text","text":"visible prompt\nsecond line"}]"#, t + 1, "{}");
    message("t3", "assistant", r#"[{"type":"text","text":"reply"}]"#, t + 2, "{}");

    let status = goose::status_in(&db, &cwd, 0).unwrap();
    assert_eq!(status.title.as_deref(), Some("visible prompt"));
}

/// On Windows goose stores the plain folder, never the canonicalized (`\\?\…`) form, so the
/// reader matches the folder as given. (Unix keeps canonicalizing both sides.)
#[test]
#[cfg(windows)]
fn goose_matches_the_plain_folder_on_windows() {
    use crate::readers::goose;
    let dir = temp_dir("goose-winpath");
    let db = dir.join("sessions.db");
    let cwd = dir.join("work");
    fs::create_dir_all(&cwd).unwrap();
    // Deliberately not canonicalized: this is the form goose writes.
    assert!(!cwd.to_string_lossy().starts_with(r"\\?\"), "the test needs a plain path to prove anything");
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE TABLE schema_version(version INTEGER PRIMARY KEY);
         INSERT INTO schema_version VALUES(18);
         CREATE TABLE sessions(id TEXT PRIMARY KEY, name TEXT, working_dir TEXT, created_at TIMESTAMP, session_type TEXT);
         CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT, session_id TEXT,
           role TEXT, content_json TEXT, created_timestamp INTEGER, metadata_json TEXT);",
    )
    .unwrap();
    c.execute("INSERT INTO sessions VALUES('s1','main',?1,'2026-11-01 00:00:00','user')", [cwd.to_string_lossy().into_owned()]).unwrap();
    c.execute(
        "INSERT INTO messages(message_id, session_id, role, content_json, created_timestamp, metadata_json) \
         VALUES('w1','s1','user',?1,1791000000,'{}')",
        [r#"[{"type":"text","text":"hi"}]"#],
    )
    .unwrap();
    c.execute(
        "INSERT INTO messages(message_id, session_id, role, content_json, created_timestamp, metadata_json) \
         VALUES('w2','s1','assistant',?1,1791000001,'{}')",
        [r#"[{"type":"text","text":"hello"}]"#],
    )
    .unwrap();

    let status = goose::status_in(&db, &cwd, 0).unwrap();
    assert_eq!(status.session_id.as_deref(), Some("s1"));
    assert_eq!(status.state, Some(State::Waiting));
}

/// A session file that can't be read is a failed read, which isn't cached as an empty one.
#[test]
fn an_unreadable_session_file_is_a_failed_read() {
    let dir = temp_dir("unreadable");
    assert!(history::claude(&dir.join("missing.jsonl")).is_none());
    assert!(crate::readers::pi::history(&dir.join("missing.jsonl")).is_none());
    assert!(crate::readers::gemini::history(&dir.join("missing.jsonl")).is_none());
    assert!(crate::readers::grok::history(&dir.join("missing.jsonl")).is_none());
}
