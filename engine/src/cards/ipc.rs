//! The private connection between launches' MCP servers and the engine: a Unix socket in
//! Lantern's private cards folder (Mac, Linux), or a named pipe only this user can open
//! (Windows). One request per connection, as one line of JSON (`{"launch", "key", "op", ...}`),
//! answered with one line (`{"ok": true, ...}` or `{"ok": false, "message"}`).

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Cards, Op};

/// The longest request line accepted.
const MAX_LINE: u64 = 64 * 1024;
/// Connections handled at once; more are turned away.
const MAX_CONNECTIONS: usize = 32;
/// How long a connection may take to send its request.
#[cfg(unix)]
const READ_LIMIT: Duration = Duration::from_secs(5);

#[derive(Deserialize)]
struct Request {
    launch: String,
    key: String,
    #[serde(flatten)]
    op: Op,
}

/// Reads one request line, at most `MAX_LINE` long.
fn read_request(stream: impl Read) -> Result<Request> {
    let mut line = String::new();
    BufReader::new(stream.take(MAX_LINE + 1)).read_line(&mut line)?;
    if line.len() as u64 > MAX_LINE || !line.ends_with('\n') {
        bail!("request too long or cut off");
    }
    Ok(serde_json::from_str(&line)?)
}

pub(super) fn answer(cards: &Cards, stream: impl Read) -> Value {
    match read_request(stream).and_then(|r| cards.handle(&r.launch, &r.key, r.op)) {
        Ok(mut v) => {
            v["ok"] = json!(true);
            v
        }
        Err(e) => json!({"ok": false, "message": format!("{e:#}")}),
    }
}

static OPEN: AtomicUsize = AtomicUsize::new(0);

/// Runs `serve` on its own thread, unless too many connections are open already.
fn spawn(serve: impl FnOnce() + Send + 'static) {
    if OPEN.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
        OPEN.fetch_sub(1, Ordering::SeqCst);
        return;
    }
    std::thread::spawn(move || {
        serve();
        OPEN.fetch_sub(1, Ordering::SeqCst);
    });
}

/// Where the engine listens: in Lantern's private cards folder, or, when that path is too long
/// for a socket (about 100 bytes), in a folder of this user's own under /tmp.
#[cfg(unix)]
pub fn socket() -> std::path::PathBuf {
    let path = super::dir().join("engine.sock");
    if path.as_os_str().len() < 100 {
        return path;
    }
    let uid = unsafe { libc::getuid() };
    std::path::PathBuf::from(format!("/tmp/october-lantern-{uid}/cards.sock"))
}

/// The socket's folder, made private to this user, and refused if anyone else owns it or can
/// read it.
#[cfg(unix)]
fn private_folder(socket: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let dir = socket.parent().context("no folder")?;
    if !dir.exists() {
        crate::bus::create_private_dir(dir)?;
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::getuid() } || !crate::platform::is_private(&meta) {
        bail!("{} isn't private to this user", dir.display());
    }
    Ok(())
}

/// Starts accepting connections. Fails when another engine is already listening.
#[cfg(unix)]
pub fn listen(cards: Arc<Cards>) -> Result<()> {
    use std::os::unix::net::UnixListener;
    let dir = super::dir();
    if !crate::platform::is_private(&std::fs::metadata(&dir)?) {
        bail!("{} can be read by others", dir.display());
    }
    // The lock is held for the life of the engine: one engine on the computer serves cards.
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("engine.lock"))?;
    if lock.try_lock().is_err() {
        bail!("another Lantern engine is serving cards");
    }
    std::mem::forget(lock);
    let path = socket();
    private_folder(&path)?;
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("listening at {}", path.display()))?;
    crate::platform::set_mode(&path, 0o600)?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let cards = cards.clone();
            spawn(move || {
                let _ = stream.set_read_timeout(Some(READ_LIMIT));
                let reply = answer(&cards, &stream);
                let _ = writeln!(&stream, "{reply}");
            });
        }
    });
    Ok(())
}

/// One request to the engine and its answer.
#[cfg(unix)]
pub fn request(request: &Value) -> Result<Value> {
    let stream = std::os::unix::net::UnixStream::connect(socket()).context("Lantern isn't running")?;
    exchange(stream, request)
}

pub(super) fn exchange<S: Read + Write>(mut stream: S, request: &Value) -> Result<Value> {
    writeln!(stream, "{request}")?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    serde_json::from_str(&line).context("Lantern didn't answer")
}

#[cfg(windows)]
fn pipe_name() -> String {
    use sha2::{Digest, Sha256};
    let profile = super::dir();
    let digest = Sha256::digest(profile.to_string_lossy().to_lowercase().as_bytes());
    format!(r"\\.\pipe\lantern-cards-{}", digest.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// A security descriptor that lets only this user (and the system) open the pipe: the string form
/// `D:P(A;;GA;;;<user's SID>)(A;;GA;;;SY)`.
#[cfg(windows)]
fn owner_only() -> Result<windows_sys::Win32::Security::PSECURITY_DESCRIPTOR> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            bail!("{}", std::io::Error::last_os_error());
        }
        let mut buffer = vec![0u8; 256];
        let mut size = 0u32;
        let ok = GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), buffer.len() as u32, &mut size);
        CloseHandle(token);
        if ok == 0 {
            bail!("{}", std::io::Error::last_os_error());
        }
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut sid: windows_sys::core::PWSTR = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            bail!("{}", std::io::Error::last_os_error());
        }
        let len = (0..).take_while(|&i| *sid.add(i) != 0).count();
        let sid_text = String::from_utf16_lossy(std::slice::from_raw_parts(sid, len));
        LocalFree(sid.cast());
        let sddl = crate::platform::wide(format!("D:P(A;;GA;;;{sid_text})(A;;GA;;;SY)"));
        let mut descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, std::ptr::null_mut()) == 0
        {
            bail!("{}", std::io::Error::last_os_error());
        }
        Ok(descriptor)
    }
}

/// Starts accepting connections. Fails when another engine already serves the pipe.
#[cfg(windows)]
pub fn listen(cards: Arc<Cards>) -> Result<()> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
        PIPE_WAIT,
    };
    let descriptor = owner_only()? as usize;
    let name = crate::platform::wide(pipe_name());
    let create = move |first: bool| {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor as _,
            bInheritHandle: 0,
        };
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
                &attributes,
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            return Err(anyhow::anyhow!("{}", std::io::Error::last_os_error()));
        }
        Ok(pipe as usize)
    };
    // The first instance fails if another engine already has the pipe.
    let first = create(true).context("another Lantern engine is serving cards")?;
    std::thread::spawn(move || {
        let mut next = Some(first);
        loop {
            let pipe = match next.take() {
                Some(p) => p,
                None => match create(false) {
                    Ok(p) => p,
                    Err(_) => return,
                },
            };
            let handle = pipe as windows_sys::Win32::Foundation::HANDLE;
            let connected =
                unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            let file = unsafe { std::fs::File::from_raw_handle(handle as _) };
            if !connected {
                continue;
            }
            let cards = cards.clone();
            spawn(move || {
                // A named pipe has no read timeout; a client that never sends holds one thread
                // until it disconnects, within MAX_CONNECTIONS.
                let reply = answer(&cards, &file);
                let _ = writeln!(&file, "{reply}");
                let _ = (&file).flush();
            });
        }
    });
    Ok(())
}

/// One request to the engine and its answer.
#[cfg(windows)]
pub fn request(request: &Value) -> Result<Value> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(pipe_name()) {
            Ok(pipe) => {
                crate::platform::verify_pipe_server(&pipe)?;
                return exchange(pipe, request);
            }
            // ERROR_PIPE_BUSY: every instance is taken for a moment.
            Err(e) if e.raw_os_error() == Some(231) && std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(e) => return Err(e).context("Lantern isn't running"),
        }
    }
}
