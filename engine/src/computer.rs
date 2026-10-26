//! Using the computer's apps: Cua Driver (github.com/trycua/cua, MIT) for the agents Lantern
//! starts. With it an agent can see and work any app's windows (click, type, scroll, menus,
//! screenshots, the user's signed-in browser) in the background, without taking over the mouse,
//! with its own cursor on screen.
//!
//! The `cua-driver` program isn't in the app: it's downloaded the first time it's needed (the
//! pinned release in `engine/cua.json`, checked against its SHA-256) into Lantern's support folder.
//! The engine runs one driver of Lantern's own on a private socket (`cua-driver serve`) and stops
//! it when the engine exits. On macOS it runs embedded: as the engine's child it is inside
//! Lantern's app, so it uses the Accessibility and Screen Recording access Lantern already has and
//! never asks for its own. Its telemetry is off (`DO_NOT_TRACK`).
//!
//! Agents get it through MCP (`cua-driver mcp --socket …`, added per launch beside October Bus,
//! see `bus::attach`), or, for agents with no per-launch MCP, the `cua-driver call --socket …`
//! command in their first message.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::hooks::support_dir;

struct Pin {
    version: String,
    url: String,
    sha256: String,
}

fn manifest() -> Option<serde_json::Value> {
    serde_json::from_str(include_str!("../cua.json")).ok()
}

/// The pinned release for this system and chip.
fn pin() -> Option<Pin> {
    let v = manifest()?;
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    let key = if cfg!(target_os = "macos") {
        "macos".to_string()
    } else if cfg!(windows) {
        format!("windows-{arch}")
    } else {
        format!("linux-{arch}")
    };
    let t = &v["targets"][key.as_str()];
    Some(Pin { version: v["version"].as_str()?.into(), url: t["url"].as_str()?.into(), sha256: t["sha256"].as_str()?.into() })
}

/// The pinned version, or "" when this system has no build.
pub fn version() -> String {
    pin().map(|p| p.version).unwrap_or_default()
}

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// Where the pinned version lives, named by its version.
fn install_dir() -> Option<PathBuf> {
    Some(support_dir().join("cua-driver").join(pin()?.version))
}

/// The `cua-driver` program, once downloaded (or `LANTERN_CUA_BIN`, for development).
pub fn binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LANTERN_CUA_BIN").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    install_dir().map(|d| d.join(exe("cua-driver"))).filter(|p| p.is_file())
}

/// Downloads the pinned driver if it isn't here yet and checks it before keeping it.
pub fn ensure_installed() -> Result<PathBuf> {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap();
    if let Some(p) = binary() {
        return Ok(p);
    }
    let pin = pin().context("Cua Driver has no build for this system")?;
    let dest = install_dir().context("no install path")?;
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600))).build().into();
    let mut resp = agent.get(&pin.url).call().context("downloading Cua Driver")?;
    let bytes = resp.body_mut().with_config().limit(256 * 1024 * 1024).read_to_vec().context("downloading Cua Driver")?;
    use sha2::Digest;
    let sum: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if sum != pin.sha256 {
        bail!("the Cua Driver download didn't match its checksum, so it wasn't used");
    }
    let parent = dest.parent().context("no folder")?;
    std::fs::create_dir_all(parent)?;
    let work = parent.join(format!(".download-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&work)?;
    let result = (|| -> Result<()> {
        let archive = work.join(if pin.url.ends_with(".zip") { "driver.zip" } else { "driver.tar.gz" });
        std::fs::write(&archive, &bytes)?;
        // macOS's and Windows' own tar (bsdtar) read both; GNU tar reads Linux's .tar.gz.
        let tar = if cfg!(target_os = "macos") { "/usr/bin/tar" } else { "tar" };
        let out = crate::run::output(Command::new(tar).arg("-xf").arg(&archive).arg("-C").arg(&work), Duration::from_secs(120))?;
        if !out.status.success() {
            bail!("couldn't unpack Cua Driver");
        }
        // The programs only: the SDK libraries in the archive are for apps that link it.
        let keep = work.join("keep");
        std::fs::create_dir_all(&keep)?;
        for name in ["cua-driver", "cua-driver-uia", "cua-cursor-theme"] {
            let from = work.join(exe(name));
            if from.is_file() {
                crate::platform::set_mode(&from, 0o755)?;
                std::fs::rename(&from, keep.join(exe(name)))?;
            }
        }
        if !keep.join(exe("cua-driver")).is_file() {
            bail!("the Cua Driver download has no cua-driver program");
        }
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&keep, &dest)?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    result?;
    // Earlier pinned versions are no longer used.
    for old in std::fs::read_dir(parent).into_iter().flatten().flatten() {
        if old.path() != dest {
            let _ = std::fs::remove_dir_all(old.path());
        }
    }
    let _ = driver(&dest.join(exe("cua-driver"))).args(["telemetry", "disable"]).output();
    Ok(dest.join(exe("cua-driver")))
}

/// Lantern's own daemon endpoint: a socket in the support folder, or a named pipe per user on
/// Windows.
pub fn socket() -> String {
    if cfg!(windows) {
        let user: String = std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric()).take(32).collect();
        format!(r"\\.\pipe\october-lantern-cua-{user}")
    } else {
        support_dir().join("cua-driver.sock").to_string_lossy().into_owned()
    }
}

/// A `cua-driver` command with Lantern's settings: telemetry off, and embedded in Lantern's app
/// on macOS.
fn driver(bin: &PathBuf) -> Command {
    let mut cmd = Command::new(bin);
    cmd.env("DO_NOT_TRACK", "1");
    if cfg!(target_os = "macos") {
        cmd.env("CUA_DRIVER_EMBEDDED", "1");
    }
    cmd
}

/// The MCP server an agent starts to reach Lantern's driver: (command, arguments).
pub fn mcp_server() -> Option<(String, Vec<String>)> {
    let bin = binary()?.to_string_lossy().into_owned();
    let mut args = vec!["mcp".to_string()];
    if cfg!(target_os = "macos") {
        args.push("--embedded".into());
    }
    args.extend(["--socket".to_string(), socket()]);
    Some((bin, args))
}

/// The shell command that runs one Cua Driver tool against Lantern's driver.
pub fn call_command() -> Option<String> {
    let bin = binary()?.to_string_lossy().into_owned();
    Some(if cfg!(windows) {
        format!("\"{bin}\" call --socket \"{}\"", socket())
    } else {
        format!("{} call --socket {}", crate::launch::quote(&bin), crate::launch::quote(&socket()))
    })
}

/// What an agent's first message says about using the computer's apps.
pub fn note(has_tools: bool) -> Option<String> {
    let how = if has_tools {
        "You have the cua-driver tools for this.".to_string()
    } else {
        let call = call_command()?;
        format!(
            "Run its tools from the shell: `{call} <tool> '<json>'` (for example `{call} list_apps`; `{call} get_window_state \
             '{{\"pid\":123,\"window_id\":456}}' --screenshot-out-file /tmp/w.png` saves a screenshot)."
        )
    };
    Some(format!(
        "You can also use the apps on this computer through Cua Driver, in the background, without taking over the mouse. {how} \
         Prefer files, the terminal and APIs when they can do the job; use the apps when the task needs them (a website in the \
         user's browser, a form, a desktop app). After each action, look at the window again to check what happened."
    ))
}

/// The driver this engine started. Holding `_owner` (an OS lock on `cua-driver.lock`) is what makes
/// it this engine's: one engine on the computer owns the driver at a time, and the lock goes with
/// the process, even if it crashes.
struct Daemon {
    child: Child,
    _owner: std::fs::File,
}

static DAEMON: Mutex<Option<Daemon>> = Mutex::new(None);

fn running(bin: &PathBuf) -> bool {
    let mut cmd = driver(bin);
    cmd.args(["status", "--socket", &socket()]);
    crate::run::output(&mut cmd, Duration::from_secs(5)).is_ok_and(|o| o.status.success())
}

/// This engine's driver is running and reachable. A driver another engine owns doesn't count: its
/// agents aren't given it, and this engine can't turn it off or restart it.
pub fn ready() -> bool {
    let ours = DAEMON.lock().unwrap().as_mut().is_some_and(|d| d.child.try_wait().ok().flatten().is_none());
    ours && binary().is_some_and(|b| running(&b))
}

/// Takes the computer-wide driver lock at `path`; `None` while another process holds it.
fn claim(path: &std::path::Path) -> Result<Option<std::fs::File>> {
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).context("opening Cua Driver's lock")?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e).context("locking Cua Driver"),
    }
}

/// What was last asked of the driver (on or off, from Settings) and how many requests there have
/// been. Only the latest request may spawn the driver or report it ready: turning it off while an
/// earlier start is still downloading, waiting for the lock or starting cancels that start instead
/// of letting it finish afterwards.
struct Wanted {
    on: bool,
    generation: u64,
}

static WANTED: Mutex<Wanted> = Mutex::new(Wanted { on: false, generation: 0 });

/// Records a new request to turn the driver on or off; returns its generation.
pub fn want(on: bool) -> u64 {
    let mut w = WANTED.lock().unwrap();
    w.on = on;
    w.generation += 1;
    w.generation
}

/// No request has come after this one.
pub fn latest(generation: u64) -> bool {
    WANTED.lock().unwrap().generation == generation
}

fn wanted_on(generation: u64) -> bool {
    let w = WANTED.lock().unwrap();
    w.on && w.generation == generation
}

/// How a start ended when it didn't fail.
#[derive(Debug, PartialEq, Eq)]
pub enum Started {
    Ready,
    /// A later request (off, or another start) replaced this one; it spawned nothing it left running.
    Superseded,
}

/// Downloads the driver if needed and starts Lantern's daemon if it isn't running, for the request
/// `generation` (from `want(true)`).
pub fn start(generation: u64) -> Result<Started> {
    static STARTING: Mutex<()> = Mutex::new(());
    let _guard = STARTING.lock().unwrap();
    if !wanted_on(generation) {
        return Ok(Started::Superseded);
    }
    let bin = ensure_installed()?;
    if !wanted_on(generation) {
        return Ok(Started::Superseded);
    }
    {
        let mut daemon = DAEMON.lock().unwrap();
        if let Some(d) = daemon.as_mut()
            && d.child.try_wait().ok().flatten().is_none()
            && running(&bin)
        {
            return Ok(Started::Ready);
        }
        *daemon = None;
    }
    // Only the engine holding the lock may stop, replace or start the driver. Another copy of
    // Lantern (or a development preview) keeps it: replacing it would cut off its agents mid-task.
    // A Lantern that is quitting (an update relaunching) lets go within a few seconds.
    std::fs::create_dir_all(support_dir())?;
    let lock = support_dir().join("cua-driver.lock");
    let deadline = Instant::now() + Duration::from_secs(10);
    let owner = loop {
        if !wanted_on(generation) {
            return Ok(Started::Superseded);
        }
        if let Some(file) = claim(&lock)? {
            break file;
        }
        if Instant::now() >= deadline {
            bail!("another copy of October Lantern is using Cua Driver. Quit it, then turn this back on");
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    // A daemon left by an earlier engine (Lantern quit without stopping it): ours to replace.
    let _ = crate::run::output(driver(&bin).args(["stop", "--socket", &socket()]), Duration::from_secs(5));
    if !cfg!(windows) {
        let _ = std::fs::remove_file(socket());
    }
    let log = std::fs::File::create(support_dir().join("cua-driver.log")).ok();
    let mut cmd = driver(&bin);
    cmd.args(["serve", "--socket", &socket(), "--grant", "existing-profile"]);
    if cfg!(target_os = "macos") {
        cmd.args(["--embedded", "--host-bundle-id", "dev.october.lantern"]);
    }
    // Not detached from Lantern's app on macOS (that keeps Lantern's permissions); its own process
    // group, so it isn't interrupted with the engine, and no console window on Windows.
    crate::platform::detach(cmd.stdin(Stdio::null()));
    match log {
        Some(f) => cmd.stdout(f.try_clone()?).stderr(f),
        None => cmd.stdout(Stdio::null()).stderr(Stdio::null()),
    };
    {
        // Checked and spawned under the daemon's lock, which `stop_for` takes too: an off request
        // either comes first (and this start spawns nothing) or finds the daemon and stops it.
        let mut daemon = DAEMON.lock().unwrap();
        if !wanted_on(generation) {
            return Ok(Started::Superseded);
        }
        let child = cmd.spawn().context("starting Cua Driver")?;
        *daemon = Some(Daemon { child, _owner: owner });
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while !running(&bin) {
        if !wanted_on(generation) {
            // The off (or newer) request stops what this one spawned.
            return Ok(Started::Superseded);
        }
        if DAEMON.lock().unwrap().as_mut().is_some_and(|d| d.child.try_wait().ok().flatten().is_some()) {
            bail!("Cua Driver stopped as it started (see cua-driver.log in Lantern's support folder)");
        }
        if Instant::now() >= deadline {
            bail!("Cua Driver didn't start (see cua-driver.log in Lantern's support folder)");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(if wanted_on(generation) { Started::Ready } else { Started::Superseded })
}

/// Stops Lantern's daemon for the request `generation`, unless a later request came in (a newer
/// start's daemon isn't this request's to stop).
pub fn stop_for(generation: u64) {
    let taken = {
        let mut daemon = DAEMON.lock().unwrap();
        if !latest(generation) {
            return;
        }
        daemon.take()
    };
    if let Some(d) = taken {
        shut(d);
    }
}

/// Stops Lantern's daemon, whatever was asked last (the engine is exiting, or uninstalling).
pub fn stop() {
    let taken = DAEMON.lock().unwrap().take();
    if let Some(d) = taken {
        shut(d);
    }
}

fn shut(mut d: Daemon) {
    if let Some(bin) = binary() {
        let _ = crate::run::output(driver(&bin).args(["stop", "--socket", &socket()]), Duration::from_secs(3));
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while d.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = d.child.kill();
    let _ = d.child.wait();
}

/// Stops whichever engine's daemon is running (uninstalling: one started by an engine that was
/// then killed outlives it, holding its files in Lantern's folder).
pub fn stop_any() {
    stop();
    if let Some(bin) = binary() {
        let _ = crate::run::output(driver(&bin).args(["stop", "--socket", &socket()]), Duration::from_secs(5));
    }
}

/// Status for the app: `ready`, `installing`, `failed` (with `message`) or `off`.
pub fn status_json(status: &str, message: Option<String>) -> serde_json::Value {
    serde_json::json!({"type": "computer", "status": status, "message": message, "version": version()})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_holder_of_the_driver_lock_at_a_time() {
        let dir = std::env::temp_dir().join(format!("lantern-cua-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cua-driver.lock");
        let first = claim(&path).unwrap().expect("a free lock is taken");
        assert!(claim(&path).unwrap().is_none(), "a second claim waits while the first holds it");
        drop(first);
        assert!(claim(&path).unwrap().is_some(), "it's free again once the holder lets go");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_latest_request_starts_the_driver() {
        let first = want(true);
        assert!(wanted_on(first));
        let off = want(false);
        assert!(!wanted_on(first), "turning it off cancels the earlier start");
        assert!(latest(off) && !latest(first));
        // A start for the replaced request does nothing: no download, no lock, no driver.
        assert_eq!(start(first).unwrap(), Started::Superseded);
        let again = want(true);
        assert!(wanted_on(again) && !wanted_on(off));
        // The off request's stop doesn't touch a daemon a newer start owns.
        stop_for(off);
        want(false);
    }

    #[test]
    fn every_system_has_a_pinned_build_with_a_checksum() {
        let v = manifest().unwrap();
        for key in ["macos", "windows-x86_64", "windows-aarch64", "linux-x86_64", "linux-aarch64"] {
            let t = &v["targets"][key];
            assert!(t["url"].as_str().unwrap().starts_with("https://github.com/trycua/cua/releases/download/"), "{key}");
            assert_eq!(t["sha256"].as_str().unwrap().len(), 64, "{key}");
        }
        assert!(pin().is_some());
    }

    #[test]
    fn agents_reach_lanterns_own_driver() {
        let s = socket();
        if cfg!(windows) {
            assert!(s.starts_with(r"\\.\pipe\october-lantern-cua-"));
        } else {
            assert!(s.ends_with("cua-driver.sock"));
        }
    }
}
