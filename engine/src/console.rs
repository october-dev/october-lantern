//! Typing a reply into an agent's own Windows console: Windows Terminal, VS Code's terminal or a
//! console window, for agents Lantern didn't start (its own sessions go through pty.rs).
//!
//! Attaching to a console changes the whole process, so the engine never does it itself: each
//! delivery runs `lantern-engine console-type`, which attaches to the agent's console, checks it,
//! writes the keystrokes into the console's input and exits.
//!
//! Windows can't say which process reads a console's input, so instead of the foreground check
//! used on macOS and Linux the helper refuses unless:
//! - the agent is the process Lantern saw (same pid and start time), still alive,
//! - it is still attached to the console (so the text can't land in a shell after it exits),
//! - nothing else is attached except the agent and the programs that started it (its shell). A
//!   tool the agent is running in that terminal would be attached too, and could read the text.
use anyhow::Result;

/// One key press: its virtual key, its scan code and the UTF-16 unit it types.
#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub vk: u16,
    pub scan: u16,
    pub unit: u16,
}

#[cfg(windows)]
const ENTER: Key = Key { vk: 0x0D, scan: 0x1C, unit: b'\r' as u16 };
#[cfg(any(windows, test))]
const ESCAPE: Key = Key { vk: 0x1B, scan: 0x01, unit: 0x1B };
#[cfg(any(windows, test))]
const TAB: Key = Key { vk: 0x09, scan: 0x0F, unit: 0x09 };

/// What a delivery types: one line of text, or a single key ("escape", "tab" or one character).
#[cfg(any(windows, test))]
pub fn keys(mode: &str, payload: &str) -> Option<Vec<Key>> {
    let typed = |text: &str| text.encode_utf16().map(|unit| Key { vk: 0, scan: 0, unit }).collect::<Vec<_>>();
    match mode {
        "text" => Some(typed(&payload.replace(['\r', '\n'], " "))),
        "key" if payload == "escape" => Some(vec![ESCAPE]),
        "key" if payload == "tab" => Some(vec![TAB]),
        "key" if payload.chars().count() == 1 => Some(typed(payload)),
        _ => None,
    }
}

/// Every process on the console must be the helper, the agent, one of the agent's ancestors, or
/// the agent's own native program under it (`native`: Codex's npm wrapper runs its codex binary).
#[cfg(any(windows, test))]
pub fn others_on_console(attached: &[u32], helper: u32, agent: u32, ancestors: &[u32], native: impl Fn(u32) -> bool) -> Vec<u32> {
    attached.iter().copied().filter(|pid| *pid != helper && *pid != agent && !ancestors.contains(pid) && !native(*pid)).collect()
}

/// The exact file names of the program an agent runs natively under a wrapper (Codex's npm
/// package runs its platform binary), by the helper's `kind` argument. Nothing else may share the
/// console, not even another `codex-*` tool.
#[cfg(any(windows, test))]
pub fn native_programs(kind: &str) -> &'static [&'static str] {
    match kind {
        "codex" => &["codex.exe", "codex-x86_64-pc-windows-msvc.exe", "codex-aarch64-pc-windows-msvc.exe"],
        _ => &[],
    }
}

/// The helper's exit code when some of the text may have gone in: the engine reports the delivery
/// as uncertain, never as a failure that's safe to retry.
pub const UNCERTAIN_EXIT: i32 = 3;

/// How much of a write reached the console.
#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    All,
    Nothing,
    Part,
}

/// What a delivery amounts to: the text (or key), then Enter for text (`entered` is `None` for a
/// single key). Anything that may have put keys in the console without completing is uncertain.
#[cfg(any(windows, test))]
pub fn outcome(typed: Written, entered: Option<Written>) -> Result<()> {
    let uncertain = |message: &str| Err(crate::deliver::Uncertain(message.into()).into());
    match (typed, entered) {
        (Written::Nothing, _) => anyhow::bail!("couldn't type into the agent's console"),
        (Written::Part, _) => uncertain("only part of it went in; check the agent's terminal before sending again"),
        (Written::All, None | Some(Written::All)) => Ok(()),
        (Written::All, Some(_)) => uncertain("the text went in but Enter didn't"),
    }
}

/// The re-check before Enter: the text is already in, so a console that changed hands means the
/// delivery is unfinished (uncertain), and Enter isn't pressed.
#[cfg(any(windows, test))]
pub fn before_enter(check: Result<()>) -> Result<()> {
    check.map_err(|changed| {
        crate::deliver::Uncertain(format!("the text went in, but {changed:#} before Enter, so Lantern didn't press it")).into()
    })
}

/// The helper's result, from its exit: uncertain and failed stay distinct across the process.
pub fn from_exit(code: Option<i32>, stderr: &str) -> Result<()> {
    match code {
        Some(0) => Ok(()),
        Some(UNCERTAIN_EXIT) => Err(crate::deliver::Uncertain(stderr.trim().to_string()).into()),
        _ => anyhow::bail!("{}", if stderr.trim().is_empty() { "couldn't type into the agent's console" } else { stderr.trim() }),
    }
}

/// Engine side: types into the agent's console through a helper process (see the module comment).
pub fn send(agent: &crate::model::Agent, mode: &str, payload: &str) -> Result<()> {
    let exe = std::env::current_exe()?;
    let (pid, start) = (agent.pid.to_string(), agent.start_time.to_string());
    let kind = if agent.kind == crate::model::Kind::Codex { "codex" } else { "" };
    let out = crate::run::output(
        std::process::Command::new(exe).args(["console-type", &pid, &start, kind, mode, payload]),
        std::time::Duration::from_secs(5),
    );
    match out {
        Ok(out) => from_exit(out.status.code(), &String::from_utf8_lossy(&out.stderr)),
        // Killed while it may have been typing.
        Err(error) if error.downcast_ref::<crate::run::TimedOut>().is_some() => {
            Err(crate::deliver::Uncertain(format!("Lantern's typing helper {error}; check the agent's terminal")).into())
        }
        Err(error) => Err(error),
    }
}

/// The helper: `lantern-engine console-type <pid> <start> <agent kind> text|key <payload>`.
#[cfg(windows)]
pub fn type_into(pid: u32, start: u64, kind: &str, mode: &str, payload: &str) -> Result<()> {
    use anyhow::{Context, bail};
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::Console::{
        AttachConsole, FreeConsole, GetConsoleProcessList, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT, KEY_EVENT_RECORD, KEY_EVENT_RECORD_0,
        WriteConsoleInputW,
    };

    let keys = keys(mode, payload).context("nothing to type")?;
    let ancestors = crate::procs::ancestors_of(pid);
    let allowed = native_programs(kind);
    // The same agent, still on this console, with nothing else reading it. Checked before the text
    // and again before Enter: the agent may exit (back to its shell) or start a tool in between.
    let check = || -> Result<()> {
        let live = crate::procs::live(pid).context("the agent has exited")?;
        if live.start != start {
            bail!("the agent has exited and another process took its place");
        }
        let mut attached = vec![0u32; 64];
        let mut count = unsafe { GetConsoleProcessList(attached.as_mut_ptr(), attached.len() as u32) } as usize;
        if count > attached.len() {
            attached.resize(count + 16, 0);
            count = unsafe { GetConsoleProcessList(attached.as_mut_ptr(), attached.len() as u32) } as usize;
        }
        attached.truncate(count.min(attached.len()));
        if !attached.contains(&pid) {
            bail!("the agent is no longer on its console");
        }
        let native = |other: u32| {
            !allowed.is_empty()
                && crate::procs::ancestors_of(other).contains(&pid)
                && crate::procs::live(other)
                    .and_then(|l| l.exe)
                    .is_some_and(|exe| allowed.contains(&exe.rsplit(['\\', '/']).next().unwrap_or(&exe).to_lowercase().as_str()))
        };
        if !others_on_console(&attached, std::process::id(), pid, &ancestors, native).is_empty() {
            bail!("something else is running in the agent's terminal, so Lantern won't type there now");
        }
        Ok(())
    };
    crate::procs::live(pid).filter(|live| live.start == start).context("the agent has exited")?;
    unsafe {
        FreeConsole();
        if AttachConsole(pid) == 0 {
            bail!("couldn't reach the agent's console");
        }
    }
    let result = (|| -> Result<()> {
        check()?;
        let name: Vec<u16> = "CONIN$\0".encode_utf16().collect();
        let input = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if input == INVALID_HANDLE_VALUE {
            bail!("couldn't open the agent's console input");
        }
        let records = |keys: &[Key]| -> Vec<INPUT_RECORD> {
            keys.iter()
                .flat_map(|key| {
                    [true, false].map(|down| INPUT_RECORD {
                        EventType: KEY_EVENT as u16,
                        Event: INPUT_RECORD_0 {
                            KeyEvent: KEY_EVENT_RECORD {
                                bKeyDown: down as i32,
                                wRepeatCount: 1,
                                wVirtualKeyCode: key.vk,
                                wVirtualScanCode: key.scan,
                                uChar: KEY_EVENT_RECORD_0 { UnicodeChar: key.unit },
                                dwControlKeyState: 0,
                            },
                        },
                    })
                })
                .collect()
        };
        let write = |records: &[INPUT_RECORD]| -> Written {
            let mut written = 0u32;
            let ok = unsafe { WriteConsoleInputW(input, records.as_ptr(), records.len() as u32, &mut written) } != 0;
            match written as usize {
                n if ok && n == records.len() => Written::All,
                0 => Written::Nothing,
                _ => Written::Part,
            }
        };
        let typed = write(&records(&keys));
        // A paste, then Return as a key press of its own (see deliver::ENTER_AFTER_PASTE), only
        // if the console still belongs to the agent alone.
        let entered = (typed == Written::All && mode == "text").then(|| {
            std::thread::sleep(crate::deliver::ENTER_AFTER_PASTE);
            before_enter(check()).map(|()| write(&records(&[ENTER])))
        });
        unsafe { CloseHandle(input) };
        match entered {
            Some(Err(changed)) => Err(changed),
            Some(Ok(entered)) => outcome(typed, Some(entered)),
            None => outcome(typed, (mode == "text").then_some(Written::Nothing)),
        }
    })();
    unsafe { FreeConsole() };
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_is_one_line_of_keys_and_single_keys_are_exact() {
        let typed = keys("text", "fix it\nnow é").unwrap();
        let text: String = char::decode_utf16(typed.iter().map(|k| k.unit)).map(|c| c.unwrap()).collect();
        assert_eq!(text, "fix it now é", "newlines would submit early");
        assert!(typed.iter().all(|k| k.vk == 0));
        assert_eq!(keys("key", "escape"), Some(vec![ESCAPE]));
        assert_eq!(keys("key", "tab"), Some(vec![TAB]));
        assert_eq!(keys("key", "1"), Some(vec![Key { vk: 0, scan: 0, unit: b'1' as u16 }]));
        assert_eq!(keys("key", "12"), None);
        assert_eq!(keys("other", "x"), None);
        // A character outside the BMP is two units, as Windows expects.
        assert_eq!(keys("text", "🙂").unwrap().len(), 2);
    }

    #[test]
    fn only_the_agent_and_its_own_shell_may_share_the_console() {
        // helper 9, agent 5, started by shell 4 (in Windows Terminal 2).
        let none = |_| false;
        assert!(others_on_console(&[9, 5, 4], 9, 5, &[4, 2], none).is_empty());
        assert_eq!(others_on_console(&[9, 5, 4, 7], 9, 5, &[4, 2], none), vec![7], "a tool the agent runs");
        // Codex's wrapper (5) runs codex.exe (6); a tool (7) still blocks typing.
        let codex = |pid| pid == 6;
        assert!(others_on_console(&[9, 5, 6, 4], 9, 5, &[4, 2], codex).is_empty());
        assert_eq!(others_on_console(&[9, 5, 6, 7], 9, 5, &[4], codex), vec![7]);
        assert!(native_programs("codex").contains(&"codex-x86_64-pc-windows-msvc.exe"));
        assert!(!native_programs("codex").contains(&"codex-tool.exe"), "exact names only");
        assert!(native_programs("").is_empty());
    }

    #[test]
    fn enter_waits_past_the_paste_window_of_agent_tuis() {
        // Codex reads an Enter within ~120 ms of a pasted burst as a newline.
        assert!(crate::deliver::ENTER_AFTER_PASTE >= std::time::Duration::from_millis(300));
    }

    fn uncertain(result: Result<()>) -> bool {
        result.is_err_and(|e| e.downcast_ref::<crate::deliver::Uncertain>().is_some())
    }

    #[test]
    fn anything_that_may_have_typed_is_uncertain_never_a_plain_failure() {
        assert!(outcome(Written::All, Some(Written::All)).is_ok());
        assert!(outcome(Written::All, None).is_ok(), "a single key");
        let nothing = outcome(Written::Nothing, None);
        assert!(nothing.is_err() && !uncertain(nothing), "nothing typed: safe to retry");
        assert!(uncertain(outcome(Written::Part, None)));
        assert!(uncertain(outcome(Written::Part, Some(Written::All))));
        assert!(uncertain(outcome(Written::All, Some(Written::Nothing))), "text in, no Enter");
        assert!(uncertain(outcome(Written::All, Some(Written::Part))));
        // The agent left, or a tool started, during the pause: no Enter, and it's uncertain.
        assert!(before_enter(Ok(())).is_ok());
        let changed = before_enter(Err(anyhow::anyhow!("the agent is no longer on its console")));
        assert!(changed.as_ref().is_err_and(|e| e.to_string().contains("didn't press it")) && uncertain(changed));
        // ...and that survives the helper's exit.
        assert!(from_exit(Some(0), "").is_ok());
        assert!(uncertain(from_exit(Some(UNCERTAIN_EXIT), "the text went in but Enter didn't")));
        let failed = from_exit(Some(1), "the agent is no longer on its console");
        assert!(failed.as_ref().is_err_and(|e| e.to_string() == "the agent is no longer on its console") && !uncertain(failed));
        assert!(!uncertain(from_exit(None, "")));
    }
}
