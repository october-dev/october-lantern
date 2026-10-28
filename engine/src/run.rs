//! Running helper programs (tmux, osascript, lsof, sqlite3...) with a time limit.
//!
//! The helper runs in a process group of its own (a job object on Windows), and its output is read
//! while it runs, so a chatty helper can't fill its pipe and stall. The limit covers the whole run,
//! including output held open by anything the helper started: when time is up, the whole group is
//! killed.

use std::io::Read;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// The helper didn't finish within its limit (and was killed).
#[derive(Debug)]
pub struct TimedOut(pub Duration);

impl std::fmt::Display for TimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gave up after {:.1} s", self.0.as_secs_f32())
    }
}

impl std::error::Error for TimedOut {}

pub fn output(cmd: &mut Command, limit: Duration) -> Result<Output> {
    collect(cmd, limit, None)
}

/// Credential-bearing probes must fail on excessive output or incomplete pipe closure, never
/// accept a truncated result. Their captured bytes are not included in errors.
#[cfg(unix)]
pub fn output_capped(cmd: &mut Command, limit: Duration, max_bytes: usize) -> Result<Output> {
    collect(cmd, limit, Some(max_bytes))
}

fn collect(cmd: &mut Command, limit: Duration, max_bytes: Option<usize>) -> Result<Output> {
    let deadline = Instant::now() + limit;
    let mut child = Group::prepare(cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()), max_bytes.is_some())
        .spawn()
        .with_context(|| format!("starting {}", cmd.get_program().to_string_lossy()))?;
    let group = Group::of(&child);
    let kill_group = || group.kill();

    // Read both pipes as they fill; each reader says when its pipe closed.
    let (closed, pipes_done) = mpsc::channel();
    let buffers = [Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new()))];
    let bytes = Arc::new(AtomicUsize::new(0));
    let overflow = Arc::new(AtomicBool::new(false));
    let read_failed = Arc::new(AtomicBool::new(false));
    let pipes: [Option<Box<dyn Read + Send>>; 2] = [
        child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
        child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
    ];
    for (pipe, buffer) in pipes.into_iter().zip(buffers.iter().cloned()) {
        let closed = closed.clone();
        let bytes = bytes.clone();
        let overflow = overflow.clone();
        let read_failed = read_failed.clone();
        std::thread::spawn(move || {
            if let Some(mut pipe) = pipe {
                let mut chunk = [0u8; 16 * 1024];
                loop {
                    let n = match pipe.read(&mut chunk) {
                        Ok(n) => n,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => {
                            read_failed.store(true, Ordering::Release);
                            break;
                        }
                    };
                    if n == 0 {
                        break;
                    }
                    if let Some(max) = max_bytes {
                        let used = bytes.fetch_add(n, Ordering::Relaxed);
                        if n > max.saturating_sub(used) {
                            overflow.store(true, Ordering::Release);
                            break;
                        }
                    }
                    buffer.lock().unwrap().extend_from_slice(&chunk[..n]);
                }
            }
            let _ = closed.send(());
        });
    }

    let status: Option<ExitStatus> = loop {
        if overflow.load(Ordering::Acquire) {
            kill_group();
            let _ = child.wait();
            anyhow::bail!("helper exceeded its output limit");
        }
        if let Some(s) = child.try_wait()? {
            break Some(s);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let Some(status) = status else {
        kill_group();
        let _ = child.wait();
        return Err(TimedOut(limit).into());
    };
    // Something the helper started may still hold its output open: wait for that only until
    // the deadline, then end the group.
    for _ in 0..2 {
        if pipes_done.recv_timeout(deadline.saturating_duration_since(Instant::now())).is_err() {
            kill_group();
            if max_bytes.is_some() {
                return Err(TimedOut(limit).into());
            }
            break;
        }
    }
    if max_bytes.is_some() {
        // Startup files may leave a background process even after closing both output pipes.
        kill_group();
    }
    if overflow.load(Ordering::Acquire) {
        anyhow::bail!("helper exceeded its output limit");
    }
    if max_bytes.is_some() && read_failed.load(Ordering::Acquire) {
        anyhow::bail!("helper output could not be read completely");
    }
    let take = |i: usize| std::mem::take(&mut *buffers[i].lock().unwrap());
    Ok(Output { status, stdout: take(0), stderr: take(1) })
}

/// The helper and everything it starts.
#[cfg(unix)]
struct Group(i32);

#[cfg(unix)]
impl Group {
    fn prepare(cmd: &mut Command, new_session: bool) -> &mut Command {
        use std::os::unix::process::CommandExt;
        if new_session {
            // A null stdin alone does not prevent rc files reading the parent's /dev/tty.
            // SAFETY: setsid is async-signal-safe; no Rust state is accessed after fork.
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                })
            }
        } else {
            cmd.process_group(0)
        }
    }

    fn of(child: &std::process::Child) -> Self {
        Group(child.id() as i32)
    }

    fn kill(&self) {
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}

/// A job object holding the helper; what it starts joins the job too. (Anything it starts in
/// the instant before it joins is missed.)
#[cfg(windows)]
struct Group(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Group {
    fn prepare(cmd: &mut Command, _new_session: bool) -> &mut Command {
        use std::os::windows::process::CommandExt;
        // No console window flashing up for each helper.
        cmd.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
    }

    fn of(child: &std::process::Child) -> Self {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if !job.is_null() {
                AssignProcessToJobObject(job, child.as_raw_handle() as _);
            }
            Group(job)
        }
    }

    fn kill(&self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for Group {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}
