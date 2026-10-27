//! Cards: what an agent Lantern started asks of the person (a question, a step only they can do)
//! and what it reports (a status, its steps, its result). They're kept in a small private
//! database, so they outlive the agent's process and the engine.
//!
//! Each launch is registered before it starts (`register`): it gets an id and a secret key, and
//! its agent gets `lantern-engine cards-mcp <id>`, an MCP server (`mcp.rs`) that reads the key from
//! Lantern's private folder and talks to this engine over a private local connection (`ipc.rs`).
//! The person answers in the app (`answer`). An answer goes only to the tool call still waiting
//! for it: the waiting call renews a short lease while it waits, and a card whose call stopped
//! waiting (the agent was interrupted, exited, or its tool call timed out) is closed, never typed
//! in later. Agents can't answer cards: the connection only creates them and waits.
//!
//! Anyone running programs as this user can read the key and the database; the key ties a card to
//! its launch, it doesn't protect cards from the agent's own shell.

pub mod ipc;
pub mod mcp;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::hooks::support_dir;
use crate::model::{Agent, Kind, Route};

/// How long a question or hand-off waits for the person before it's closed unanswered. Each
/// agent's tool timeout is set a little longer (`TOOL_TIMEOUT`), so the agent hears "no answer"
/// from Lantern rather than a timeout.
pub const WAIT_LIMIT: Duration = Duration::from_secs(25 * 60);
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(27 * 60);
/// A waiting call checks in at least this often; one that stops is treated as gone.
const LEASE_MS: i64 = 20_000;
/// One `wait` request holds this long before saying "still waiting".
const POLL: Duration = Duration::from_secs(5);
/// Cards one launch can have waiting at once, and in all.
const MAX_WAITING: i64 = 4;
const MAX_CARDS: i64 = 200;
/// Launches kept, newest first; older ones are deleted with their cards.
const KEEP: i64 = 200;
/// What the app is sent: launches from the last three days, or with a card waiting.
const SHOWN_FOR_MS: i64 = 3 * 24 * 3600 * 1000;
const SHOWN_MAX: i64 = 40;
const SHOWN_CARDS: i64 = 30;

/// Epoch milliseconds, as SQLite stores them.
fn now() -> i64 {
    crate::hooks::now_ms() as i64
}

pub fn dir() -> PathBuf {
    support_dir().join("cards")
}

fn key_path(launch: &str) -> PathBuf {
    dir().join("keys").join(launch)
}

/// The key a launch's MCP server presents, from Lantern's private folder.
pub fn read_key(launch: &str) -> Result<String> {
    if !valid_id(launch) {
        bail!("not a launch id");
    }
    Ok(std::fs::read_to_string(key_path(launch)).context("this launch isn't registered with Lantern")?.trim().to_string())
}

fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|u| u.hyphenated().to_string() == id)
}

fn digest(key: &str) -> Vec<u8> {
    Sha256::digest(key.as_bytes()).to_vec()
}

/// Which agents get the card tools: those whose MCP tool timeout Lantern can raise past
/// `WAIT_LIMIT` per launch (Claude Code `timeout`, Codex `tool_timeout_sec`, October's MCP
/// adapter `requestTimeoutMs`).
pub fn supported(kind: Kind) -> bool {
    matches!(kind, Kind::Claude | Kind::Codex | Kind::October)
}

/// Why the person has to do a step themselves. Lantern words each one; the agent's own words are
/// shown as the agent's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    SignIn,
    Verification,
    Payment,
    Identity,
    Physical,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Done,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StepIn {
    pub id: String,
    pub title: String,
}

/// What a launch's MCP server asks of the engine.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Op {
    /// The server started (the agent has the tools).
    Hello,
    Guide {
        operation_id: String,
        question: String,
    },
    SuggestRoutine {
        operation_id: String,
        name: String,
        prompt: String,
    },
    Ask {
        operation_id: String,
        question: String,
        details: Option<String>,
        #[serde(default)]
        options: Vec<String>,
    },
    HandOff {
        operation_id: String,
        reason: Reason,
        step: String,
        why: Option<String>,
        #[serde(default)]
        instructions: Vec<String>,
        url: Option<String>,
    },
    /// Waits up to `POLL` for the card's answer, renewing its lease.
    Wait {
        card: String,
    },
    /// The answer reached the server, which is handing it to the agent.
    Ack {
        card: String,
    },
    /// The agent stopped waiting (it cancelled the tool call).
    Cancel {
        card: String,
    },
    Status {
        text: String,
    },
    Steps {
        steps: Vec<StepIn>,
    },
    StepDone {
        id: String,
    },
    Result {
        operation_id: String,
        summary: String,
        outcome: Outcome,
        #[serde(default)]
        files: Vec<String>,
    },
}

/// What the person chose: one of the question's options, their own words, or, for a hand-off,
/// whether they did the step.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
}

/// Where a card stands. `Answered` is saved but not yet handed to the agent; `Unconfirmed` was
/// saved but the agent's call went away before it took it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Waiting,
    Answered,
    Delivered,
    Unconfirmed,
    Expired,
    Stopped,
    Cancelled,
    Shown,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Waiting => "waiting",
            State::Answered => "answered",
            State::Delivered => "delivered",
            State::Unconfirmed => "unconfirmed",
            State::Expired => "expired",
            State::Stopped => "stopped",
            State::Cancelled => "cancelled",
            State::Shown => "shown",
        }
    }

    fn parse(s: &str) -> State {
        match s {
            "waiting" => State::Waiting,
            "answered" => State::Answered,
            "delivered" => State::Delivered,
            "unconfirmed" => State::Unconfirmed,
            "expired" => State::Expired,
            "cancelled" => State::Cancelled,
            "shown" => State::Shown,
            _ => State::Stopped,
        }
    }
}

/// The engine's card service: one database, used from the main loop and the connection threads.
pub struct Cards {
    db: Mutex<Connection>,
    /// Bumped whenever a card is answered or closed, for waiting calls.
    resolved: (Mutex<u64>, Condvar),
    /// Told after every change, so the app gets a fresh `cards` event.
    on_change: Mutex<Option<Box<dyn Fn() + Send>>>,
}

fn bounded(label: &str, s: &str, max: usize) -> Result<String> {
    let s = s.trim();
    if s.chars().count() > max {
        bail!("{label} is too long (at most {max} characters)");
    }
    if s.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        bail!("{label} has control characters");
    }
    Ok(s.to_string())
}

fn required(label: &str, s: &str, max: usize) -> Result<String> {
    let s = bounded(label, s, max)?;
    if s.is_empty() {
        bail!("{label} is empty");
    }
    Ok(s)
}

/// A web address a hand-off may show: http(s) only, no user name or password in it.
pub fn checked_url(raw: &str, https_only: bool) -> Result<String> {
    let url = url::Url::parse(raw.trim()).context("url isn't a web address")?;
    let ok_scheme = url.scheme() == "https" || (!https_only && url.scheme() == "http");
    if !ok_scheme || url.host_str().is_none_or(str::is_empty) {
        bail!("url must be an {} web address", if https_only { "https" } else { "http or https" });
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("url can't contain a user name or password");
    }
    if url.as_str().len() > 2000 {
        bail!("url is too long");
    }
    Ok(url.into())
}

/// A file or folder a result names: a full path to something that exists, as it really is
/// (symbolic links followed).
fn checked_file(raw: &str) -> Result<Value> {
    let path = Path::new(raw.trim());
    if !path.is_absolute() {
        bail!("files must be full paths ({raw} isn't)");
    }
    let real = path.canonicalize().with_context(|| format!("{raw} doesn't exist"))?;
    let meta = std::fs::metadata(&real)?;
    let name = real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| real.to_string_lossy().into_owned());
    Ok(json!({"path": real.to_string_lossy(), "name": name, "folder": meta.is_dir()}))
}

impl Cards {
    pub fn open(path: &Path) -> Result<Cards> {
        let db = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS launches(
               id TEXT PRIMARY KEY, key BLOB NOT NULL, kind TEXT NOT NULL, cwd TEXT NOT NULL, title TEXT NOT NULL,
               session TEXT, agent TEXT, tools INTEGER NOT NULL DEFAULT 0,
               created INTEGER NOT NULL, ended INTEGER, status TEXT, steps TEXT, revision INTEGER NOT NULL DEFAULT 1);
             CREATE TABLE IF NOT EXISTS cards(
               id TEXT PRIMARY KEY, launch TEXT NOT NULL REFERENCES launches(id) ON DELETE CASCADE,
               kind TEXT NOT NULL, body TEXT NOT NULL, state TEXT NOT NULL, answer TEXT, answered_by TEXT,
               created INTEGER NOT NULL, resolved INTEGER, delivered INTEGER, lease INTEGER, expires INTEGER,
               revision INTEGER NOT NULL DEFAULT 1);
             CREATE INDEX IF NOT EXISTS cards_launch ON cards(launch, created);",
        )?;
        // Calls that were waiting when the engine stopped get a moment to come back.
        db.execute("UPDATE cards SET lease = MAX(COALESCE(lease, 0), ?1) WHERE state IN ('waiting', 'answered')", [now() + LEASE_MS])?;
        Ok(Cards { db: Mutex::new(db), resolved: (Mutex::new(0), Condvar::new()), on_change: Mutex::new(None) })
    }

    /// The engine's own, in Lantern's private folder.
    pub fn open_default() -> Result<Cards> {
        crate::bus::create_private_dir(&dir())?;
        crate::bus::create_private_dir(&dir().join("keys"))?;
        Cards::open(&dir().join("cards.db"))
    }

    pub fn on_change(&self, f: impl Fn() + Send + 'static) {
        *self.on_change.lock().unwrap() = Some(Box::new(f));
    }

    fn changed(&self) {
        if let Some(f) = self.on_change.lock().unwrap().as_ref() {
            f();
        }
    }

    fn wake(&self) {
        let (lock, cv) = &self.resolved;
        *lock.lock().unwrap() += 1;
        cv.notify_all();
    }

    /// A new launch, before it starts: its id, and the key its MCP server will present (also
    /// written to Lantern's private folder, where the server reads it). `key_dir` is `None` in
    /// tests.
    pub fn register(
        &self,
        kind: Kind,
        cwd: &Path,
        title: &str,
        key_dir: Option<&Path>,
        operation: Option<&str>,
    ) -> Result<(String, String)> {
        let id = operation.map(str::to_string).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if !valid_id(&id) {
            bail!("Invalid launch operation identity");
        }
        let db = self.db.lock().unwrap();
        if db.query_row("SELECT 1 FROM launches WHERE id=?1", [&id], |_| Ok(())).optional()?.is_some() {
            bail!("This launch was already admitted; reconcile it before starting again");
        }
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
        let key: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        if let Some(dir) = key_dir {
            let path = dir.join(&id);
            crate::hooks::write_atomic(&path, key.as_bytes())?;
            crate::platform::set_mode(&path, 0o600)?;
        }
        let title = crate::model::truncate(title.lines().next().unwrap_or_default(), 80);
        db.execute(
            "INSERT INTO launches(id, key, kind, cwd, title, created) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, digest(&key), kind.as_str(), cwd.to_string_lossy(), title, now()],
        )?;
        // Old launches go, with their cards and keys.
        let old: Vec<String> = db
            .prepare("SELECT id FROM launches WHERE ended IS NOT NULL AND NOT EXISTS(SELECT 1 FROM cards WHERE launch=launches.id AND state IN ('waiting','answered','unconfirmed')) ORDER BY created DESC LIMIT -1 OFFSET ?1")?
            .query_map([KEEP], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for gone in old {
            db.execute("DELETE FROM launches WHERE id = ?1", [&gone])?;
            if let Some(dir) = key_dir {
                let _ = std::fs::remove_file(dir.join(&gone));
            }
        }
        Ok((id, key))
    }

    pub fn cancel_waits(&self) -> Result<()> {
        self.db.lock().unwrap().execute("UPDATE cards SET state=CASE WHEN state='answered' THEN 'unconfirmed' ELSE 'cancelled' END,revision=revision+1,resolved=?1 WHERE state IN ('waiting','answered')",[now()])?;
        self.wake();
        self.changed();
        Ok(())
    }
    pub fn session_for_control(&self, launch: &str) -> Result<String> {
        self.db
            .lock()
            .unwrap()
            .query_row("SELECT session FROM launches WHERE id=?1 AND ended IS NULL", [launch], |r| r.get(0))
            .context("The owned session is not available")
    }
    pub fn session_stopped(&self, launch: &str) -> Result<()> {
        self.db.lock().unwrap().execute("UPDATE launches SET ended=?2 WHERE id=?1", params![launch, now()])?;
        self.changed();
        Ok(())
    }

    /// The launch's terminal session (tmux or Lantern's own), which ties it to its agent.
    pub fn started(&self, launch: &str, session: Option<&str>) -> Result<()> {
        self.db.lock().unwrap().execute("UPDATE launches SET session = ?2 WHERE id = ?1", params![launch, session])?;
        self.changed();
        Ok(())
    }

    /// A launch that didn't start.
    pub fn discard(&self, launch: &str, key_dir: Option<&Path>) {
        let _ = self.db.lock().unwrap().execute("DELETE FROM launches WHERE id = ?1", [launch]);
        if let Some(dir) = key_dir {
            let _ = std::fs::remove_file(dir.join(launch));
        }
    }

    /// Tie launches to discovered agents. Missing discovery is not proof of session exit.
    pub fn bind(&self, agents: &[Agent]) -> bool {
        let session_of = |a: &Agent| match &a.route {
            Route::Pty { session } => Some(session.clone()),
            _ => a
                .tmux
                .as_ref()
                .filter(|t| t.socket.as_deref() == Some(crate::launch::LANTERN_SOCKET))
                .map(|t| t.target.split(':').next().unwrap_or_default().to_string()),
        };
        let db = self.db.lock().unwrap();
        let Ok(mut open) = db.prepare("SELECT id, session, agent FROM launches WHERE ended IS NULL AND session IS NOT NULL") else {
            return false;
        };
        let rows: Vec<(String, String, Option<String>)> =
            open.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map(|rows| rows.flatten().collect()).unwrap_or_default();
        drop(open);
        let mut changed = false;
        for (id, session, bound) in rows {
            let running = agents.iter().find(|a| a.live && session_of(a).as_deref() == Some(session.as_str()));
            match (running, bound) {
                (Some(agent), bound) if bound.as_deref() != Some(agent.id.as_str()) => {
                    changed |= db.execute("UPDATE launches SET agent = ?2 WHERE id = ?1", params![id, agent.id]).is_ok_and(|n| n > 0);
                }
                _ => {}
            }
        }
        drop(db);
        if changed {
            self.changed();
        }
        changed
    }

    /// Probe off the scan thread. A failed/unknown probe keeps the launch open.
    fn refresh_sessions(&self, alive: impl Fn(&str) -> Option<bool>) {
        let rows: Vec<(String, String)> = {
            let db = self.db.lock().unwrap();
            let Ok(mut q) = db.prepare("SELECT id,session FROM launches WHERE ended IS NULL AND session IS NOT NULL AND created<?1") else {
                return;
            };
            let Ok(rows) = q.query_map([now() - 30_000], |r| Ok((r.get(0)?, r.get(1)?))) else { return };
            rows.flatten().collect()
        };
        for (id, session) in rows {
            if alive(&session) == Some(false) {
                let _ = self.session_stopped(&id);
            }
        }
    }

    /// Closes cards whose call stopped waiting, or that waited too long. Returns whether any were.
    pub fn sweep(&self) -> bool {
        let now = now();
        let db = self.db.lock().unwrap();
        let mut n = db
            .execute(
                "UPDATE cards SET state = 'expired', resolved = ?1, revision = revision + 1 WHERE state = 'waiting' AND expires < ?1",
                [now],
            )
            .unwrap_or(0);
        n += db
            .execute(
                "UPDATE cards SET state = 'stopped', resolved = ?1, revision = revision + 1 WHERE state = 'waiting' AND lease < ?1",
                [now],
            )
            .unwrap_or(0);
        n += db
            .execute("UPDATE cards SET state = 'unconfirmed', revision = revision + 1 WHERE state = 'answered' AND lease < ?1", [now])
            .unwrap_or(0);
        drop(db);
        if n > 0 {
            self.wake();
            self.changed();
        }
        n > 0
    }

    /// The launch a request comes from, if its key is right.
    fn authorize(&self, launch: &str, key: &str) -> Result<()> {
        use subtle::ConstantTimeEq;
        let stored: Option<Vec<u8>> =
            self.db.lock().unwrap().query_row("SELECT key FROM launches WHERE id = ?1", [launch], |r| r.get(0)).optional()?;
        match stored {
            Some(stored) if bool::from(stored.ct_eq(&digest(key))) => Ok(()),
            _ => bail!("not allowed"),
        }
    }

    /// Handles one request from a launch's MCP server.
    pub fn handle(&self, launch: &str, key: &str, op: Op) -> Result<Value> {
        self.authorize(launch, key)?;
        if crate::computer::held() && !matches!(op, Op::Wait { .. } | Op::Ack { .. } | Op::Cancel { .. } | Op::Hello) {
            bail!("App access is paused. Wait for the person to continue.");
        }
        let now = now();
        let result = match op {
            Op::Guide { operation_id, question } => {
                let question = required("question", &question, 200)?;
                let card = self.add(launch, &operation_id, "handOff", json!({"reason":"other","step":question,"guide":true}), true)?;
                json!({"card":card})
            }
            Op::SuggestRoutine { operation_id, name, prompt } => {
                let name = required("name", &name, 120)?;
                let prompt = required("prompt", &prompt, 8000)?;
                let (kind, cwd): (String, String) =
                    self.db
                        .lock()
                        .unwrap()
                        .query_row("SELECT kind,cwd FROM launches WHERE id=?1", [launch], |r| Ok((r.get(0)?, r.get(1)?)))?;
                let draft = json!({"name":name,"prompt":prompt,"cwd":cwd,"kind":kind,"timezone":"UTC","cadence":"daily","hour":9,"minute":0,"weekday":0,"preferences":""});
                let card = self.add(launch, &operation_id, "routine", json!({"routine":draft}), false)?;
                json!({"card":card})
            }
            Op::Hello => {
                self.db.lock().unwrap().execute("UPDATE launches SET tools = 1 WHERE id = ?1", [launch])?;
                json!({})
            }
            Op::Ask { operation_id, question, details, options } => {
                let question = required("question", &question, 500)?;
                let details = details.map(|d| bounded("details", &d, 4000)).transpose()?.filter(|d| !d.is_empty());
                if options.len() > 6 {
                    bail!("at most 6 options");
                }
                let mut seen = Vec::new();
                for o in &options {
                    let o = required("an option", o, 80)?;
                    if seen.contains(&o) {
                        bail!("options must be different");
                    }
                    seen.push(o);
                }
                let card =
                    self.add(launch, &operation_id, "ask", json!({"question": question, "details": details, "options": seen}), true)?;
                json!({"card": card})
            }
            Op::HandOff { operation_id, reason, step, why, instructions, url } => {
                let step = required("step", &step, 200)?;
                let why = why.map(|w| bounded("why", &w, 600)).transpose()?.filter(|w| !w.is_empty());
                if instructions.len() > 3 {
                    bail!("at most 3 instructions");
                }
                let instructions: Vec<String> = instructions.iter().map(|i| required("an instruction", i, 300)).collect::<Result<_>>()?;
                let url = url.filter(|u| !u.trim().is_empty()).map(|u| checked_url(&u, reason == Reason::SignIn)).transpose()?;
                let card = self.add(
                    launch,
                    &operation_id,
                    "handOff",
                    json!({"reason": reason, "step": step, "why": why, "instructions": instructions, "url": url}),
                    true,
                )?;
                json!({"card": card})
            }
            Op::Wait { card } => return self.wait(launch, &card),
            Op::Ack { card } => {
                let db = self.db.lock().unwrap();
                db.execute(
                    "UPDATE cards SET state = 'delivered', delivered = ?3, revision = revision + 1
                     WHERE id = ?1 AND launch = ?2 AND state IN ('answered', 'unconfirmed')",
                    params![card, launch, now],
                )?;
                json!({})
            }
            Op::Cancel { card } => {
                self.db.lock().unwrap().execute(
                    "UPDATE cards SET state = CASE WHEN state='answered' THEN 'unconfirmed' ELSE 'cancelled' END, resolved = ?3, revision = revision + 1 WHERE id = ?1 AND launch = ?2 AND state IN ('waiting','answered')",
                    params![card, launch, now],
                )?;
                self.wake();
                json!({})
            }
            Op::Status { text } => {
                let text = bounded("status", &text, 200)?;
                let text = (!text.is_empty()).then_some(text);
                self.db
                    .lock()
                    .unwrap()
                    .execute("UPDATE launches SET status = ?2, revision = revision + 1 WHERE id = ?1", params![launch, text])?;
                json!({})
            }
            Op::Steps { steps } => {
                if steps.len() > 12 {
                    bail!("at most 12 steps");
                }
                let db = self.db.lock().unwrap();
                let old: Option<String> = db.query_row("SELECT steps FROM launches WHERE id = ?1", [launch], |r| r.get(0))?;
                let old: Vec<Value> = old.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
                let mut ids = Vec::new();
                let mut list = Vec::new();
                for s in steps {
                    let id = required("a step id", &s.id, 40)?;
                    if ids.contains(&id) {
                        bail!("step ids must be different");
                    }
                    // A step already done stays done.
                    let done = old.iter().any(|o| o["id"] == id.as_str() && o["done"] == true);
                    list.push(json!({"id": id, "title": required("a step title", &s.title, 120)?, "done": done}));
                    ids.push(id);
                }
                db.execute(
                    "UPDATE launches SET steps = ?2, revision = revision + 1 WHERE id = ?1",
                    params![launch, Value::from(list).to_string()],
                )?;
                json!({})
            }
            Op::StepDone { id } => {
                let db = self.db.lock().unwrap();
                let old: Option<String> = db.query_row("SELECT steps FROM launches WHERE id = ?1", [launch], |r| r.get(0))?;
                let mut steps: Vec<Value> = old.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
                let step = steps.iter_mut().find(|s| s["id"] == id.as_str()).context("no step with that id; set_steps first")?;
                step["done"] = json!(true);
                db.execute(
                    "UPDATE launches SET steps = ?2, revision = revision + 1 WHERE id = ?1",
                    params![launch, Value::from(steps).to_string()],
                )?;
                json!({})
            }
            Op::Result { operation_id, summary, outcome, files } => {
                let summary = required("summary", &summary, 4000)?;
                if files.len() > 10 {
                    bail!("at most 10 files");
                }
                let files: Vec<Value> = files.iter().map(|f| checked_file(f)).collect::<Result<_>>()?;
                let card =
                    self.add(launch, &operation_id, "result", json!({"summary": summary, "outcome": outcome, "files": files}), false)?;
                json!({"card": card})
            }
        };
        self.changed();
        Ok(result)
    }

    fn add(&self, launch: &str, operation_id: &str, kind: &str, body: Value, waits: bool) -> Result<String> {
        let db = self.db.lock().unwrap();
        if crate::computer::held() {
            bail!("App access is paused; no question was created.");
        }
        if !valid_id(operation_id) {
            bail!("Invalid card operation id");
        }
        let encoded_body = serde_json::to_string(&body)?;
        let existing: Option<(String, String, String)> = db
            .query_row("SELECT launch,kind,body FROM cards WHERE id=?1", [operation_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        if let Some((owner, old_kind, old_body)) = existing {
            if owner != launch || old_kind != kind || old_body != encoded_body {
                bail!("This operation already names a different card");
            }
            return Ok(operation_id.to_string());
        }
        let (waiting, all): (i64, i64) =
            db.query_row("SELECT COUNT(*) FILTER (WHERE state = 'waiting'), COUNT(*) FROM cards WHERE launch = ?1", [launch], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        if waits && waiting >= MAX_WAITING {
            bail!("{MAX_WAITING} cards are already waiting for the person; wait for those first");
        }
        if all >= MAX_CARDS {
            bail!("this session has made too many cards");
        }
        let id = operation_id.to_string();
        let now = now();
        let (state, lease, expires) = if waits {
            (State::Waiting, Some(now + LEASE_MS), Some(now + WAIT_LIMIT.as_millis() as i64))
        } else {
            (State::Shown, None, None)
        };
        db.execute(
            "INSERT INTO cards(id, launch, kind, body, state, created, lease, expires) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, launch, kind, encoded_body, state.as_str(), now, lease, expires],
        )?;
        Ok(id)
    }

    fn card_state(&self, launch: &str, card: &str) -> Result<(State, Option<String>, i64)> {
        let row: Option<(String, Option<String>, Option<i64>)> = self
            .db
            .lock()
            .unwrap()
            .query_row("SELECT state, answer, expires FROM cards WHERE id = ?1 AND launch = ?2", params![card, launch], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let (state, answer, expires) = row.context("no such card")?;
        Ok((State::parse(&state), answer, expires.unwrap_or(0)))
    }

    /// Waits up to `POLL` for the card to be answered or closed, renewing its lease.
    fn wait(&self, launch: &str, card: &str) -> Result<Value> {
        let until = std::time::Instant::now() + POLL;
        loop {
            self.sweep();
            let seen = *self.resolved.0.lock().unwrap();
            let now = now();
            self.db.lock().unwrap().execute(
                "UPDATE cards SET lease = ?3 WHERE id = ?1 AND launch = ?2 AND state IN ('waiting', 'answered') AND lease >= ?4",
                params![card, launch, now + LEASE_MS, now],
            )?;
            let (state, answer, expires) = self.card_state(launch, card)?;
            if state == State::Waiting && expires <= now {
                self.sweep();
                continue;
            }
            if state != State::Waiting {
                let _dispatch_guard = crate::computer::action_guard();
                let answer: Option<Value> = if state == State::Answered && !crate::computer::held() {
                    answer.and_then(|a| serde_json::from_str(&a).ok())
                } else {
                    None
                };
                return Ok(json!({"state": state, "answer": answer}));
            }
            let left = until.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Ok(json!({"state": state}));
            }
            let (lock, cv) = &self.resolved;
            let guard = lock.lock().unwrap();
            if *guard == seen {
                let _ = cv.wait_timeout(guard, left.min(Duration::from_millis((expires - now).max(1) as u64))).unwrap();
            }
        }
    }

    /// The person's answer to a card (from the app). Saved only if the card is still waiting, at
    /// the revision the person saw, and the answer fits it; then the waiting call takes it.
    pub fn answer(&self, card: &str, revision: i64, answer: &Answer, by: &str) -> Result<()> {
        let _dispatch_guard = crate::computer::action_guard();
        if crate::computer::held() {
            bail!("App access is paused; this answer was not queued.");
        }
        let db = self.db.lock().unwrap();
        let row: Option<(String, String, String, i64, Option<i64>)> = db
            .query_row("SELECT kind, body, state, revision, lease FROM cards WHERE id = ?1", [card], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .optional()?;
        let Some((kind, body, state, current, lease)) = row else { bail!("This card isn't here any more.") };
        if State::parse(&state) != State::Waiting || lease.is_some_and(|l| l < now()) {
            bail!("The agent isn't waiting for this any more.");
        }
        if current != revision {
            bail!("This card changed; look at it again.");
        }
        let body: Value = serde_json::from_str(&body)?;
        let text = answer.text.as_deref().map(|t| required("Your answer", t, 2000)).transpose()?;
        let checked = match kind.as_str() {
            "ask" => match (&answer.choice, &text) {
                (Some(c), None) if body["options"].as_array().is_some_and(|o| o.iter().any(|o| o == c.as_str())) => {
                    Answer { choice: Some(c.clone()), ..Default::default() }
                }
                (None, Some(t)) => Answer { text: Some(t.clone()), ..Default::default() },
                _ => bail!("Choose one of the options, or type an answer."),
            },
            "handOff" => match answer.done {
                Some(done) if answer.choice.is_none() => Answer { done: Some(done), text, ..Default::default() },
                _ => bail!("Say whether you did it."),
            },
            _ => bail!("This card doesn't take an answer."),
        };
        let n = db.execute(
            "UPDATE cards SET state = 'answered', answer = ?2, answered_by = ?3, resolved = ?4, revision = revision + 1
             WHERE id = ?1 AND state = 'waiting' AND revision = ?5",
            params![card, serde_json::to_string(&checked)?, by, now(), revision],
        )?;
        drop(db);
        if n == 0 {
            bail!("The agent isn't waiting for this any more.");
        }
        self.wake();
        self.changed();
        Ok(())
    }

    /// Launches for the app, newest first, with their cards.
    pub fn snapshot(&self) -> Value {
        let db = self.db.lock().unwrap();
        let since = now() - SHOWN_FOR_MS;
        let launches = (|| -> rusqlite::Result<Vec<Value>> {
            let mut q = db.prepare(
                "SELECT id, kind, cwd, title, agent, tools, created, ended, status, steps, revision FROM launches
                 WHERE created >= ?1 OR id IN (SELECT launch FROM cards WHERE state = 'waiting')
                 ORDER BY created DESC LIMIT ?2",
            )?;
            let mut cards = db.prepare(
                "SELECT id, kind, body, state, answer, created, resolved, revision FROM
                 (SELECT * FROM cards WHERE launch = ?1 ORDER BY created DESC LIMIT ?2) ORDER BY created",
            )?;
            let rows = q
                .query_map(params![since, SHOWN_MAX], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        json!({
                            "id": r.get::<_, String>(0)?,
                            "kind": r.get::<_, String>(1)?,
                            "cwd": r.get::<_, String>(2)?,
                            "title": r.get::<_, String>(3)?,
                            "agentId": r.get::<_, Option<String>>(4)?,
                            "tools": r.get::<_, bool>(5)?,
                            "createdAt": r.get::<_, i64>(6)?,
                            "endedAt": r.get::<_, Option<i64>>(7)?,
                            "status": r.get::<_, Option<String>>(8)?,
                            "steps": r.get::<_, Option<String>>(9)?.and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(json!([])),
                            "revision": r.get::<_, i64>(10)?,
                        }),
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut out = Vec::new();
            for (id, mut launch) in rows {
                let list = cards
                    .query_map(params![id, SHOWN_CARDS], |r| {
                        let mut card: Value = serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(json!({}));
                        card["id"] = json!(r.get::<_, String>(0)?);
                        card["kind"] = json!(r.get::<_, String>(1)?);
                        card["state"] = json!(r.get::<_, String>(3)?);
                        card["answer"] = r.get::<_, Option<String>>(4)?.and_then(|a| serde_json::from_str(&a).ok()).unwrap_or(Value::Null);
                        card["createdAt"] = json!(r.get::<_, i64>(5)?);
                        card["resolvedAt"] = json!(r.get::<_, Option<i64>>(6)?);
                        card["revision"] = json!(r.get::<_, i64>(7)?);
                        Ok(card)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                launch["cards"] = Value::from(list);
                out.push(launch);
            }
            Ok(out)
        })();
        Value::from(launches.unwrap_or_default())
    }
}

/// The engine's card service, once `serve` has opened it.
static SERVICE: std::sync::OnceLock<Arc<Cards>> = std::sync::OnceLock::new();

pub fn service() -> Option<&'static Arc<Cards>> {
    SERVICE.get()
}

/// Opens the service and starts listening for launches' MCP servers. Without it (another engine
/// already listens, or the folder cannot be used) supported launches refuse to start.
pub fn start() -> Option<&'static Arc<Cards>> {
    let cards = match Cards::open_default() {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!("lantern-engine: cards are off: {e:#}");
            return None;
        }
    };
    if let Err(e) = ipc::listen(cards.clone()) {
        eprintln!("lantern-engine: cards are off: {e:#}");
        return None;
    }
    let monitor = cards.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            monitor.refresh_sessions(session_alive);
        }
    });
    let _ = SERVICE.set(cards);
    SERVICE.get()
}

/// Only an explicit session reply proves exit; discovery omission and transport errors don't.
#[cfg(unix)]
fn session_alive(session: &str) -> Option<bool> {
    if !session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let out = crate::run::output(
        crate::tmux::command().env("LC_ALL", "C").args(["-L", crate::launch::LANTERN_SOCKET, "has-session", "-t", &format!("={session}")]),
        std::time::Duration::from_secs(2),
    )
    .ok()?;
    if out.status.success() {
        return Some(true);
    }
    let error = String::from_utf8_lossy(&out.stderr);
    if error.contains("can't find session:")
        || error.contains("no server running on")
        || (error.contains("error connecting to") && error.contains("No such file or directory"))
    {
        Some(false)
    } else {
        None
    }
}
#[cfg(windows)]
fn session_alive(session: &str) -> Option<bool> {
    if !crate::pty::valid_name(session) {
        return None;
    }
    if crate::pty::dir().join(format!("{session}.ended")).is_file() {
        return Some(false);
    }
    crate::pty::info(session).ok().map(|info| info.alive)
}

/// What a launch gets: its id, and the MCP server command for its agent.
pub struct Registered {
    pub id: String,
    pub server: (String, Vec<String>),
}

/// Registers a launch of `kind` that gets the card tools, when the service is running.
pub fn register(kind: Kind, cwd: &Path, title: &str, operation: Option<&str>) -> Option<Registered> {
    let cards = service().filter(|_| supported(kind))?;
    let engine = crate::hooks::engine_copy().ok()?;
    let (id, _) = cards.register(kind, cwd, title, Some(&dir().join("keys")), operation).ok()?;
    Some(Registered { server: (engine.to_string_lossy().into_owned(), vec!["cards-mcp".into(), id.clone()]), id })
}

/// A registered launch that won't start, or didn't get the tools after all.
pub fn forget(id: &str) {
    if let Some(cards) = service() {
        cards.discard(id, Some(&dir().join("keys")));
    }
}

/// Records how a registered launch went: its session when it started, else it's forgotten.
pub fn launched(registered: Option<Registered>, result: Result<crate::launch::Launched>) -> Result<crate::launch::Launched> {
    let (Some(r), Some(cards)) = (registered, service()) else { return result };
    match result {
        Ok(mut l) => {
            let _ = cards.started(&r.id, l.session.as_deref());
            l.launch = Some(r.id);
            Ok(l)
        }
        Err(e) => {
            let _ = cards
                .db
                .lock()
                .unwrap()
                .execute("UPDATE launches SET status=?2 WHERE id=?1", params![r.id, format!("Start unconfirmed: {e:#}")]);
            cards.changed();
            Err(e)
        }
    }
}

/// What the first message says about the card tools.
pub fn note() -> &'static str {
    "You have Lantern's tools (lantern): when you need the person to decide something, use ask_person instead of asking \
     in your reply; when a step needs them (signing in, a code, a payment, anything physical), use hand_off or \
     need_sign_in; keep them posted with set_status and set_steps; and finish with show_result."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_creation_reply_reuses_the_card_and_rejects_an_altered_body() {
        let (cards, launch, key) = cards();
        let operation = uuid::Uuid::new_v4().to_string();
        let request = |question: &str| Op::Ask {
            operation_id: operation.clone(),
            question: question.into(),
            details: None,
            options: vec!["Yes".into(), "No".into()],
        };
        let first = cards.handle(&launch, &key, request("Approve this exact thing?")).unwrap();
        let retry = cards.handle(&launch, &key, request("Approve this exact thing?")).unwrap();
        assert_eq!(first, retry);
        assert!(cards.handle(&launch, &key, request("Approve something else?")).is_err());
        let count: i64 = cards.db.lock().unwrap().query_row("SELECT COUNT(*) FROM cards WHERE launch=?1", [launch], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    fn cards() -> (Cards, String, String) {
        let path = std::env::temp_dir().join(format!("lantern-cards-{}.db", uuid::Uuid::new_v4()));
        let cards = Cards::open(&path).unwrap();
        let (launch, key) = cards.register(Kind::October, Path::new("/tmp/p"), "Book the flight\nmore", None, None).unwrap();
        (cards, launch, key)
    }

    fn ask(cards: &Cards, launch: &str, key: &str, options: &[&str]) -> String {
        let op = Op::Ask {
            operation_id: uuid::Uuid::new_v4().to_string(),
            question: "Which one?".into(),
            details: None,
            options: options.iter().map(|o| o.to_string()).collect(),
        };
        cards.handle(launch, key, op).unwrap()["card"].as_str().unwrap().to_string()
    }

    fn wait(cards: &Cards, launch: &str, key: &str, card: &str) -> Value {
        cards.handle(launch, key, Op::Wait { card: card.into() }).unwrap()
    }

    fn state(cards: &Cards, card: &str) -> String {
        cards.db.lock().unwrap().query_row("SELECT state FROM cards WHERE id = ?1", [card], |r| r.get(0)).unwrap()
    }

    fn choice(c: &str) -> Answer {
        Answer { choice: Some(c.into()), ..Default::default() }
    }

    #[test]
    fn an_answer_reaches_the_waiting_call_once_and_is_receipted() {
        let (cards, launch, key) = cards();
        let card = ask(&cards, &launch, &key, &["Morning", "Evening"]);
        assert_eq!(state(&cards, &card), "waiting");
        // Only an offered option, at the revision the person saw.
        assert!(cards.answer(&card, 1, &choice("Night"), "app").is_err());
        assert!(cards.answer(&card, 2, &choice("Morning"), "app").is_err());
        cards.answer(&card, 1, &choice("Evening"), "app").unwrap();
        // A second answer (another device, a double click) doesn't replace it.
        assert!(cards.answer(&card, 1, &choice("Morning"), "app").is_err());
        let got = wait(&cards, &launch, &key, &card);
        assert_eq!(got["state"], "answered");
        assert_eq!(got["answer"]["choice"], "Evening");
        assert_eq!(state(&cards, &card), "answered");
        cards.handle(&launch, &key, Op::Ack { card: card.clone() }).unwrap();
        assert_eq!(state(&cards, &card), "delivered");
    }

    #[test]
    fn only_the_launch_with_its_key_can_make_or_wait_for_cards() {
        let (cards, launch, key) = cards();
        assert!(cards.handle(&launch, "wrong", Op::Hello).is_err());
        let (other, other_key) = cards.register(Kind::Claude, Path::new("/tmp"), "x", None, None).unwrap();
        let card = ask(&cards, &launch, &key, &[]);
        assert!(cards.handle(&other, &key, Op::Hello).is_err());
        // Another launch can't see this one's card.
        assert!(cards.handle(&other, &other_key, Op::Wait { card }).is_err());
    }

    #[test]
    fn a_card_whose_call_stopped_waiting_closes_and_takes_no_answer() {
        let (cards, launch, key) = cards();
        let card = ask(&cards, &launch, &key, &["Yes", "No"]);
        cards.db.lock().unwrap().execute("UPDATE cards SET lease = 1 WHERE id = ?1", [&card]).unwrap();
        assert!(cards.sweep());
        assert_eq!(state(&cards, &card), "stopped");
        assert!(cards.answer(&card, 2, &choice("Yes"), "app").is_err());
        // Cancelled by the agent, and waited too long: closed too, never a yes.
        let cancelled = ask(&cards, &launch, &key, &["Yes"]);
        cards.handle(&launch, &key, Op::Cancel { card: cancelled.clone() }).unwrap();
        assert_eq!(wait(&cards, &launch, &key, &cancelled)["state"], "cancelled");
        let old = ask(&cards, &launch, &key, &["Yes"]);
        cards.db.lock().unwrap().execute("UPDATE cards SET expires = 1 WHERE id = ?1", [&old]).unwrap();
        let got = wait(&cards, &launch, &key, &old);
        assert_eq!(got["state"], "expired");
        assert!(got["answer"].is_null());
    }

    #[test]
    fn an_answer_the_call_never_took_is_unconfirmed() {
        let (cards, launch, key) = cards();
        let card = ask(&cards, &launch, &key, &["Yes"]);
        cards.answer(&card, 1, &choice("Yes"), "app").unwrap();
        cards.db.lock().unwrap().execute("UPDATE cards SET lease = 1 WHERE id = ?1", [&card]).unwrap();
        cards.sweep();
        assert_eq!(state(&cards, &card), "unconfirmed");
    }

    #[test]
    fn hand_offs_take_done_or_not_and_only_safe_addresses() {
        let (cards, launch, key) = cards();
        let hand_off = |reason, url: &str| Op::HandOff {
            operation_id: uuid::Uuid::new_v4().to_string(),
            reason,
            step: "Enter the code".into(),
            why: None,
            instructions: vec![],
            url: Some(url.into()),
        };
        for bad in ["javascript:alert(1)", "file:///etc/passwd", "https://me:secret@bank.example", "http://bank.example"] {
            assert!(cards.handle(&launch, &key, hand_off(Reason::SignIn, bad)).is_err(), "{bad}");
        }
        assert!(cards.handle(&launch, &key, hand_off(Reason::Verification, "http://router.local")).is_ok());
        let card = cards.handle(&launch, &key, hand_off(Reason::SignIn, "https://bank.example/login")).unwrap()["card"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(cards.answer(&card, 1, &choice("Yes"), "app").is_err());
        cards.answer(&card, 1, &Answer { done: Some(false), text: Some("Not home".into()), ..Default::default() }, "app").unwrap();
        let got = wait(&cards, &launch, &key, &card);
        assert_eq!(got["answer"]["done"], false);
        assert_eq!(got["answer"]["text"], "Not home");
    }

    #[test]
    fn progress_and_results_are_kept_for_the_app() {
        let (cards, launch, key) = cards();
        cards.handle(&launch, &key, Op::Status { text: "Comparing prices".into() }).unwrap();
        let steps =
            |ids: &[&str]| Op::Steps { steps: ids.iter().map(|i| StepIn { id: i.to_string(), title: format!("Step {i}") }).collect() };
        cards.handle(&launch, &key, steps(&["a", "b"])).unwrap();
        cards.handle(&launch, &key, Op::StepDone { id: "a".into() }).unwrap();
        assert!(cards.handle(&launch, &key, Op::StepDone { id: "zz".into() }).is_err());
        // Replanning keeps a done step done.
        cards.handle(&launch, &key, steps(&["a", "c"])).unwrap();
        let file = std::env::temp_dir().join(format!("lantern-result-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&file, "x").unwrap();
        let result = |files: Vec<String>| Op::Result {
            operation_id: uuid::Uuid::new_v4().to_string(),
            summary: "Booked".into(),
            outcome: Outcome::Partial,
            files,
        };
        assert!(cards.handle(&launch, &key, result(vec!["relative.txt".into()])).is_err());
        assert!(cards.handle(&launch, &key, result(vec!["/no/such/file".into()])).is_err());
        cards.handle(&launch, &key, result(vec![file.to_string_lossy().into_owned()])).unwrap();

        let all = cards.snapshot();
        let shown = all.as_array().unwrap().iter().find(|l| l["id"] == launch.as_str()).unwrap();
        assert_eq!(shown["title"], "Book the flight");
        assert_eq!(shown["status"], "Comparing prices");
        assert_eq!(shown["steps"][0]["done"], true);
        assert_eq!(shown["steps"][1]["done"], false);
        let card = &shown["cards"][0];
        assert_eq!(
            (card["kind"].as_str(), card["outcome"].as_str(), card["state"].as_str()),
            (Some("result"), Some("partial"), Some("shown"))
        );
        assert_eq!(card["files"][0]["name"], file.file_name().unwrap().to_string_lossy().as_ref());
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn agent_text_is_bounded() {
        let (cards, launch, key) = cards();
        let ask = |question: String, options: Vec<String>| {
            cards.handle(&launch, &key, Op::Ask { operation_id: uuid::Uuid::new_v4().to_string(), question, details: None, options })
        };
        assert!(ask("x".repeat(501), vec![]).is_err());
        assert!(ask("ok?\u{1b}[2J".into(), vec![]).is_err());
        assert!(ask("ok?".into(), (0..7).map(|i| i.to_string()).collect()).is_err());
        assert!(ask("ok?".into(), vec!["Yes".into(), "Yes".into()]).is_err());
        for _ in 0..MAX_WAITING {
            ask("ok?".into(), vec![]).unwrap();
        }
        assert!(ask("one more?".into(), vec![]).is_err());
    }

    #[test]
    fn discovery_gaps_do_not_end_a_session() {
        let (cards, launch, _) = cards();
        cards.started(&launch, Some("october-p-123")).unwrap();
        let mut agent = crate::tests::agent_for(42, 7, None);
        agent.id = "october:42:7".into();
        agent.tmux = Some(crate::model::TmuxPane {
            socket: Some(crate::launch::LANTERN_SOCKET.into()),
            target: "october-p-123:0.0".into(),
            pane_id: "%1".into(),
        });
        assert!(cards.bind(std::slice::from_ref(&agent)));
        let bound = |c: &Cards| c.snapshot().as_array().unwrap().iter().find(|l| l["id"] == launch.as_str()).unwrap().clone();
        assert_eq!(bound(&cards)["agentId"], "october:42:7");
        assert!(bound(&cards)["endedAt"].is_null());
        assert!(!cards.bind(&[]));
        assert!(bound(&cards)["endedAt"].is_null());
        cards.db.lock().unwrap().execute("UPDATE launches SET created=?2 WHERE id=?1", params![launch, now() - 60_000]).unwrap();
        cards.refresh_sessions(|_| None);
        assert!(bound(&cards)["endedAt"].is_null());
        cards.refresh_sessions(|_| Some(true));
        assert!(bound(&cards)["endedAt"].is_null());
        cards.refresh_sessions(|_| Some(false));
        assert!(bound(&cards)["endedAt"].is_number());
    }

    #[cfg(unix)]
    #[test]
    fn requests_go_over_one_line_each_way() {
        use std::io::Write;
        let (cards, launch, key) = cards();
        let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        let reply = std::thread::spawn(move || {
            let r = ipc::exchange(client, &json!({"launch": launch, "key": key, "op": "status", "text": "Working"})).unwrap();
            r["ok"].as_bool()
        });
        let answered = ipc::answer(&cards, &server);
        writeln!(&server, "{answered}").unwrap();
        assert_eq!(reply.join().unwrap(), Some(true));
        let (refusing, client) = std::os::unix::net::UnixStream::pair().unwrap();
        let reply = std::thread::spawn(move || ipc::exchange(client, &json!({"launch": "x", "key": "y", "op": "hello"})).unwrap());
        let answered = ipc::answer(&cards, &refusing);
        writeln!(&refusing, "{answered}").unwrap();
        assert_eq!(reply.join().unwrap()["ok"], false);
    }
}
