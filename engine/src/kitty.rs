//! Replying to agents in kitty, and jumping to their windows.
//!
//! kitty's remote control addresses windows by id: `kitty @ --to <socket> action --match
//! id:<id> send_text all <text>` and `kitty @ --to <socket> focus-window --match id:<id>`.
//! kitty exports the window id (`KITTY_WINDOW_ID`) in every child's environment, and the
//! control socket (`KITTY_LISTEN_ON`) when a control socket exists: `kitty.conf` needs
//! `listen_on unix:/tmp/kitty` (or `kitty --listen-on`) with `allow_remote_control` permitting
//! it, then a restart. `socket-only` is the setup to use (Lantern passes no password, so a
//! password challenge it cannot answer means no route); a kitty without a socket simply has no
//! route. The scanner reads both values from the agent's own environment (see scanner.rs).
//!
//! Direct input needs kitty 0.50.0 or newer: older releases ignore the write's `--match` and
//! act on the active window instead, while still reporting success. Lantern never trusts the
//! version on disk (an upgraded app can talk to an older running kitty); it probes the running
//! instance instead. The probe asks for the no-op `discard_event` action on a window id that
//! cannot exist: a fixed kitty matches the target before dispatching and refuses with `No
//! matching windows`, while an affected one runs the no-op and reports success. Only the
//! no-match refusal grants a direct route; anything else — success, denial, timeout, another
//! error — means `Route::None`, and replies stay copy and paste. The scan caches the answer
//! briefly (see scanner.rs), and every send checks the running instance again, so a restarted
//! kitty is always re-gated.
//!
//! Every reply and key also checks the window is still the agent's: `kitty @ ls` lists each
//! window with the pids in its foreground, and the send is refused unless the agent's pid is in
//! its own window's list. That stops typing into whatever sits between kitty and the agent
//! (Neovim's `:terminal`).
//!
//! The checks only prove the destination; the write itself goes through `action`, which waits
//! for kitty's answer (unlike `send-text`, which always reports success even when nothing
//! arrived). A denied or unhandled write fails, so Lantern keeps the draft instead of saying
//! Sent. Each write is acknowledged on its own: text, then Enter, then the key. After a partial
//! delivery (the text went in but Enter didn't) or a timeout, the outcome is uncertain, never
//! success. An acknowledgement proves the write ran on the requested window (the gate above
//! established targeting first); it does not prove the agent submitted the reply.
//!
//! The `send_text` action takes a keyboard mode and the text (`all` matches any mode); its
//! parser expands ANSI-C escapes server-side and drops the whitespace between the mode and the
//! text, and around the whole action. So user text goes in with its backslashes doubled and its
//! leading/trailing whitespace as escapes, and keys go as the bytes the terminal would deliver
//! (`\e`, `\t`, `\r`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// The CLI from the agent's own app (`<bundle>/Contents/MacOS/kitty`), when it exists: the CLI
/// and the instance always come from the same install, wherever it lives.
pub fn cli_for(bundle_path: &str) -> Option<PathBuf> {
    let cli = Path::new(bundle_path).join("Contents/MacOS/kitty");
    cli.is_file().then_some(cli)
}

/// How long the send-time window check may take. It types nothing, so running out of time is a
/// refusal, not an uncertainty.
const LS_LIMIT: Duration = Duration::from_secs(5);

/// The exact `kitty @ …` argv: the global `--to` before the subcommand, `--match` after it.
fn argv(socket: &str, subcommand: &str, window: &str, rest: &[&str]) -> Vec<String> {
    let mut full = vec![
        "@".to_string(),
        "--to".to_string(),
        socket.to_string(),
        subcommand.to_string(),
        "--match".to_string(),
        format!("id:{window}"),
    ];
    full.extend(rest.iter().map(|s| s.to_string()));
    full
}

/// The `action` argv for one acknowledged write. `send_text` is the first positional, so what
/// follows it (however dash-prefixed) is action text, never options — and no `--`: it would join
/// into the action string itself.
fn action_argv(socket: &str, window: &str, payload: &str) -> Vec<String> {
    argv(socket, "action", window, &["send_text", "all", payload])
}

fn subcommand(argv: &[String]) -> &str {
    argv.get(3).map(String::as_str).unwrap_or("@")
}

fn run(cli: &Path, argv: Vec<String>, typing: bool) -> Result<()> {
    let out = crate::deliver::output_within(Command::new(cli).args(&argv), crate::deliver::SEND_LIMIT, typing)?;
    check_action(&argv, &out)
}

/// `kitty @ action` waits for kitty's answer, so a failure here (a denied write, an unhandled
/// action) is a refusal: nothing was typed, the draft stays, no Sent is shown.
fn check_action(argv: &[String], out: &std::process::Output) -> Result<()> {
    if !out.status.success() {
        bail!("kitty {} failed: {}", subcommand(argv), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// A window id no kitty can have: ids are small sequential numbers, so the probe's target can
/// never match a real window.
const BOGUS_WINDOW: &str = "4294967295";

/// The capability probe: the no-op `discard_event` action on the impossible target. A fixed
/// kitty refuses with `No matching windows`; an affected one runs the no-op and succeeds.
fn probe_argv(socket: &str) -> Vec<String> {
    argv(socket, "action", BOGUS_WINDOW, &["discard_event"])
}

/// How long the capability probe may take. It types nothing, so running out of time means no
/// route, never uncertainty.
const PROBE_LIMIT: Duration = Duration::from_secs(5);

/// True only when the probe failed with the recognized no-match refusal: the running kitty
/// honors `--match`. Probe success (affected kitty runs the no-op), denial, and any other
/// failure all mean no route — the gate fails closed, so an unrecognized answer disables direct
/// delivery instead of risking a mistype.
fn targeting_honored(out: &std::process::Output) -> bool {
    if out.status.success() {
        return false;
    }
    String::from_utf8_lossy(&out.stderr).contains("No matching windows")
}

/// Whether the kitty behind `socket` honors `--match` for `action`. Anything but the
/// recognized no-match refusal — including a probe that never answers — is `false`.
pub fn targeting_works(cli: &Path, socket: &str) -> bool {
    match crate::deliver::output_within(Command::new(cli).args(probe_argv(socket)), PROBE_LIMIT, false) {
        Ok(out) => {
            let honored = targeting_honored(&out);
            if !honored {
                if out.status.success() {
                    crate::elog::line("kitty targeting probe ran: this kitty predates targeted input (needs 0.50.0 or newer)");
                } else {
                    crate::elog::line(format!(
                        "kitty targeting probe refused without the no-match answer: {}",
                        String::from_utf8_lossy(&out.stderr).trim()
                    ));
                }
            }
            honored
        }
        Err(e) => {
            crate::elog::line(format!("kitty targeting probe failed to run: {e:#}"));
            false
        }
    }
}

/// Refuses unless the running kitty still honors `--match`: a restarted instance is caught at
/// send time even when the scan saw a healthy one. Nothing is typed either way.
fn verify_targeting(cli: &Path, socket: &str) -> Result<()> {
    if targeting_works(cli, socket) {
        Ok(())
    } else {
        bail!("kitty didn't prove it honors targeted input (needs 0.50.0 or newer); nothing was typed")
    }
}

/// What `kitty @ ls` reports, trimmed to what the send-time check needs: each window's id and
/// the pids in its foreground (kitty `window.py` `as_dict`). Everything else is ignored, so
/// newer kitty fields never break the parse.
#[derive(Deserialize)]
struct LsOsWindow {
    #[serde(default)]
    tabs: Vec<LsTab>,
}

#[derive(Deserialize)]
struct LsTab {
    #[serde(default)]
    windows: Vec<LsWindow>,
}

#[derive(Deserialize)]
struct LsWindow {
    id: u64,
    #[serde(default)]
    foreground_processes: Vec<LsProcess>,
}

#[derive(Deserialize)]
struct LsProcess {
    pid: u32,
}

/// Refuses unless the agent's window is in the `ls` output with the agent's pid in its
/// foreground: a window that moved, or something sitting between kitty and the agent, is a
/// refusal, not a mistype.
fn check_ls(os_windows: &[LsOsWindow], window: u64, pid: u32, handle: &str) -> Result<()> {
    let found = os_windows.iter().flat_map(|o| &o.tabs).flat_map(|t| &t.windows).find(|w| w.id == window);
    let Some(found) = found else { bail!("@{handle}'s kitty window is gone") };
    if !found.foreground_processes.iter().any(|p| p.pid == pid) {
        bail!("@{handle} isn't in the foreground of its kitty window (something else is running there)");
    }
    Ok(())
}

/// The send-time check: the agent's window still exists on its own instance with the agent's
/// pid in its foreground. Refuses when `ls` errors.
pub fn verify_window(cli: &Path, socket: &str, window: &str, agent: &crate::model::Agent) -> Result<()> {
    let Ok(id) = window.parse::<u64>() else { bail!("@{}'s kitty window isn't one Lantern understands", agent.handle) };
    let out = crate::deliver::output_within(Command::new(cli).args(argv(socket, "ls", window, &[])), LS_LIMIT, false)?;
    if !out.status.success() {
        bail!("kitty ls failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let os_windows: Vec<LsOsWindow> =
        serde_json::from_slice(&out.stdout).with_context(|| format!("@{}'s kitty didn't list its windows", agent.handle))?;
    check_ls(&os_windows, id, agent.pid, &agent.handle)
}

/// Whitespace (or anything control-like) at the edge of the text, as an ANSI-C escape: the
/// action parser would otherwise drop it with the whitespace around the mode and the action.
/// `\x20` always carries exactly two hex digits, so whatever follows can't join it.
fn escape_edge(c: char) -> String {
    match c {
        ' ' => "\\x20".to_string(),
        '\t' => "\\t".to_string(),
        c if (c as u32) <= 0xffff => format!("\\u{:04x}", c as u32),
        c => format!("\\U{:08x}", c as u32),
    }
}

fn is_edge(c: char) -> bool {
    c.is_whitespace() || c.is_control()
}

/// User text as the `send_text` action takes it: backslashes doubled (the parser expands
/// ANSI-C escapes), and leading/trailing whitespace as escapes (the parser drops it). The
/// middle goes through untouched: dashes, quotes, Unicode, inner spacing.
fn encode(text: &str) -> String {
    let leading_len = text.chars().take_while(|c| is_edge(*c)).map(char::len_utf8).sum::<usize>();
    let (leading, rest) = text.split_at(leading_len);
    let trailing_len = rest.chars().rev().take_while(|c| is_edge(*c)).map(char::len_utf8).sum::<usize>();
    let (middle, trailing) = rest.split_at(rest.len() - trailing_len);
    let mut out = String::with_capacity(text.len() + 16);
    for c in leading.chars() {
        out.push_str(&escape_edge(c));
    }
    for c in middle.chars() {
        if c == '\\' {
            out.push_str("\\\\");
        } else {
            out.push(c);
        }
    }
    for c in trailing.chars() {
        out.push_str(&escape_edge(c));
    }
    out
}

/// Enter as its `send_text` payload.
const ENTER: &str = "\\r";

/// Type `text` into the window and press Enter (one line; a newline would submit early), each
/// write acknowledged by kitty. Empty text only presses Enter.
pub fn send_text(cli: &Path, socket: &str, window: &str, text: &str) -> Result<()> {
    verify_targeting(cli, socket)?;
    send_text_with(text, &mut |payload: &str| run(cli, action_argv(socket, window, payload), true))
}

/// The text→Enter sequence over any action runner: Enter only goes in after the text was
/// acknowledged, and an Enter kitty never confirmed is uncertainty (the text is already in) —
/// unless no text went in, which is a plain refusal.
fn send_text_with(text: &str, act: &mut dyn FnMut(&str) -> Result<()>) -> Result<()> {
    let line = text.replace(['\r', '\n'], " ");
    if !line.is_empty() {
        act(&encode(&line))?;
    }
    // Give the TUI a moment to take the text before Enter.
    std::thread::sleep(Duration::from_millis(60));
    act(ENTER).map_err(|e| {
        if line.is_empty() { e } else { crate::deliver::Uncertain(format!("the text went in but Enter didn't ({e:#})")).into() }
    })
}

/// One key as its `send_text` payload: the bytes the terminal would deliver for it.
fn stroke(key: crate::deliver::Key) -> String {
    match key {
        crate::deliver::Key::Escape => "\\e".to_string(),
        crate::deliver::Key::Tab => "\\t".to_string(),
        crate::deliver::Key::Enter => "\\r".to_string(),
        crate::deliver::Key::Char(c) => encode(&c.to_string()),
    }
}

/// Press one key: Escape, Tab, Enter or a single character. A denied key is a refusal: nothing
/// was typed.
pub fn send_key(cli: &Path, socket: &str, window: &str, key: crate::deliver::Key) -> Result<()> {
    verify_targeting(cli, socket)?;
    let stroke = stroke(key);
    run(cli, action_argv(socket, window, &stroke), true)
}

/// Bring the window's tab to the front (the app activates kitty afterwards). `focus-window`
/// honors its match on every release, but Open shares the reply's route: one requirement for
/// both, and an ungated kitty only comes forward.
pub fn activate(cli: &Path, socket: &str, window: &str) -> Result<()> {
    verify_targeting(cli, socket)?;
    run(cli, argv(socket, "focus-window", window, &[]), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_commands_are_well_formed() {
        assert_eq!(
            action_argv("unix:/tmp/kitty-123", "3", "hello"),
            ["@", "--to", "unix:/tmp/kitty-123", "action", "--match", "id:3", "send_text", "all", "hello"]
        );
        assert_eq!(
            argv("unix:/tmp/kitty-123", "focus-window", "3", &[]),
            ["@", "--to", "unix:/tmp/kitty-123", "focus-window", "--match", "id:3"]
        );
        assert_eq!(argv("unix:/tmp/kitty-123", "ls", "3", &[]), ["@", "--to", "unix:/tmp/kitty-123", "ls", "--match", "id:3"]);
    }

    #[test]
    fn dash_payloads_are_typed_not_parsed() {
        for payload in ["-h hello", "--", "--from-file=/etc/hosts"] {
            let full = action_argv("unix:/tmp/kitty-123", "3", payload);
            let action_at = full.iter().position(|a| a == "send_text").expect("the send_text action");
            assert_eq!(&full[action_at..], ["send_text", "all", payload], "the payload rides after the first positional");
            assert!(!full[..action_at].iter().any(|a| a == "--"), "no separator before the action");
        }
    }

    #[test]
    fn backslashes_stay_literal() {
        assert_eq!(encode("plain"), "plain");
        assert_eq!(encode("C:\\temp\\new"), "C:\\\\temp\\\\new");
        assert_eq!(encode("\\e"), "\\\\e", "an escape-looking reply must not become Escape");
        assert_eq!(encode("\\n"), "\\\\n", "an escape-looking reply must not become a newline");
    }

    #[test]
    fn edge_whitespace_survives_the_action_parser() {
        assert_eq!(encode("  padded  "), "\\x20\\x20padded\\x20\\x20");
        assert_eq!(encode("\tindented"), "\\tindented");
        assert_eq!(encode("trailing\t"), "trailing\\t");
        assert_eq!(encode("   "), "\\x20\\x20\\x20", "an all-space reply must still type something");
        assert_eq!(encode("a  b"), "a  b", "inner spacing goes through untouched");
        assert_eq!(encode(""), "", "empty text stays empty (the text step is skipped)");
    }

    #[test]
    fn quotes_and_unicode_go_through_untouched() {
        assert_eq!(encode(r#"say "hi" 'bye'"#), r#"say "hi" 'bye'"#);
        assert_eq!(encode("héllo → ✓"), "héllo → ✓");
        assert_eq!(encode("-1"), "-1");
    }

    #[test]
    fn keys_send_as_terminal_bytes() {
        assert_eq!(stroke(crate::deliver::Key::Escape), "\\e");
        assert_eq!(stroke(crate::deliver::Key::Tab), "\\t");
        assert_eq!(stroke(crate::deliver::Key::Enter), "\\r");
        assert_eq!(stroke(crate::deliver::Key::Char('1')), "1");
        assert_eq!(stroke(crate::deliver::Key::Char('\\')), "\\\\");
        assert_eq!(stroke(crate::deliver::Key::Char(' ')), "\\x20", "a space key must survive the action parser");
    }

    #[test]
    fn denied_write_is_failure_not_sent() {
        let mut attempted = Vec::new();
        let denied = send_text_with("hello", &mut |payload: &str| {
            attempted.push(payload.to_string());
            Err(anyhow::anyhow!("kitty action failed: not allowed"))
        });
        let e = denied.expect_err("a denied write must fail");
        assert!(e.downcast_ref::<crate::deliver::Uncertain>().is_none(), "nothing was typed, so it is a refusal, not uncertainty");
        assert_eq!(attempted, ["hello"], "Enter must not follow a denied text");
    }

    #[test]
    fn unconfirmed_enter_is_uncertain() {
        let mut attempted = Vec::new();
        let outcome = send_text_with("hello", &mut |payload: &str| {
            attempted.push(payload.to_string());
            if payload == ENTER { Err(anyhow::anyhow!("kitty action failed: not allowed")) } else { Ok(()) }
        });
        let e = outcome.expect_err("an unconfirmed Enter must not report success");
        assert!(e.downcast_ref::<crate::deliver::Uncertain>().is_some(), "the text is already in: uncertainty, not failure");
        assert_eq!(attempted, ["hello", ENTER]);
    }

    #[test]
    fn empty_text_only_presses_enter() {
        let mut attempted = Vec::new();
        send_text_with("", &mut |payload: &str| {
            attempted.push(payload.to_string());
            Ok(())
        })
        .expect("Enter still goes in");
        assert_eq!(attempted, [ENTER], "no empty action (kitty would reject it as unknown)");
    }

    #[test]
    fn empty_text_with_denied_enter_is_a_refusal() {
        let outcome = send_text_with("", &mut |_| Err(anyhow::anyhow!("kitty action failed: not allowed")));
        let e = outcome.expect_err("a denied Enter must fail");
        assert!(e.downcast_ref::<crate::deliver::Uncertain>().is_none(), "no text went in, so it is a refusal");
    }

    #[cfg(unix)]
    fn output_of(success: bool, stderr: &str) -> std::process::Output {
        use std::os::unix::process::ExitStatusExt;
        std::process::Output {
            status: std::process::ExitStatus::from_raw(if success { 0 } else { 1 << 8 }),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[cfg(windows)]
    fn output_of(success: bool, stderr: &str) -> std::process::Output {
        use std::os::windows::process::ExitStatusExt;
        std::process::Output {
            status: std::process::ExitStatus::from_raw(if success { 0 } else { 1 }),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn action_denial_propagates() {
        let argv = action_argv("unix:/tmp/kitty-123", "3", "hello");
        check_action(&argv, &output_of(true, "")).expect("an acknowledged write succeeds");
        let e = check_action(&argv, &output_of(false, "kitty: not allowed\n")).expect_err("a denied write fails");
        assert!(format!("{e:#}").contains("not allowed"), "kitty's reason reaches the user: {e:#}");
        check_action(&argv, &output_of(false, "")).expect_err("a bare failure still fails");
    }

    /// A trimmed `kitty @ ls` answer: the agent's window (id 3) with pid 4242 in its
    /// foreground, another window running something else, and an empty second OS window.
    const LS_SAMPLE: &str = r#"[
  {"id": 1, "tabs": [{"id": 1, "windows": [
    {"id": 3, "title": "agent", "pid": 4240, "foreground_processes": [{"pid": 4242, "cmdline": ["claude"], "cwd": "/tmp"}]},
    {"id": 4, "title": "other", "pid": 4300, "foreground_processes": [{"pid": 4310}]}
  ]}]},
  {"id": 2, "tabs": []}
]"#;

    #[test]
    fn window_check_reads_foreground_pids() {
        let os_windows: Vec<LsOsWindow> = serde_json::from_str(LS_SAMPLE).expect("the sample parses");
        check_ls(&os_windows, 3, 4242, "claude-1").expect("the agent's pid in its window passes");
        check_ls(&os_windows, 3, 9999, "claude-1").expect_err("a pid that isn't in the foreground refuses");
        check_ls(&os_windows, 4, 4242, "claude-1").expect_err("the agent's pid in another window refuses");
        check_ls(&os_windows, 7, 4242, "claude-1").expect_err("a missing window refuses");
    }

    #[test]
    fn probe_asks_the_noop_on_an_impossible_window() {
        assert_eq!(
            probe_argv("unix:/tmp/kitty-123"),
            ["@", "--to", "unix:/tmp/kitty-123", "action", "--match", "id:4294967295", "discard_event"]
        );
    }

    #[test]
    fn gate_only_trusts_the_no_match_refusal() {
        assert!(
            targeting_honored(&output_of(false, "No matching windows for expression: id:4294967295\n")),
            "the fixed refusal grants a route"
        );
        assert!(!targeting_honored(&output_of(true, "")), "probe success is an affected kitty: no route");
        assert!(!targeting_honored(&output_of(false, "kitty: not allowed\n")), "a permission denial is not proof of targeting: no route");
        assert!(
            !targeting_honored(&output_of(false, "Unknown action: discard_event\n")),
            "an unknown probe action proves nothing: no route"
        );
        assert!(!targeting_honored(&output_of(false, "")), "a bare failure proves nothing: no route");
    }
}
