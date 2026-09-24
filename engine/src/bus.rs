//! October Bus for the agents Lantern starts: every agent gets the Bus's tools (see its peers,
//! message them, share tasks), and Lantern links it to the other agents it started, so they can
//! work together.
//!
//! Lantern uses the public October Bus (github.com/october-dev/october-bus): a local daemon, with
//! its data on this Mac, no account. The `october-bus` program isn't in the app: it's downloaded
//! the first time it's needed (a small signed build for this Mac's chip, from a pinned commit,
//! published with Lantern's releases) and checked against the SHA-256 in `engine/bus.json`.
//! Lantern starts the daemon when it isn't running and keeps its agents in one scope, "lantern".
//!
//! Each agent gets an MCP entry that registers it with the Bus when the agent starts it
//! (`october-bus mcp stdio --scope lantern --agent <id>`), passed per launch so no config file of
//! the user's is changed: Claude Code `--mcp-config`, Codex `-c` overrides, OpenCode
//! `OPENCODE_CONFIG`. The October harness talks to the Bus natively and runs under
//! `october-bus agent run`. Other agents start without the Bus for now.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::hooks::support_dir;
use crate::launch::quote;
use crate::model::Kind;

pub const SCOPE: &str = "lantern";

/// The pinned build: commit, and this chip's download and checksum.
struct Pin {
    commit: String,
    url: String,
    sha256: String,
}

fn pin() -> Option<Pin> {
    let v: serde_json::Value = serde_json::from_str(include_str!("../bus.json")).ok()?;
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
    Some(Pin {
        commit: v["commit"].as_str()?.to_string(),
        url: v[arch]["url"].as_str()?.to_string(),
        sha256: v[arch]["sha256"].as_str()?.to_string(),
    })
}

/// Where the downloaded Bus lives, named by its commit.
fn installed_path() -> Option<PathBuf> {
    let p = pin()?;
    Some(support_dir().join(format!("bin/october-bus-{}", &p.commit[..7.min(p.commit.len())])))
}

/// The `october-bus` program, once downloaded (or `LANTERN_BUS_BIN`, for development).
pub fn binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LANTERN_BUS_BIN").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    installed_path().filter(|p| p.is_file())
}

/// Downloads the pinned Bus for this Mac if it isn't here yet, and checks it before keeping it.
pub fn ensure_installed() -> Result<PathBuf> {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap();
    if let Some(p) = binary() {
        return Ok(p);
    }
    let pin = pin().context("this build has no October Bus to download")?;
    let dest = installed_path().context("no install path")?;
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(120))).build().into();
    let mut resp = agent.get(&pin.url).call().context("downloading October Bus")?;
    let bytes = resp.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec().context("downloading October Bus")?;
    use sha2::Digest;
    let sum: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if sum != pin.sha256 {
        bail!("the October Bus download didn't match its checksum, so it wasn't used");
    }
    let bin_dir = dest.parent().context("no folder")?;
    std::fs::create_dir_all(bin_dir)?;
    let work = bin_dir.join(format!(".bus-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&work)?;
    let result = (|| -> Result<()> {
        let zip = work.join("bus.zip");
        std::fs::write(&zip, &bytes)?;
        let out = crate::run::output(Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(&zip).arg(&work), Duration::from_secs(30))?;
        if !out.status.success() {
            bail!("couldn't unpack October Bus");
        }
        use std::os::unix::fs::PermissionsExt;
        let bin = work.join("october-bus");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&bin, &dest)?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    result?;
    // Older pinned builds are no longer used.
    for old in std::fs::read_dir(bin_dir).into_iter().flatten().flatten() {
        let name = old.file_name().to_string_lossy().into_owned();
        if name.starts_with("october-bus-") && old.path() != dest {
            let _ = std::fs::remove_file(old.path());
        }
    }
    Ok(dest)
}

/// Which agents Lantern can connect.
pub fn supported(kind: Kind) -> bool {
    matches!(kind, Kind::Claude | Kind::Codex | Kind::Opencode | Kind::October)
}

fn bus(args: &[&str], limit: Duration) -> Result<std::process::Output> {
    let bin = binary().context("October Bus isn't downloaded yet")?;
    crate::run::output(Command::new(bin).args(args), limit)
}

/// Starts the daemon if it isn't running and makes sure Lantern's scope exists.
pub fn ensure_ready() -> Result<()> {
    static READY: Mutex<Option<Instant>> = Mutex::new(None);
    if READY.lock().unwrap().is_some_and(|at| at.elapsed() < Duration::from_secs(60)) {
        return Ok(());
    }
    ensure_installed()?;
    let running = |_: ()| bus(&["status"], Duration::from_secs(3)).is_ok_and(|o| o.status.success());
    if !running(()) {
        let bin = binary().context("October Bus isn't downloaded yet")?;
        let log = std::fs::File::create(support_dir().join("october-bus.log")).ok();
        use std::os::unix::process::CommandExt;
        let mut cmd = Command::new(bin);
        cmd.arg("start").stdin(Stdio::null()).process_group(0);
        match log {
            Some(f) => cmd.stdout(f.try_clone()?).stderr(f),
            None => cmd.stdout(Stdio::null()).stderr(Stdio::null()),
        };
        // Runs on its own, outliving the engine; the next engine finds it through its run file.
        cmd.spawn().context("starting October Bus")?;
        let deadline = Instant::now() + Duration::from_secs(8);
        while !running(()) {
            if Instant::now() >= deadline {
                bail!("October Bus didn't start (see october-bus.log in Lantern's support folder)");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let scopes = bus(&["scope", "list"], Duration::from_secs(5))?;
    if !String::from_utf8_lossy(&scopes.stdout).contains(&format!("\"{SCOPE}\"")) {
        let out = bus(&["scope", "create", SCOPE], Duration::from_secs(5))?;
        if !out.status.success() {
            bail!("couldn't create Lantern's Bus scope: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
    }
    *READY.lock().unwrap() = Some(Instant::now());
    Ok(())
}

/// How one launch joins the Bus: arguments for the agent, environment, or a wrapper command.
#[derive(Debug, Default, PartialEq)]
pub struct Attach {
    pub id: String,
    pub name: String,
    /// Shell words added after the agent's program name.
    pub args: Vec<String>,
    /// `KEY=value` set for the agent.
    pub env: Vec<(String, String)>,
    /// Shell words run in place of the program, with the program after them.
    pub wrap: Vec<String>,
}

/// A new Bus identity for an agent Lantern is starting.
pub fn new_id(kind: Kind) -> String {
    format!("lantern-{}-{}", kind.as_str(), &uuid::Uuid::new_v4().simple().to_string()[..8])
}

/// Prepares one launch. Writes the agent's MCP file, when it needs one, in Lantern's support
/// folder.
pub fn attach(kind: Kind, id: &str, name: &str) -> Result<Attach> {
    let bin = binary().context("October Bus isn't downloaded yet")?;
    let bin = bin.to_string_lossy().into_owned();
    let mut a = Attach { id: id.into(), name: name.into(), ..Default::default() };
    let dir = support_dir().join("bus");
    std::fs::create_dir_all(&dir)?;
    let generate = |host: &str, file: &PathBuf| -> Result<()> {
        let _ = std::fs::remove_file(file);
        let out = bus(
            &["harness", "config", host, "--scope", SCOPE, "--agent", id, "--name", name, "--output", &file.to_string_lossy()],
            Duration::from_secs(5),
        )?;
        if !out.status.success() {
            bail!("October Bus couldn't prepare {host}: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    };
    match kind {
        Kind::Claude => {
            let file = dir.join(format!("{id}.mcp.json"));
            generate("claude-code", &file)?;
            a.args = vec!["--mcp-config".into(), quote(&file.to_string_lossy())];
        }
        Kind::Codex => {
            // Codex takes config overrides as TOML values.
            let args = mcp_args(id, name);
            let toml_args = format!("[{}]", args.iter().map(|s| toml_string(s)).collect::<Vec<_>>().join(","));
            for kv in [
                format!("mcp_servers.october_bus.command={}", toml_string(&bin)),
                format!("mcp_servers.october_bus.args={toml_args}"),
                "mcp_servers.october_bus.startup_timeout_sec=20".to_string(),
            ] {
                a.args.push("-c".into());
                a.args.push(quote(&kv));
            }
        }
        Kind::Opencode => {
            let file = dir.join(format!("{id}.opencode.json"));
            generate("opencode", &file)?;
            a.env.push(("OPENCODE_CONFIG".into(), file.to_string_lossy().into_owned()));
        }
        Kind::October => {
            a.wrap = vec![quote(&bin), "agent".into(), "run".into(), "--scope".into(), SCOPE.into(), "--id".into(), quote(id)];
            a.wrap.extend(["--name".into(), quote(name), "--".into()]);
        }
        _ => bail!("October Bus isn't set up for this agent yet"),
    }
    Ok(a)
}

/// `october-bus mcp stdio` arguments that register the agent when the agent starts it.
fn mcp_args(id: &str, name: &str) -> Vec<String> {
    ["mcp", "stdio", "--scope", SCOPE, "--agent", id, "--name", name].iter().map(|s| s.to_string()).collect()
}

fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The line the agent's first message gets, so it knows it has teammates.
pub fn note(name: &str) -> String {
    format!(
        "You're connected to October Bus as \"{name}\", together with the other agents Lantern started on this Mac. \
         Use its tools (list_peers, message_peer, check_inbox, and the task tools) to coordinate with them, and check \
         your inbox when you finish a step."
    )
}

/// Agents Lantern started on the Bus, remembered across engine restarts for a day.
fn registry() -> PathBuf {
    support_dir().join("bus/agents.json")
}

fn known() -> Vec<(String, u64)> {
    let now = crate::hooks::now_ms();
    std::fs::read(registry())
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<(String, u64)>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, at)| now.saturating_sub(*at) < 24 * 3600 * 1000)
        .collect()
}

/// Links a newly started agent with every other agent Lantern started, as soon as it has
/// registered (when the agent starts its Bus connection), retrying for two minutes.
pub fn link_when_ready(id: String) {
    static LOCK: Mutex<()> = Mutex::new(());
    let others: Vec<String> = {
        let _guard = LOCK.lock().unwrap();
        let mut all = known();
        let others = all.iter().map(|(i, _)| i.clone()).filter(|i| *i != id).collect();
        all.push((id.clone(), crate::hooks::now_ms()));
        let _ = std::fs::create_dir_all(support_dir().join("bus"));
        let _ = crate::hooks::write_atomic(&registry(), &serde_json::to_vec(&all).unwrap_or_default());
        others
    };
    std::thread::spawn(move || {
        let mut pending = others;
        let deadline = Instant::now() + Duration::from_secs(120);
        while !pending.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_secs(3));
            pending.retain(|other| {
                let linked = bus(&["link", "--scope", SCOPE, &id, other], Duration::from_secs(5)).is_ok_and(|o| o.status.success());
                !linked
            });
        }
    });
}
