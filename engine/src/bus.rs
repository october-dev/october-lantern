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
//! `OPENCODE_CONFIG`, GitHub Copilot `--additional-mcp-config`, Goose `--with-extension`, and
//! Gemini CLI / Qwen Code a system-settings file of Lantern's (its own copy of any system settings
//! already there, plus the Bus). The October harness talks to the Bus natively and runs under
//! `october-bus agent run`. Grok, Cursor and Pi have no per-launch way yet and start without it.
//! The same per-launch config carries Cua Driver's MCP server (see `computer`), on the Bus or not.

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
    // The legacy manifest contains macOS binaries only. Never run one on Linux.
    if !cfg!(target_os = "macos") {
        return None;
    }
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
    // Linux and Windows packages carry a native build from the same pinned source as the Mac
    // package. On Windows it runs from Lantern's own copy, named by commit like the Mac download,
    // so the daemon and agents' Bus connections never hold the installed app's files.
    if cfg!(windows) {
        return windows_copy().filter(|p| p.is_file());
    }
    if cfg!(target_os = "linux") {
        return bundled();
    }
    installed_path().filter(|p| p.is_file())
}

/// Downloads the pinned Bus for this Mac if it isn't here yet, and checks it before keeping it.
/// The build that came with the app, next to the engine (Linux and Windows).
fn bundled() -> Option<PathBuf> {
    let name = format!("october-bus{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe().ok().map(|p| p.with_file_name(name)).filter(|p| p.is_file())
}

/// Where Windows keeps its copy of the bundled build.
fn windows_copy() -> Option<PathBuf> {
    let v: serde_json::Value = serde_json::from_str(include_str!("../bus.json")).ok()?;
    let commit = v["commit"].as_str()?;
    Some(support_dir().join(format!("bin/october-bus-{}.exe", &commit[..7.min(commit.len())])))
}

pub fn ensure_installed() -> Result<PathBuf> {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap();
    if let Some(p) = binary() {
        return Ok(p);
    }
    if cfg!(windows) {
        let source = bundled().context("this copy of Lantern has no October Bus; rebuild it with scripts/build-bus-windows.ps1")?;
        let dest = windows_copy().context("no install path")?;
        std::fs::create_dir_all(dest.parent().context("no folder")?)?;
        let tmp = dest.with_extension("new");
        std::fs::copy(&source, &tmp)?;
        std::fs::rename(&tmp, &dest)?;
        return Ok(dest);
    }
    let pin = pin().context("no pinned October Bus build for this OS; set LANTERN_BUS_BIN to a native october-bus binary")?;
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
        let bin = work.join("october-bus");
        crate::platform::set_mode(&bin, 0o755)?;
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
    matches!(kind, Kind::Claude | Kind::Codex | Kind::Opencode | Kind::October | Kind::Gemini | Kind::Qwen | Kind::Copilot | Kind::Goose)
}

fn bus(args: &[&str], limit: Duration) -> Result<std::process::Output> {
    let bin = binary().context("October Bus isn't downloaded yet")?;
    crate::run::output(Command::new(bin).args(args), limit)
}

/// Stops Lantern's Bus daemon, if one is running (uninstalling: it runs from Lantern's folder).
pub fn stop() {
    if binary().is_some() {
        let _ = bus(&["stop"], Duration::from_secs(5));
    }
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
        std::fs::create_dir_all(support_dir())?;
        let log = std::fs::File::create(support_dir().join("october-bus.log")).ok();
        let mut cmd = Command::new(bin);
        crate::platform::detach(cmd.arg("start").stdin(Stdio::null()));
        match log {
            Some(f) => cmd.stdout(f.try_clone()?).stderr(f),
            None => cmd.stdout(Stdio::null()).stderr(Stdio::null()),
        };
        // Runs on its own, outliving the engine; the next engine finds it through its run file.
        let mut child = cmd.spawn().context("starting October Bus")?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
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

/// One word of an agent's command line. Unix launches go through the login shell, so words are
/// quoted for it; Windows launches pass each word as its own argument, untouched.
fn word(s: &str) -> String {
    if cfg!(windows) { s.to_string() } else { quote(s) }
}

/// How one launch joins the Bus and gets other MCP servers (Cua Driver): arguments for the agent,
/// environment, or a wrapper command.
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
    /// The agent joins October Bus.
    pub on_bus: bool,
    /// The agent got Cua Driver's MCP tools.
    pub computer: bool,
}

/// A new Bus identity for an agent Lantern is starting.
pub fn new_id(kind: Kind) -> String {
    format!("lantern-{}-{}", kind.as_str(), &uuid::Uuid::new_v4().simple().to_string()[..8])
}

/// Agents that take MCP servers per launch (all the Bus's but the October harness).
pub fn takes_mcp(kind: Kind) -> bool {
    supported(kind) && kind != Kind::October
}

/// An MCP server: (command, arguments).
pub type Server = (String, Vec<String>);

/// The MCP server name Cua Driver gets in an agent's config.
const COMPUTER: &str = "cua_driver";

/// Prepares one launch: on October Bus when `on_bus`, and with Cua Driver's MCP server when
/// `computer` is given (command, arguments). Writes the agent's MCP file, when it needs one, in
/// Lantern's support folder.
pub fn attach(kind: Kind, id: &str, name: &str, on_bus: bool, computer: Option<&Server>) -> Result<Attach> {
    let computer = computer.filter(|_| takes_mcp(kind));
    let mut a = Attach { id: id.into(), name: name.into(), on_bus, computer: computer.is_some(), ..Default::default() };
    if !on_bus && computer.is_none() {
        return Ok(a);
    }
    let bin = if on_bus { binary().context("October Bus isn't downloaded yet")?.to_string_lossy().into_owned() } else { String::new() };
    let dir = support_dir().join("bus");
    std::fs::create_dir_all(&dir)?;
    // The Bus's config for `host` when on the Bus, else `empty`; then Cua Driver added by `add`.
    let config = |host: &str, file: &PathBuf, empty: serde_json::Value, add: &dyn Fn(&mut serde_json::Value, &Server)| -> Result<()> {
        let _ = std::fs::remove_file(file);
        let mut v = if on_bus {
            let out = bus(
                &["harness", "config", host, "--scope", SCOPE, "--agent", id, "--name", name, "--output", &file.to_string_lossy()],
                Duration::from_secs(5),
            )?;
            if !out.status.success() {
                bail!("October Bus couldn't prepare {host}: {}", String::from_utf8_lossy(&out.stderr).trim());
            }
            serde_json::from_slice(&std::fs::read(file)?).context("October Bus wrote an unreadable config file")?
        } else {
            empty
        };
        if let Some(c) = computer {
            add(&mut v, c);
        }
        crate::hooks::write_atomic(file, &serde_json::to_vec_pretty(&v)?)?;
        Ok(())
    };
    let stdio = |v: &mut serde_json::Value, (command, args): &Server| {
        v["mcpServers"][COMPUTER] = serde_json::json!({"type": "stdio", "command": command, "args": args});
    };
    match kind {
        Kind::Claude => {
            let file = dir.join(format!("{id}.mcp.json"));
            config("claude-code", &file, serde_json::json!({"mcpServers": {}}), &stdio)?;
            a.args = vec!["--mcp-config".into(), word(&file.to_string_lossy())];
        }
        Kind::Codex => {
            // Codex takes config overrides as TOML values.
            let mut servers = Vec::new();
            if on_bus {
                servers.push(("october_bus", bin.clone(), mcp_args(id, name)));
            }
            if let Some((command, args)) = computer {
                servers.push((COMPUTER, command.clone(), args.clone()));
            }
            for (server, command, args) in servers {
                let toml_args = format!("[{}]", args.iter().map(|s| toml_string(s)).collect::<Vec<_>>().join(","));
                for kv in [
                    format!("mcp_servers.{server}.command={}", toml_string(&command)),
                    format!("mcp_servers.{server}.args={toml_args}"),
                    format!("mcp_servers.{server}.startup_timeout_sec=20"),
                ] {
                    a.args.push("-c".into());
                    a.args.push(word(&kv));
                }
            }
        }
        Kind::Opencode => {
            let file = dir.join(format!("{id}.opencode.json"));
            config("opencode", &file, serde_json::json!({"mcp": {}}), &|v, (command, args)| {
                let mut words = vec![command.clone()];
                words.extend(args.iter().cloned());
                v["mcp"][COMPUTER] = serde_json::json!({"type": "local", "command": words, "enabled": true});
            })?;
            a.env.push(("OPENCODE_CONFIG".into(), file.to_string_lossy().into_owned()));
        }
        Kind::Copilot => {
            let file = dir.join(format!("{id}.copilot.json"));
            config("copilot-cli", &file, serde_json::json!({"mcpServers": {}}), &|v, (command, args)| {
                v["mcpServers"][COMPUTER] = serde_json::json!({"type": "local", "command": command, "args": args, "tools": ["*"]});
            })?;
            a.args = vec!["--additional-mcp-config".into(), word(&format!("@{}", file.to_string_lossy()))];
        }
        Kind::Gemini | Kind::Qwen => {
            let (host, var, system) = match (kind == Kind::Gemini, cfg!(windows)) {
                (true, false) => ("gemini-cli", "GEMINI_CLI_SYSTEM_SETTINGS_PATH", "/Library/Application Support/GeminiCli/settings.json"),
                (true, true) => ("gemini-cli", "GEMINI_CLI_SYSTEM_SETTINGS_PATH", r"C:\ProgramData\gemini-cli\settings.json"),
                (false, false) => ("qwen-code", "QWEN_CODE_SYSTEM_SETTINGS_PATH", "/Library/Application Support/QwenCode/settings.json"),
                (false, true) => ("qwen-code", "QWEN_CODE_SYSTEM_SETTINGS_PATH", r"C:\ProgramData\qwen-code\settings.json"),
            };
            let file = dir.join(format!("{id}.{host}.json"));
            config(host, &file, serde_json::json!({"mcpServers": {}}), &|v, (command, args)| {
                v["mcpServers"][COMPUTER] = serde_json::json!({"command": command, "args": args});
            })?;
            // These read one system settings file; keep whatever the computer's own one says.
            let existing = std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(system));
            let merged = merge_settings(std::fs::read(&existing).ok().as_deref(), &std::fs::read(&file)?)?;
            crate::hooks::write_atomic(&file, &merged)?;
            a.env.push((var.into(), file.to_string_lossy().into_owned()));
        }
        Kind::Goose => {
            // `goose session --with-extension '<command>'`, the command in one string Goose splits
            // the way a shell would; one per server.
            a.args = vec!["session".into()];
            let mut servers = Vec::new();
            if on_bus {
                servers.push((bin.clone(), mcp_args(id, name)));
            }
            if let Some(c) = computer {
                servers.push(c.clone());
            }
            for (command, args) in servers {
                let mut words = vec![quote(&command)];
                words.extend(args.iter().map(|w| quote(w)));
                a.args.extend(["--with-extension".into(), word(&words.join(" "))]);
            }
        }
        Kind::October => {
            // The harness talks to the Bus natively; it uses Cua Driver through its command
            // (`computer::note`).
            a.wrap = vec![word(&bin), "agent".into(), "run".into(), "--scope".into(), SCOPE.into(), "--id".into(), word(id)];
            a.wrap.extend(["--name".into(), word(name), "--".into()]);
        }
        _ => bail!("October Bus isn't set up for this agent yet"),
    }
    Ok(a)
}

/// A settings file with `bus`'s MCP servers (October Bus's, Cua Driver's) added to `base` (another
/// settings file, kept as it is otherwise).
pub(crate) fn merge_settings(base: Option<&[u8]>, bus: &[u8]) -> Result<Vec<u8>> {
    let bus: serde_json::Value = serde_json::from_slice(bus).context("October Bus wrote an unreadable settings file")?;
    let mut out =
        base.and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok()).filter(|v| v.is_object()).unwrap_or(serde_json::json!({}));
    if !out["mcpServers"].is_object() {
        out["mcpServers"] = serde_json::json!({});
    }
    for (name, server) in bus["mcpServers"].as_object().into_iter().flatten() {
        out["mcpServers"][name] = server.clone();
    }
    Ok(serde_json::to_vec_pretty(&out)?)
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
        "You're connected to October Bus as \"{name}\", together with the other agents Lantern started on this computer. \
         Use its tools (list_peers, message_peer, check_inbox, and the task tools) to coordinate with them, and check \
         your inbox when you finish a step."
    )
}

/// Lantern's own identity on the Bus: what the person sends an agent from Lantern comes from it.
const YOU: &str = "lantern-you";

/// What the agent is told in its terminal, right after the message is in its Bus inbox: Bus
/// messages wait until an agent checks, and one waiting for you never would.
pub const NUDGE: &str = "I sent you a message on October Bus. Check your inbox (check_inbox) and do what it asks.";

/// Sends `text` to the agent with Bus id `to`, as the person using Lantern. Lantern's identity
/// registers (linked to the agent) for just this message: `october-bus agent run` hands its token
/// to `lantern-engine bus-post`, which posts it.
pub fn send_as_you(to: &str, text: &str) -> Result<()> {
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    ensure_ready()?;
    let dir = support_dir().join("bus");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("outgoing-{}.txt", uuid::Uuid::new_v4().simple()));
    crate::hooks::write_atomic(&file, text.as_bytes())?;
    // `LANTERN_ENGINE_BIN`: the engine to post with, for tests (whose own program is the test runner).
    let engine = match std::env::var_os("LANTERN_ENGINE_BIN") {
        Some(bin) => PathBuf::from(bin),
        None => std::env::current_exe()?,
    };
    let engine = engine.to_string_lossy().into_owned();
    let file_arg = file.to_string_lossy().into_owned();
    let out = bus(
        &[
            "agent",
            "run",
            "--scope",
            SCOPE,
            "--id",
            YOU,
            "--name",
            "You (from Lantern)",
            "--connect-to",
            to,
            "--",
            &engine,
            "bus-post",
            to,
            &file_arg,
        ],
        Duration::from_secs(12),
    );
    let _ = std::fs::remove_file(&file);
    let out = out?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        bail!("October Bus didn't take it{}", if why.is_empty() { String::new() } else { format!(": {why}") });
    }
    Ok(())
}

/// `lantern-engine bus-post <to> <file>`: posts the file's text to `to` with the agent token
/// `october-bus agent run` gives it.
pub fn post(to: &str, file: &str) -> Result<()> {
    let address = std::env::var("OCTOBER_BUS_ADDRESS").context("run under october-bus agent run")?;
    let token = std::env::var("OCTOBER_BUS_AGENT_TOKEN").context("run under october-bus agent run")?;
    let text = std::fs::read_to_string(file).context("reading the message")?;
    let body = format!("From the person using Lantern (reply in your own session: nobody reads this Bus identity's inbox):\n\n{text}");
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(8))).build().into();
    let message = serde_json::json!({"to": to, "body": body, "mode": "notify", "idempotencyKey": uuid::Uuid::new_v4().to_string()});
    let mut response = agent
        .post(format!("{}/v1/messages", address.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {token}"))
        .send_json(&message)
        .context("posting to October Bus")?;
    println!("{}", response.body_mut().read_to_string().unwrap_or_default());
    Ok(())
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

#[cfg(test)]
mod computer_tests {
    use super::*;

    fn driver() -> (String, Vec<String>) {
        ("/x/cua-driver".into(), vec!["mcp".into(), "--socket".into(), "/x/cua.sock".into()])
    }

    #[test]
    fn cua_driver_is_added_per_launch_without_the_bus() {
        let codex = attach(Kind::Codex, "id", "Codex · p", false, Some(&driver())).unwrap();
        assert!(codex.computer && !codex.on_bus);
        let joined = codex.args.join(" ");
        assert!(joined.contains("mcp_servers.cua_driver.command="), "{joined}");
        assert!(joined.contains("cua.sock"), "{joined}");
        assert!(!joined.contains("october_bus"), "{joined}");

        let goose = attach(Kind::Goose, "id", "Goose · p", false, Some(&driver())).unwrap();
        assert_eq!(goose.args[..2], ["session".to_string(), "--with-extension".to_string()]);

        // The October harness has no per-launch MCP: it's told the command instead.
        let october = attach(Kind::October, "id", "October · p", false, Some(&driver())).unwrap();
        assert!(!october.computer && october.wrap.is_empty());
        let none = attach(Kind::Codex, "id", "Codex · p", false, None).unwrap();
        assert!(none.args.is_empty() && !none.computer);
    }

    #[test]
    fn settings_files_keep_every_server() {
        let merged = merge_settings(
            Some(br#"{"theme":"dark","mcpServers":{"mine":{"command":"x"}}}"#),
            br#"{"mcpServers":{"october_bus":{"command":"b"},"cua_driver":{"command":"c"}}}"#,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&merged).unwrap();
        assert_eq!(v["theme"], "dark");
        for name in ["mine", "october_bus", "cua_driver"] {
            assert!(v["mcpServers"][name].is_object(), "{name}");
        }
    }
}
