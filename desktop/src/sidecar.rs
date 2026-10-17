//! Keep engine ownership, discovery and terminal delivery in the Rust sidecar.
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{
    Arc, Mutex, RwLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

pub struct Sidecar {
    child: Child,
    input: EngineSender,
    pub events: Receiver<Result<Value>>,
    latest: Arc<Mutex<HashMap<String, Value>>>,
    observer: Arc<Mutex<Option<Observer>>>,
    /// Makes the engine's command again, to restart it after it stops.
    command: Box<dyn Fn() -> Result<Command> + Send>,
}

#[derive(Clone)]
pub struct EngineSender {
    input: Arc<Mutex<ChildStdin>>,
    compatible: Arc<AtomicBool>,
}
impl EngineSender {
    fn send(&self, request: &Value) -> Result<()> {
        let mut input = self.input.lock().map_err(|_| anyhow::anyhow!("engine input unavailable"))?;
        writeln!(input, "{request}")?;
        Ok(input.flush()?)
    }
    // Serialize token writes and vault commands with sign-out. Check the epoch AFTER taking the
    // input lock: a late network response must never restore credentials after sign-out.
    /// `access`: the October access token and when it expires, or `None` after sign-out. The
    /// engine gets it for the phone and for October Harness sessions (`october.session`).
    pub fn credentials(
        &self,
        generation: u64,
        current: &AtomicU64,
        access: Option<(&str, std::time::Instant)>,
        after: impl FnOnce(),
    ) -> Result<bool> {
        let mut input = self.input.lock().map_err(|_| anyhow::anyhow!("engine input unavailable"))?;
        if current.load(Ordering::SeqCst) != generation {
            return Ok(false);
        }
        after();
        anyhow::ensure!(self.compatible.load(Ordering::SeqCst), "engine handshake unavailable");
        let requests = match access {
            Some((access, expires)) => {
                let left = expires.saturating_duration_since(std::time::Instant::now());
                let expires_at = (std::time::SystemTime::now() + left).duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                [
                    serde_json::json!({"type":"phone.token","accessToken":access}),
                    serde_json::json!({"type":"october.session","accessToken":access,"expiresAt":expires_at}),
                ]
            }
            None => [serde_json::json!({"type":"phone.stop"}), serde_json::json!({"type":"october.session"})],
        };
        for request in requests {
            writeln!(input, "{request}")?;
        }
        input.flush()?;
        Ok(true)
    }
}
struct Observer {
    tx: std::sync::mpsc::Sender<crate::os::Event>,
    snapshot: Arc<RwLock<String>>,
    notifications: bool,
    finished: bool,
    notices: crate::state::Notices,
}
impl Observer {
    fn update(&mut self, event: &Value) {
        let Some(agents) = event["agents"].as_array() else { return };
        let summary: Vec<Value> = agents
            .iter()
            .map(|agent| {
                serde_json::json!({
                    "id":agent["id"],"handle":agent["handle"],"project":agent["project"],"state":agent["state"]
                })
            })
            .collect();
        if let Ok(mut snapshot) = self.snapshot.write() {
            *snapshot = serde_json::to_string(&summary).unwrap_or_else(|_| "[]".into());
        }
        let changes = self.notices.transitions(agents, self.finished);
        if self.notifications {
            for agent in changes {
                let message = if text(&agent, "question").is_empty() { text(&agent, "lastMessage") } else { text(&agent, "question") };
                crate::os::notify(
                    self.tx.clone(),
                    text(&agent, "id").into(),
                    format!("{} · {}", text(&agent, "handle"), state_label(&agent)),
                    short(message, 240),
                );
            }
        }
    }
}

pub fn engine_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("LANTERN_ENGINE") {
        let path = PathBuf::from(path);
        if !path.is_file() {
            bail!("LANTERN_ENGINE does not point to a file: {}", path.display());
        }
        return Ok(path);
    }
    let filename = format!("lantern-engine{}", std::env::consts::EXE_SUFFIX);
    let bundled = std::env::current_exe()?.with_file_name(filename);
    if bundled.is_file() {
        return Ok(bundled);
    }
    bail!("lantern-engine sidecar is missing; run scripts/build-app.sh or set LANTERN_ENGINE")
}

/// Where the engine's log goes: Lantern's own stderr, or on Windows (where the app has no
/// console) a file in its data folder. The last run's log is kept as `engine-previous.log`, so
/// why an engine stopped survives the restart. An update's `--check-sidecar` leaves both alone.
fn engine_log() -> Stdio {
    #[cfg(windows)]
    if !std::env::args().any(|arg| arg == "--check-sidecar") {
        let path = crate::settings::data_dir().join("logs").join("engine.log");
        let _ = std::fs::rename(&path, path.with_file_name("engine-previous.log"));
        let file = std::fs::create_dir_all(path.parent().unwrap()).and_then(|_| std::fs::File::create(&path));
        if let Ok(file) = file {
            return file.into();
        }
    }
    Stdio::inherit()
}

impl Sidecar {
    pub fn start() -> Result<Self> {
        Self::with_terminal("")
    }
    pub fn with_terminal(terminal: &str) -> Result<Self> {
        let terminal = terminal.trim().to_string();
        let command = move || -> Result<Command> {
            let path = engine_path()?;
            let mut command = crate::os::command(&path);
            if !terminal.is_empty() {
                command.env("LANTERN_TERMINAL", &terminal);
            }
            command.arg("serve");
            Ok(command)
        };
        Self::spawn(Box::new(command)).context("starting Lantern's engine")
    }
    fn spawn(command: Box<dyn Fn() -> Result<Command> + Send>) -> Result<Self> {
        let latest = Arc::new(Mutex::new(HashMap::new()));
        let observer = Arc::new(Mutex::new(None::<Observer>));
        let compatible = Arc::new(AtomicBool::new(false));
        let (child, stdin, events) = Self::run(&command, &latest, &observer, &compatible)?;
        let input = EngineSender { input: Arc::new(Mutex::new(stdin)), compatible };
        Ok(Self { child, input, events, latest, observer, command })
    }
    /// Starts the engine and the thread reading its events.
    fn run(
        command: &dyn Fn() -> Result<Command>,
        latest: &Arc<Mutex<HashMap<String, Value>>>,
        observer: &Arc<Mutex<Option<Observer>>>,
        compatible: &Arc<AtomicBool>,
    ) -> Result<(Child, ChildStdin, Receiver<Result<Value>>)> {
        let mut child = command()?.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(engine_log()).spawn()?;
        let stdin = child.stdin.take().context("engine stdin missing")?;
        let output = child.stdout.take().context("engine stdout missing")?;
        let (tx, events) = mpsc::channel();
        let reader_latest = latest.clone();
        let reader_observer = observer.clone();
        let compatible = compatible.clone();
        compatible.store(false, Ordering::SeqCst);
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let event = line.map_err(anyhow::Error::from).and_then(|l| serde_json::from_str(&l).context("invalid engine response"));
                if let Ok(value) = &event {
                    let kind = text(value, "type");
                    if kind == "hello" {
                        compatible.store(value["protocol"] == 3, Ordering::SeqCst);
                    }
                    if kind == "snapshot"
                        && let Ok(mut observer) = reader_observer.lock()
                        && let Some(observer) = observer.as_mut()
                    {
                        observer.update(value);
                    }
                    if matches!(kind, "snapshot" | "installed" | "october" | "phone")
                        && let Ok(mut latest) = reader_latest.lock()
                    {
                        latest.insert(kind.to_string(), value.clone());
                        continue;
                    }
                }
                if tx.send(event).is_err() {
                    return;
                }
            }
            compatible.store(false, Ordering::SeqCst);
            let _ = tx.send(Err(anyhow::anyhow!("engine disconnected; close and reopen Lantern")));
        });
        Ok((child, stdin, events))
    }
    /// Starts the engine again after it stopped. Senders already handed out (sign-in's) reach the
    /// new one; it says hello like a fresh start.
    pub fn restart(&mut self) -> Result<()> {
        self.stop();
        let (child, stdin, events) = Self::run(&self.command, &self.latest, &self.observer, &self.input.compatible)?;
        *self.input.input.lock().map_err(|_| anyhow::anyhow!("engine input unavailable"))? = stdin;
        self.child = child;
        self.events = events;
        Ok(())
    }

    pub fn send(&mut self, request: &Value) -> Result<()> {
        self.input.send(request)
    }
    /// Stops the engine now (uninstalling: it holds files in Lantern's data folder open). Sessions
    /// Lantern started run in processes of their own and keep running.
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    pub fn sender(&self) -> EngineSender {
        self.input.clone()
    }
    pub fn take_latest(&self) -> Vec<Value> {
        self.latest.lock().map(|mut values| values.drain().map(|(_, value)| value).collect()).unwrap_or_default()
    }
    pub fn observe(
        &self,
        tx: std::sync::mpsc::Sender<crate::os::Event>,
        snapshot: Arc<RwLock<String>>,
        notifications: bool,
        finished: bool,
    ) {
        if let Ok(mut observer) = self.observer.lock() {
            let mut next = Observer { tx, snapshot, notifications, finished, notices: Default::default() };
            if let Ok(latest) = self.latest.lock()
                && let Some(snapshot) = latest.get("snapshot")
            {
                next.update(snapshot);
            }
            *observer = Some(next);
        }
    }
    pub fn notification_preferences(&self, enabled: bool, finished: bool) {
        if let Ok(mut observer) = self.observer.lock()
            && let Some(observer) = observer.as_mut()
        {
            observer.notifications = enabled;
            observer.finished = finished;
        }
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
pub fn test_sender() -> (Sidecar, EngineSender) {
    tests::test_sender()
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

/// The agent's state the way the Mac names it ("Needs you", "Your turn"…).
fn state_label(agent: &Value) -> &'static str {
    serde_json::from_value::<crate::model::AgentState>(agent["state"].clone()).unwrap_or_default().label()
}

fn short(text: &str, count: usize) -> String {
    let mut result: String = text.chars().take(count).collect();
    if text.chars().count() > count {
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    fn echo() -> Sidecar {
        if cfg!(windows) {
            let script = "while(($l=[Console]::In.ReadLine()) -ne $null){[Console]::Out.WriteLine($l);[Console]::Out.Flush()}";
            return Sidecar::spawn(Box::new(move || {
                let mut command = Command::new("powershell");
                command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
                Ok(command)
            }))
            .unwrap();
        }
        Sidecar::spawn(Box::new(|| Ok(Command::new("cat")))).unwrap()
    }
    #[test]
    fn locked_ui_coalesces_snapshots_and_keeps_reply_results() {
        let mut engine = echo();
        let snapshot = Arc::new(RwLock::new("[]".into()));
        let (tx, _events) = mpsc::channel();
        engine.observe(tx, snapshot.clone(), false, false);
        engine.send(&json!({"type":"hello","protocol":3})).unwrap();
        for index in 0..400 {
            engine
                .send(&json!({"type":"snapshot","agents":[{"id":"a","handle":index.to_string(),"state":"working","question":"private"}]}))
                .unwrap();
        }
        engine.send(&json!({"type":"reply","requestId":"reply-1","ok":true})).unwrap();
        // No UI polling while the sidecar reader receives the entire burst.
        assert_eq!(engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap()["type"], "hello");
        assert_eq!(engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap()["requestId"], "reply-1");
        let bridged: Value = serde_json::from_str(&snapshot.read().unwrap()).unwrap();
        assert_eq!(bridged[0]["handle"], "399");
        assert!(bridged[0].get("question").is_none());
        let latest = engine.take_latest();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0]["agents"][0]["handle"], "399");
        assert!(engine.take_latest().is_empty());
    }
    #[test]
    fn stale_auth_cannot_restore_a_signed_out_engine_or_vault() {
        let mut engine = echo();
        engine.send(&json!({"type":"hello","protocol":3})).unwrap();
        engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        let sender = engine.sender();
        let current = AtomicU64::new(2);
        assert!(!sender.credentials(1, &current, Some(("stale", std::time::Instant::now())), || panic!("stale vault write")).unwrap());
        let mut cleared = false;
        assert!(sender.credentials(2, &current, None, || cleared = true).unwrap());
        assert!(cleared);
        let event = engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        assert_eq!(event["type"], "phone.stop");
        assert!(event.get("accessToken").is_none());
        let event = engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        assert_eq!(event["type"], "october.session");
        assert!(event.get("accessToken").is_none(), "signing out clears October Harness's sign-in too");
    }
    #[test]
    fn sign_in_gives_the_engine_the_phone_and_october_tokens() {
        let mut engine = echo();
        engine.send(&json!({"type":"hello","protocol":3})).unwrap();
        engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        let expires = std::time::Instant::now() + Duration::from_secs(3600);
        assert!(engine.sender().credentials(1, &AtomicU64::new(1), Some(("tok", expires)), || {}).unwrap());
        let phone = engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        assert_eq!((phone["type"].as_str(), phone["accessToken"].as_str()), (Some("phone.token"), Some("tok")));
        let october = engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        assert_eq!((october["type"].as_str(), october["accessToken"].as_str()), (Some("october.session"), Some("tok")));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let at = october["expiresAt"].as_u64().unwrap();
        assert!((now + 3590..=now + 3610).contains(&at), "expiresAt is epoch seconds");
    }
    #[test]
    fn credentials_require_a_matching_protocol() {
        let mut engine = echo();
        engine.send(&json!({"type":"hello","protocol":99})).unwrap();
        engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        assert!(engine.sender().credentials(1, &AtomicU64::new(1), Some(("test", std::time::Instant::now())), || {}).is_err());
        engine.send(&json!({"type":"marker"})).unwrap();
        assert_eq!(engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap()["type"], "marker");
    }
    #[test]
    fn a_restarted_engine_is_reached_through_senders_handed_out_before() {
        let mut engine = echo();
        let sender = engine.sender();
        engine.restart().unwrap();
        engine.send(&json!({"type":"hello","protocol":3})).unwrap();
        assert_eq!(engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap()["type"], "hello");
        assert!(sender.credentials(1, &AtomicU64::new(1), None, || {}).unwrap(), "sign-in's sender reaches the new engine");
        assert_eq!(engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap()["type"], "phone.stop");
    }
    pub(super) fn test_sender() -> (Sidecar, EngineSender) {
        let mut engine = echo();
        engine.send(&json!({"type":"hello","protocol":3})).unwrap();
        engine.events.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        let sender = engine.sender();
        (engine, sender)
    }
}
