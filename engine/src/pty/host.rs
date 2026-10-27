//! `lantern-engine pty-host <name>`: one agent in a pseudo console, served over a named pipe.
//! Started (detached) by `start`; it ends when the agent does.
use super::*;
use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, CreatePipe, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, INFINITE, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute,
    WaitForSingleObject,
};

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// Starts a host for `spec` and waits until it has started the agent.
pub fn start(name: &str, spec: &Spec) -> Result<Session> {
    if !valid_name(name) {
        bail!("invalid session name");
    }
    std::fs::create_dir_all(dir())?;
    let error_file = dir().join(format!("{name}.error.txt"));
    let _ = std::fs::remove_file(&error_file);
    crate::hooks::write_atomic(&dir().join(format!("{name}.spec.json")), &serde_json::to_vec(spec)?)?;
    let engine = crate::hooks::engine_copy()?;
    let spawn = |flags: u32| {
        Command::new(&engine)
            .args(["pty-host", name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags)
            .spawn()
    };
    // Out of any job the engine is in, so closing the app doesn't end the session; some jobs
    // don't allow that, and then it stays in.
    let base = CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;
    let mut host = spawn(base | CREATE_BREAKAWAY_FROM_JOB).or_else(|_| spawn(base)).context("starting the session")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(session) = sessions().into_iter().find(|s| s.name == name) {
            std::thread::spawn(move || host.wait());
            return Ok(session);
        }
        if host.try_wait()?.is_some() || Instant::now() >= deadline {
            let _ = host.kill();
            let why = std::fs::read_to_string(&error_file).unwrap_or_else(|_| "the session didn't start".into());
            let _ = std::fs::remove_file(&error_file);
            bail!("{}", why.trim());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct Shared {
    control: Mutex<()>,
    input: Mutex<File>,
    screen: Mutex<vt100::Parser>,
    /// Attached windows' output streams, and the processes behind them.
    streams: Mutex<Vec<(u32, mpsc::Sender<Vec<u8>>)>>,
    console: HPCON,
    child: HANDLE,
    child_pid: u32,
    child_start: u64,
}

// The handles are only used through thread-safe Win32 calls.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

/// The host's whole life. Failing to start the agent is written where `start` reads it.
pub fn host(name: &str) -> Result<()> {
    if !valid_name(name) {
        bail!("invalid session name");
    }
    let result = run(name);
    if let Err(e) = &result {
        let _ = std::fs::write(dir().join(format!("{name}.error.txt")), format!("{e:#}"));
    }
    result
}

fn run(name: &str) -> Result<()> {
    let spec_path = dir().join(format!("{name}.spec.json"));
    let spec: Spec = serde_json::from_slice(&std::fs::read(&spec_path).context("the session's description is missing")?)?;
    let _ = std::fs::remove_file(&spec_path);

    // Claim the pipe name first: a second host for the same name stops here.
    let pipe_name = wide(pipe_name(name));
    let first = listen(&pipe_name, true).context("another session already has this name")?;

    let (console, input, output, child, child_pid) = spawn(&spec)?;
    let child_start = crate::procs::live(child_pid).map_or(0, |l| l.start);
    let shared = Arc::new(Shared {
        control: Mutex::new(()),
        input: Mutex::new(input),
        screen: Mutex::new(vt100::Parser::new(ROWS, COLS, 0)),
        streams: Mutex::new(Vec::new()),
        console,
        child,
        child_pid,
        child_start,
    });

    let record = dir().join(format!("{name}.json"));
    let me = crate::procs::live(std::process::id()).map_or(0, |l| l.start);
    let session = Session { name: name.into(), host_pid: std::process::id(), host_start: me, child_pid, child_start };
    crate::hooks::write_atomic(&record, &serde_json::to_vec(&session)?)?;

    let reader = {
        let shared = shared.clone();
        std::thread::spawn(move || read_output(output, &shared))
    };
    {
        let shared = shared.clone();
        // A handle is a pointer; it crosses to the thread as a number.
        let first = first as usize;
        std::thread::spawn(move || serve(first as HANDLE, &pipe_name, &shared));
    }

    unsafe { WaitForSingleObject(child, INFINITE) };
    let _control = shared.control.lock().unwrap();
    // Closing the console ends its output; the reader finishes what's left and stops.
    unsafe { ClosePseudoConsole(console) };
    let deadline = Instant::now() + Duration::from_secs(3);
    while !reader.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    crate::hooks::write_atomic(&dir().join(format!("{name}.ended")), b"ended")?;
    let _ = std::fs::remove_file(&record);
    std::process::exit(0)
}

/// Creates the pseudo console and starts the agent in it, with no handles of ours.
fn spawn(spec: &Spec) -> Result<(HPCON, File, File, HANDLE, u32)> {
    unsafe {
        let (mut in_read, mut in_write, mut out_read, mut out_write): (HANDLE, HANDLE, HANDLE, HANDLE) =
            (std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        if CreatePipe(&mut in_read, &mut in_write, std::ptr::null(), 0) == 0
            || CreatePipe(&mut out_read, &mut out_write, std::ptr::null(), 0) == 0
        {
            bail!("couldn't create the session's pipes");
        }
        let mut console: HPCON = 0;
        let size = COORD { X: COLS as i16, Y: ROWS as i16 };
        if CreatePseudoConsole(size, in_read, out_write, 0, &mut console) < 0 {
            bail!("Windows couldn't create a pseudo console (it needs Windows 10 1809 or later)");
        }

        let mut bytes = 0usize;
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes);
        let mut list = vec![0u8; bytes];
        let attributes = list.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        if InitializeProcThreadAttributeList(attributes, 1, 0, &mut bytes) == 0
            || UpdateProcThreadAttribute(
                attributes,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                console as *const std::ffi::c_void,
                std::mem::size_of::<HPCON>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ) == 0
        {
            bail!("couldn't attach the pseudo console");
        }
        let mut startup: STARTUPINFOEXW = std::mem::zeroed();
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        // Explicitly no standard handles, or the agent could get ours instead of the console.
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
        startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
        startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
        startup.lpAttributeList = attributes;

        let application = wide(&spec.program);
        let mut command = wide(command_line(&spec.program, &spec.args));
        let environment = environment_block(&spec.env);
        let cwd = wide(&spec.cwd);
        let mut process: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut process,
        );
        DeleteProcThreadAttributeList(attributes);
        // The console keeps its own copies.
        CloseHandle(in_read);
        CloseHandle(out_write);
        if ok == 0 {
            ClosePseudoConsole(console);
            bail!("couldn't start {}: {}", spec.program.display(), std::io::Error::last_os_error());
        }
        CloseHandle(process.hThread);
        Ok((console, File::from_raw_handle(in_write as _), File::from_raw_handle(out_read as _), process.hProcess, process.dwProcessId))
    }
}

/// Our environment with the spec's variables on top, as Windows wants it: `KEY=value` strings,
/// sorted, each ended by a NUL, then one more NUL.
fn environment_block(extra: &[(String, String)]) -> Vec<u16> {
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    for (key, value) in extra {
        vars.retain(|(k, _)| !k.to_string_lossy().eq_ignore_ascii_case(key));
        vars.push((key.into(), value.into()));
    }
    vars.sort_by_key(|(k, _)| k.to_string_lossy().to_ascii_uppercase());
    let mut block = Vec::new();
    for (key, value) in vars {
        block.extend(key.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

fn read_output(mut output: File, shared: &Shared) {
    let mut chunk = [0u8; 16 * 1024];
    while let Ok(n) = output.read(&mut chunk) {
        if n == 0 {
            break;
        }
        // Same lock order as `stream_output`: screen, then streams.
        let mut screen = shared.screen.lock().unwrap();
        screen.process(&chunk[..n]);
        shared.streams.lock().unwrap().retain(|(_, tx)| tx.send(chunk[..n].to_vec()).is_ok());
    }
    // Attached windows see their stream end.
    shared.streams.lock().unwrap().clear();
}

/// One pipe instance for the next connection.
fn listen(name: &[u16], first: bool) -> Result<HANDLE> {
    let flags = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
    let pipe = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            flags,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            std::ptr::null(),
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        bail!("{}", std::io::Error::last_os_error());
    }
    Ok(pipe)
}

fn serve(first: HANDLE, name: &[u16], shared: &Arc<Shared>) {
    let mut next = Some(first);
    loop {
        let pipe = match next.take() {
            Some(pipe) => pipe,
            None => match listen(name, false) {
                Ok(pipe) => pipe,
                Err(_) => return,
            },
        };
        let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        let mut client = 0u32;
        unsafe { GetNamedPipeClientProcessId(pipe, &mut client) };
        let file = unsafe { File::from_raw_handle(pipe as _) };
        if connected {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let _ = handle(file, client, &shared);
            });
        }
    }
}

fn reply(mut pipe: &File, value: Value) -> Result<()> {
    writeln!(pipe, "{value}")?;
    Ok(())
}

fn handle(file: File, client: u32, shared: &Shared) -> Result<()> {
    crate::platform::require_same_user(client)?;
    let mut reader = BufReader::new(&file);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let request: Value = serde_json::from_str(&line).unwrap_or_default();
    let write_input = |bytes: &[u8]| -> Result<()> {
        let mut input = shared.input.lock().unwrap();
        input.write_all(bytes)?;
        input.flush()?;
        Ok(())
    };
    match request["op"].as_str().unwrap_or("") {
        "stop" => {
            let _control = shared.control.lock().unwrap();
            unsafe {
                if windows_sys::Win32::System::Threading::TerminateProcess(shared.child, 0) == 0 {
                    bail!("{}", std::io::Error::last_os_error());
                }
                if WaitForSingleObject(shared.child, 5000) != 0 {
                    bail!("The child has not confirmed exiting");
                }
            }
            reply(&file, json!({"ok":true}))
        }
        "info" => {
            let alive = unsafe { WaitForSingleObject(shared.child, 0) } == WAIT_TIMEOUT;
            let tty = crate::procs::live(shared.child_pid).and_then(|l| l.tty);
            reply(
                &file,
                json!({"ok": true, "childPid": shared.child_pid, "childStart": shared.child_start, "childTty": tty, "alive": alive}),
            )
        }
        "text" => {
            // One line: a newline would submit early.
            let text = request["text"].as_str().unwrap_or_default().replace(['\r', '\n'], " ");
            write_input(text.as_bytes())?;
            reply(&file, json!({"ok": true}))
        }
        "key" => {
            write_input(request["bytes"].as_str().unwrap_or_default().as_bytes())?;
            reply(&file, json!({"ok": true}))
        }
        "capture" => {
            let screen = shared.screen.lock().unwrap().screen().contents();
            reply(&file, json!({"ok": true, "screen": screen}))
        }
        "resize" => {
            let cols = request["cols"].as_u64().unwrap_or(COLS as u64).clamp(20, 500) as u16;
            let rows = request["rows"].as_u64().unwrap_or(ROWS as u64).clamp(5, 300) as u16;
            unsafe { ResizePseudoConsole(shared.console, COORD { X: cols as i16, Y: rows as i16 }) };
            shared.screen.lock().unwrap().screen_mut().set_size(rows, cols);
            reply(&file, json!({"ok": true}))
        }
        "clients" => {
            let pids: Vec<u32> = shared.streams.lock().unwrap().iter().map(|(pid, _)| *pid).collect();
            reply(&file, json!({"ok": true, "pids": pids}))
        }
        "output" => stream_output(&file, client, shared),
        "input" => {
            reply(&file, json!({"ok": true}))?;
            let mut chunk = [0u8; 4096];
            loop {
                let n = reader.read(&mut chunk)?;
                if n == 0 {
                    return Ok(());
                }
                write_input(&chunk[..n])?;
            }
        }
        _ => reply(&file, json!({"ok": false, "message": "unknown request"})),
    }
}

/// The screen as it is now, then everything the agent prints, until the window goes away.
fn stream_output(mut pipe: &File, client: u32, shared: &Shared) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let screen = {
        let screen = shared.screen.lock().unwrap();
        let formatted = screen.screen().contents_formatted();
        shared.streams.lock().unwrap().push((client, tx));
        formatted
    };
    writeln!(pipe, "{}", json!({"ok": true}))?;
    pipe.write_all(&screen)?;
    for bytes in rx {
        if pipe.write_all(&bytes).is_err() {
            break;
        }
    }
    Ok(())
}
