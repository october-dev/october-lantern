//! Running helper programs (tmux, osascript, lsof, sqlite3...) with a time limit.
//!
//! The helper runs in a process group of its own, and its output is read while it runs, so a
//! chatty helper can't fill its pipe and stall. The limit covers the whole run, including output
//! held open by anything the helper started: when time is up, the whole group is killed.

use std::io::Read;
use std::process::{Command, ExitStatus, Output, Stdio};
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
    use std::os::unix::process::CommandExt;
    let deadline = Instant::now() + limit;
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .with_context(|| format!("starting {}", cmd.get_program().to_string_lossy()))?;
    let group = child.id() as i32;
    let kill_group = || unsafe {
        libc::kill(-group, libc::SIGKILL);
    };

    // Read both pipes as they fill; each reader says when its pipe closed.
    let (closed, pipes_done) = mpsc::channel();
    let buffers = [Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new()))];
    let pipes: [Option<Box<dyn Read + Send>>; 2] =
        [child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>)];
    for (pipe, buffer) in pipes.into_iter().zip(buffers.iter().cloned()) {
        let closed = closed.clone();
        std::thread::spawn(move || {
            if let Some(mut pipe) = pipe {
                let mut chunk = [0u8; 16 * 1024];
                while let Ok(n) = pipe.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    buffer.lock().unwrap().extend_from_slice(&chunk[..n]);
                }
            }
            let _ = closed.send(());
        });
    }

    let status: Option<ExitStatus> = loop {
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
            break;
        }
    }
    let take = |i: usize| std::mem::take(&mut *buffers[i].lock().unwrap());
    Ok(Output { status, stdout: take(0), stderr: take(1) })
}
