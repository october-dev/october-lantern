//! October Harness, built in: the `october` coding agent (github.com/october-dev/october-harness,
//! npm `@october-dev/october`), available as an agent even when it isn't installed on this Mac.
//!
//! It isn't in the app. When `october` is already on the user's PATH, that one is used and
//! nothing is downloaded. Otherwise, the first time it's needed, Lantern installs the pinned
//! version from `engine/harness.json` into its support folder:
//!
//! - Node.js (the harness needs >= 22.19): a compatible one October Desktop already keeps in
//!   `~/Library/Application Support/October/runtime/` is used as it is (checked with
//!   `node --version`); otherwise the pinned Node.js tarball for this Mac's chip is downloaded
//!   from nodejs.org, checked against the SHA-256 in `harness.json`, and unpacked into
//!   `runtime/node-v<ver>-<os>-<arch>/` (nodejs.org's darwin/linux/win builds).
//! - The harness: `npm install --ignore-scripts @october-dev/october@<ver>` into
//!   `harness/<ver>/`, with that Node.js, and checked by running `october --version`.
//!
//! The `october` program is a Node.js script, so a launch puts the Node.js it was installed with
//! first on PATH (see `launch::agent_command_with`).
//!
//! Sign-in: the app hands the engine the October access token (`october.session`); it's kept in
//! memory only and given to October launches as `OCTOBER_SUPABASE_*`, as October Desktop does.
//! The refresh token never leaves the app.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::hooks::support_dir;

/// The October project's Supabase (as in the app's OctoberAccount.swift).
pub const SUPABASE_URL: &str = "https://auth.october.dev";
pub const SUPABASE_ANON_KEY: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImxhdHd4aXFqZ3ZsdWlkZGNrdm1qIiwicm9sZSI6ImFub24iLCJpYXQiOjE3NTAzMDExMzEsImV4cCI6MjA2NTg3NzEzMX0.Aor5lE6ZSvv83Or_CxQNUdRzRUqit3fODkNSJpcDJ7E";

/// The pinned harness and Node.js for this Mac's chip.
#[derive(Debug, Clone, PartialEq)]
pub struct Pin {
    pub package: String,
    pub version: String,
    pub node_version: String,
    pub node_minimum: (u32, u32, u32),
    pub node_url: String,
    pub node_sha256: String,
}

/// Where Node.js and the harness live and what they're called, per system (nodejs.org's names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    /// "darwin", "linux" or "win".
    pub os: &'static str,
    /// "arm64" or "x64".
    pub arch: &'static str,
}

impl Target {
    pub fn this() -> Self {
        let os = if cfg!(windows) {
            "win"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        };
        Self { os, arch: if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" } }
    }
    fn windows(self) -> bool {
        self.os == "win"
    }
    /// The key in `harness.json` and in nodejs.org's file names: "linux-x64".
    pub fn key(self) -> String {
        format!("{}-{}", self.os, self.arch)
    }
    /// The archive nodejs.org ships for this system.
    pub fn archive(self) -> &'static str {
        match self.os {
            "darwin" => "tar.gz",
            "linux" => "tar.xz",
            _ => "zip",
        }
    }
    /// npm's name for the system, for packages' `os` lists.
    pub fn npm_os(self) -> &'static str {
        if self.windows() { "win32" } else { self.os }
    }
    /// Where `node` lives in an unpacked Node.js folder (`bin/` on macOS and Linux, the top on Windows).
    pub fn node_bin(self, node_dir: &Path) -> PathBuf {
        if self.windows() { node_dir.to_path_buf() } else { node_dir.join("bin") }
    }
    /// The Node.js folder a `node_bin` belongs to.
    pub fn node_dir(self, node_bin: &Path) -> Option<PathBuf> {
        if self.windows() { Some(node_bin.to_path_buf()) } else { node_bin.parent().map(Path::to_path_buf) }
    }
    pub fn node_exe(self, node_bin: &Path) -> PathBuf {
        node_bin.join(if self.windows() { "node.exe" } else { "node" })
    }
    pub fn npm_cli(self, node_dir: &Path) -> PathBuf {
        let lib = if self.windows() { node_dir.to_path_buf() } else { node_dir.join("lib") };
        lib.join("node_modules").join("npm").join("bin").join("npm-cli.js")
    }
    /// The installed `october` program (npm's `.cmd` shim on Windows).
    pub fn harness_bin(self, install: &Path) -> PathBuf {
        install.join("node_modules").join(".bin").join(if self.windows() { "october.cmd" } else { "october" })
    }
    /// The separator in PATH lists.
    pub fn path_separator(self) -> char {
        if self.windows() { ';' } else { ':' }
    }
}

/// Reads `harness.json` for a system and chip (`Target::key`, e.g. "linux-x64").
pub(crate) fn parse_pin(json: &str, arch: &str) -> Option<Pin> {
    let v: Value = serde_json::from_str(json).ok()?;
    let node = &v["node"];
    let pin = Pin {
        package: v["package"].as_str()?.to_string(),
        version: v["version"].as_str()?.to_string(),
        node_version: node["version"].as_str()?.to_string(),
        node_minimum: parse_node_version(node["minimum"].as_str()?)?,
        node_url: node[arch]["url"].as_str()?.to_string(),
        node_sha256: node[arch]["sha256"].as_str()?.to_lowercase(),
    };
    let safe = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    let sha_ok = pin.node_sha256.len() == 64 && pin.node_sha256.chars().all(|c| c.is_ascii_hexdigit());
    (safe(&pin.version) && safe(&pin.node_version) && sha_ok && pin.node_url.starts_with("https://nodejs.org/dist/")).then_some(pin)
}

pub fn pin() -> Option<Pin> {
    parse_pin(include_str!("../harness.json"), &Target::this().key())
}

/// "v24.21.0" or "24.21.0" (as `node --version` prints it).
pub(crate) fn parse_node_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.trim().trim_start_matches('v').split('.');
    let n = |p: Option<&str>| p.and_then(|p| p.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok());
    Some((n(parts.next())?, n(parts.next())?, n(parts.next()).unwrap_or(0)))
}

/// The bundled `october`, and the Node.js `bin` folder to put first on PATH for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Bundled {
    pub bin: PathBuf,
    pub node_bin: PathBuf,
    pub version: String,
}

fn harness_dir(root: &Path, version: &str) -> PathBuf {
    root.join("harness").join(version)
}

/// Where a finished install records which Node.js it was installed with.
const NODE_RECORD: &str = "node-bin";

/// The pinned harness installed under `root`, if it's complete and its Node.js is still there.
pub fn bundled_in(root: &Path) -> Option<Bundled> {
    let pin = pin()?;
    let dir = harness_dir(root, &pin.version);
    let target = Target::this();
    let bin = target.harness_bin(&dir);
    let node_bin = PathBuf::from(std::fs::read_to_string(dir.join(NODE_RECORD)).ok()?.trim());
    (bin.is_file() && target.node_exe(&node_bin).is_file()).then_some(Bundled { bin, node_bin, version: pin.version })
}

/// The bundled `october`, once installed (or `LANTERN_HARNESS_BIN`, for development: the Node.js
/// next to it, or `LANTERN_HARNESS_NODE_BIN`, goes first on PATH).
pub fn binary() -> Option<Bundled> {
    if let Some(bin) = std::env::var_os("LANTERN_HARNESS_BIN").map(PathBuf::from).filter(|p| p.is_file()) {
        let node_bin = std::env::var_os("LANTERN_HARNESS_NODE_BIN")
            .map(PathBuf::from)
            .or_else(|| bin.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        return Some(Bundled { bin, node_bin, version: "dev".into() });
    }
    bundled_in(&support_dir())
}

/// What's happening with the bundled install, for `status`.
enum Progress {
    Idle,
    Installing,
    Failed(String),
}

static PROGRESS: Mutex<Progress> = Mutex::new(Progress::Idle);

/// Installs the pinned harness into Lantern's support folder if it isn't there yet. Slow the
/// first time (a download of about 50 MB and an npm install); callers run it off the main loop.
pub fn ensure_installed() -> Result<Bundled> {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(b) = binary() {
        return Ok(b);
    }
    *PROGRESS.lock().unwrap() = Progress::Installing;
    let result = install_into(&support_dir(), true);
    *PROGRESS.lock().unwrap() = match &result {
        Ok(_) => Progress::Idle,
        Err(e) => Progress::Failed(format!("{e:#}")),
    };
    result
}

/// Starts `ensure_installed` on its own thread; `done` runs when it's finished.
pub fn prepare(done: impl FnOnce(Result<Bundled>) + Send + 'static) {
    std::thread::spawn(move || done(ensure_installed()));
}

/// Waits for the bundled harness until `until`, starting its install if needed.
pub fn wait_installed(until: Instant) -> Result<Bundled> {
    if let Some(b) = binary() {
        return Ok(b);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    prepare(move |r| {
        let _ = tx.send(r);
    });
    match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
        Ok(r) => r,
        Err(_) => bail!("October Harness is still downloading (only the first time). Try again in a minute."),
    }
}

/// `{"status": "ready|installing|failed|missing", "source": "path|bundled", "version", "message"}`.
/// `on_path`: `october` is already on the user's PATH (then nothing is downloaded).
pub fn status(on_path: bool) -> Value {
    let version = pin().map(|p| p.version).unwrap_or_default();
    if on_path {
        return json!({"status": "ready", "source": "path", "version": Value::Null});
    }
    if let Some(b) = binary() {
        return json!({"status": "ready", "source": "bundled", "version": b.version});
    }
    match &*PROGRESS.lock().unwrap() {
        Progress::Installing => json!({"status": "installing", "version": version}),
        Progress::Failed(m) => json!({"status": "failed", "version": version, "message": m}),
        Progress::Idle => json!({"status": "missing", "version": version}),
    }
}

/// The installed Node.js version at `node_bin/node`, if it runs.
fn node_version(node_bin: &Path) -> Option<(u32, u32, u32)> {
    let out = crate::run::output(Command::new(Target::this().node_exe(node_bin)).arg("--version"), Duration::from_secs(10)).ok()?;
    if !out.status.success() {
        return None;
    }
    parse_node_version(&String::from_utf8_lossy(&out.stdout))
}

/// A Node.js runtime folder that works here and is new enough, with npm.
fn usable_node(dir: &Path, minimum: (u32, u32, u32)) -> Option<PathBuf> {
    let target = Target::this();
    let bin = target.node_bin(dir);
    let npm = target.npm_cli(dir);
    (npm.is_file() && node_version(&bin)? >= minimum).then_some(bin)
}

/// A compatible Node.js October Desktop already keeps, newest first (macOS; elsewhere Lantern
/// downloads its own).
fn desktop_node(minimum: (u32, u32, u32)) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let runtime = crate::transcripts::home().join("Library/Application Support/October/runtime");
    let suffix = format!("-{}", Target::this().key());
    let mut found: Vec<((u32, u32, u32), PathBuf)> = std::fs::read_dir(runtime)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let version = name.strip_prefix("node-v")?.strip_suffix(&suffix)?;
            Some((parse_node_version(version)?, e.path()))
        })
        .filter(|(v, _)| *v >= minimum)
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.into_iter().find_map(|(_, dir)| usable_node(&dir, minimum))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The pinned Node.js under `root/runtime`, downloaded and checked if it isn't there.
fn download_node(root: &Path, pin: &Pin) -> Result<PathBuf> {
    let runtime = root.join("runtime");
    let target = Target::this();
    let name = format!("node-v{}-{}", pin.node_version, target.key());
    let dest = runtime.join(&name);
    if let Some(bin) = usable_node(&dest, pin.node_minimum) {
        return Ok(bin);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(300))).build().into();
    let mut resp = agent.get(&pin.node_url).call().context("downloading Node.js")?;
    let bytes = resp.body_mut().with_config().limit(256 * 1024 * 1024).read_to_vec().context("downloading Node.js")?;
    if sha256_hex(&bytes) != pin.node_sha256 {
        bail!("the Node.js download didn't match its checksum, so it wasn't used");
    }
    std::fs::create_dir_all(&runtime)?;
    let work = runtime.join(format!(".node-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&work)?;
    let result = (|| -> Result<()> {
        let archive = work.join(format!("node.{}", target.archive()));
        std::fs::write(&archive, &bytes)?;
        // macOS's and Windows' own tar (bsdtar) read .tar.gz and .zip; GNU tar reads Linux's .tar.xz.
        let (tar, flags) = match target.os {
            "darwin" => ("/usr/bin/tar", "-xzf"),
            "linux" => ("tar", "-xJf"),
            _ => ("tar", "-xf"),
        };
        let out = crate::run::output(Command::new(tar).arg(flags).arg(&archive).arg("-C").arg(&work), Duration::from_secs(120))?;
        if !out.status.success() {
            bail!("couldn't unpack Node.js");
        }
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(work.join(&name), &dest)?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    result?;
    // Older pinned Node.js versions of Lantern's own are no longer used.
    for old in std::fs::read_dir(&runtime).into_iter().flatten().flatten() {
        let n = old.file_name().to_string_lossy().into_owned();
        if n.starts_with("node-v") && old.path() != dest {
            let _ = std::fs::remove_dir_all(old.path());
        }
    }
    usable_node(&dest, pin.node_minimum).context("the downloaded Node.js doesn't run on this computer")
}

/// Installs the pinned harness under `root` (`support_dir()` in the app). `reuse_desktop`: use a
/// compatible Node.js from October Desktop instead of downloading one.
pub fn install_into(root: &Path, reuse_desktop: bool) -> Result<Bundled> {
    let target = Target::this();
    let pin = pin().context("this build has no October Harness to install")?;
    if let Some(b) = bundled_in(root) {
        return Ok(b);
    }
    let node_bin = match reuse_desktop.then(|| desktop_node(pin.node_minimum)).flatten() {
        Some(bin) => bin,
        None => download_node(root, &pin)?,
    };
    let harness = root.join("harness");
    std::fs::create_dir_all(&harness)?;
    let work = harness.join(format!(".install-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&work)?;
    let dest = harness_dir(root, &pin.version);
    let result = (|| -> Result<()> {
        let npm_cli = target.npm_cli(&target.node_dir(&node_bin).context("no Node.js folder")?);
        // Its Node.js first; then the system's basics (npm runs no install scripts here).
        let system = if cfg!(windows) { std::env::var("PATH").unwrap_or_default() } else { "/usr/bin:/bin:/usr/sbin:/sbin".to_string() };
        let path = format!("{}{}{system}", node_bin.display(), target.path_separator());
        let out = crate::run::output(
            Command::new(target.node_exe(&node_bin))
                .arg(npm_cli)
                .args(["install", "--prefix"])
                .arg(&work)
                .args(["--ignore-scripts", "--no-audit", "--no-fund", "--no-update-notifier", "--loglevel=error"])
                .arg(format!("{}@{}", pin.package, pin.version))
                .env("PATH", &path)
                .env("npm_config_cache", work.join(".npm-cache")),
            Duration::from_secs(300),
        )
        .context("installing October Harness")?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            bail!("npm couldn't install October Harness: {}", crate::model::truncate(last, 200));
        }
        let _ = std::fs::remove_dir_all(work.join(".npm-cache"));
        // npm installs every platform's optional packages from the harness's shrinkwrap (about
        // 300 MB of esbuild builds); only this system's can ever load.
        prune_other_platforms(&work.join("node_modules"), 0, target);
        let bin = target.harness_bin(&work);
        if !bin.is_file() {
            bail!("October Harness installed without its `october` program");
        }
        // npm's shim on Windows is a batch file: cmd runs it, with this Node.js first on PATH.
        let mut check = if target.os == "win" {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(&bin).env("PATH", &path);
            cmd
        } else {
            let mut cmd = Command::new(target.node_exe(&node_bin));
            cmd.arg(&bin);
            cmd
        };
        let out = crate::run::output(check.arg("--version"), Duration::from_secs(30))?;
        if !out.status.success() {
            bail!("the installed October Harness doesn't run");
        }
        std::fs::write(work.join(NODE_RECORD), node_bin.to_string_lossy().as_bytes())?;
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&work, &dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&work);
    }
    result?;
    // Older versions (and unfinished installs) are no longer used.
    for old in std::fs::read_dir(&harness).into_iter().flatten().flatten() {
        if old.path() != dest {
            let _ = std::fs::remove_dir_all(old.path());
        }
    }
    bundled_in(root).context("October Harness didn't install completely")
}

/// Whether a package's `os` / `cpu` list (npm's format, `!` for "not") allows `value`. No list
/// allows everything.
pub(crate) fn allows(list: &Value, value: &str) -> bool {
    let Some(items) = list.as_array() else { return true };
    let items: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
    if items.contains(&format!("!{value}").as_str()) {
        return false;
    }
    let positive: Vec<&&str> = items.iter().filter(|i| !i.starts_with('!')).collect();
    positive.is_empty() || positive.iter().any(|i| **i == value)
}

/// Removes packages under `node_modules` that are built for another OS or chip.
fn prune_other_platforms(node_modules: &Path, depth: usize, target: Target) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(node_modules) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        if name.starts_with('@') {
            prune_other_platforms(&path, depth + 1, target);
            continue;
        }
        let manifest: Option<Value> = std::fs::read(path.join("package.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
        if let Some(m) = manifest
            && (!allows(&m["os"], target.npm_os()) || !allows(&m["cpu"], target.arch))
        {
            let _ = std::fs::remove_dir_all(&path);
            continue;
        }
        prune_other_platforms(&path.join("node_modules"), depth + 1, target);
    }
}

// MARK: Sign-in

/// The latest October access token from the app and when it expires (unix seconds). Memory only.
static SESSION: Mutex<Option<(String, u64)>> = Mutex::new(None);

/// The app's `october.session`: a token, or `None` when signed out.
pub fn set_session(session: Option<(String, u64)>) {
    let session = session.filter(|(t, _)| !t.is_empty());
    write_token_file(session.as_ref());
    *SESSION.lock().unwrap() = session;
}

/// Where `lantern-engine october-token` reads the current access token: a 0600 file in Lantern's
/// 0700 support folder, rewritten whenever the app signs in or refreshes, and removed on sign-out.
/// Access token only (it expires within the hour); never the refresh token.
pub fn token_file() -> std::path::PathBuf {
    crate::hooks::support_dir().join("october-token.json")
}

fn write_token_file(session: Option<&(String, u64)>) {
    let path = token_file();
    let Some((token, expires_at)) = session else {
        let _ = std::fs::remove_file(&path);
        return;
    };
    use std::io::Write;
    let _ = std::fs::create_dir_all(crate::hooks::support_dir());
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let body = serde_json::json!({"accessToken": token, "expiresAt": expires_at}).to_string();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // Only you can read it (Windows: the support folder is under your own profile).
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options.open(&tmp).and_then(|mut f| f.write_all(body.as_bytes()));
    if written.is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// `lantern-engine october-token`, the harness's `OCTOBER_TOKEN_COMMAND`: prints
/// {"accessToken","expiresAt"} when the app's current token is still good for at least a minute.
pub fn print_token() -> bool {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let Ok(raw) = std::fs::read_to_string(token_file()) else { return false };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else { return false };
    match (v["accessToken"].as_str(), v["expiresAt"].as_u64()) {
        (Some(t), Some(exp)) if !t.is_empty() && exp > now + 60 => {
            println!("{}", serde_json::json!({"accessToken": t, "expiresAt": exp}));
            true
        }
        _ => false,
    }
}

/// `OCTOBER_SUPABASE_*` for an October launch: empty when signed out or the token has expired
/// (the harness then asks you to /login itself). Never a refresh token.
pub fn session_env() -> Vec<(String, String)> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut env = env_for(SESSION.lock().unwrap().as_ref(), now);
    // The harness runs this through the shell near expiry and after a 401 to get a fresh token.
    if !env.is_empty()
        && let Ok(exe) = std::env::current_exe()
    {
        env.push(("OCTOBER_TOKEN_COMMAND".into(), token_command(&exe.to_string_lossy(), cfg!(windows))));
    }
    env
}

/// The command the harness's shell runs for a fresh token: POSIX quoting, or cmd's double quotes
/// on Windows (where a path can't contain `"`).
pub(crate) fn token_command(exe: &str, windows: bool) -> String {
    if windows { format!("\"{exe}\" october-token") } else { format!("'{}' october-token", exe.replace('\'', "'\\''")) }
}

pub(crate) fn env_for(session: Option<&(String, u64)>, now: u64) -> Vec<(String, String)> {
    match session {
        Some((token, expires_at)) if *expires_at > now => vec![
            ("OCTOBER_SUPABASE_URL".into(), SUPABASE_URL.into()),
            ("OCTOBER_SUPABASE_ANON_KEY".into(), SUPABASE_ANON_KEY.into()),
            ("OCTOBER_SUPABASE_ACCESS_TOKEN".into(), token.clone()),
            ("OCTOBER_SUPABASE_EXPIRES_AT".into(), expires_at.to_string()),
        ],
        _ => Vec::new(),
    }
}
