//! Local routines. The schedule and its admission journal live with the card database.
//! Admission is committed before launching. An interrupted admission stays unknown and blocks
//! overlap; restarting the engine never repeats it or catches up missed occurrences.
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::Kind;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub name: String,
    pub prompt: String,
    pub cwd: String,
    pub kind: Kind,
    pub model: Option<String>,
    pub timezone: String,
    /// daily, weekdays, or weekly (weekday: Monday = 0).
    pub cadence: String,
    pub hour: u32,
    pub minute: u32,
    #[serde(default)]
    pub weekday: u32,
    #[serde(default)]
    pub preferences: String,
}

impl Draft {
    fn check(&self) -> Result<Tz> {
        for (label, text, limit) in [("Name", &self.name, 120), ("Task", &self.prompt, 8000), ("Preferences", &self.preferences, 4000)] {
            if text.chars().count() > limit || text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                bail!("{label} is too long or contains control characters");
            }
        }
        if self.name.trim().is_empty() || self.prompt.trim().is_empty() {
            bail!("Give the routine a name and a task");
        }
        if !Path::new(&self.cwd).is_absolute() || !Path::new(&self.cwd).is_dir() {
            bail!("Choose an existing project folder");
        }
        if self.hour > 23 || self.minute > 59 || self.weekday > 6 {
            bail!("Choose a valid local time and weekday");
        }
        if !matches!(self.cadence.as_str(), "daily" | "weekdays" | "weekly") {
            bail!("Choose Daily, Weekdays or Weekly");
        }
        if !crate::cards::supported(self.kind) {
            bail!("Routines need October, Claude Code or Codex with card support");
        }
        if self.model.as_deref().is_some_and(|m| !crate::models::valid(m)) {
            bail!("Choose a valid model");
        }
        self.timezone.parse().context("Choose an IANA timezone, such as America/Los_Angeles")
    }

    fn occurs(&self, date: NaiveDate) -> bool {
        let day = date.weekday().num_days_from_monday();
        match self.cadence.as_str() {
            "weekdays" => day < 5,
            "weekly" => day == self.weekday,
            _ => true,
        }
    }

    fn on_date(&self, zone: Tz, date: NaiveDate) -> Option<DateTime<Utc>> {
        if !self.occurs(date) {
            return None;
        }
        let local = date.and_hms_opt(self.hour, self.minute, 0)?;
        match zone.from_local_datetime(&local) {
            LocalResult::Single(t) => Some(t.with_timezone(&Utc)),
            LocalResult::Ambiguous(a, b) => Some(a.min(b).with_timezone(&Utc)),
            LocalResult::None => None,
        }
    }

    pub fn next(&self, after: DateTime<Utc>) -> Result<Option<DateTime<Utc>>> {
        let zone = self.check()?;
        let today = after.with_timezone(&zone).date_naive();
        Ok((0..15).filter_map(|n| today.checked_add_days(chrono::Days::new(n))).filter_map(|d| self.on_date(zone, d)).find(|t| *t > after))
    }
}

pub struct Routines {
    db: Mutex<Connection>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Command {
    Save { request_id: String, id: Option<String>, revision: Option<i64>, draft: Draft },
    Change { request_id: String, id: String, revision: i64, change: String },
    Run { request_id: String, id: String, revision: i64 },
    List,
}

impl Command {
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Save { request_id, .. } | Self::Change { request_id, .. } | Self::Run { request_id, .. } => Some(request_id),
            Self::List => None,
        }
    }
}

pub fn worker(emit: fn(&Value)) -> std::sync::mpsc::SyncSender<Command> {
    let (tx, rx) = std::sync::mpsc::sync_channel(32);
    if crate::cards::service().is_none() {
        drop(rx);
        emit(&json!({"type":"routines","items":[],"error":"This engine does not own the task service. Quit the other Lantern instance."}));
        return tx;
    }
    std::thread::spawn(move || {
        let routines = match Routines::open_default() {
            Ok(r) => r,
            Err(e) => {
                emit(&json!({"type":"routines","items":[],"error":format!("Routines unavailable: {e:#}")}));
                return;
            }
        };
        let mut previous = routines.last_tick().unwrap_or_else(Utc::now);
        loop {
            let command = match rx.recv_timeout(std::time::Duration::from_secs(15)) {
                Ok(c) => Some(c),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                Err(_) => break,
            };
            if let Some(command) = command {
                let (id, result) = match command {
                    Command::Save { request_id, id, revision, draft } => {
                        let result =
                            if id.is_none() { routines.create(&request_id, draft) } else { routines.save(id.as_deref(), revision, draft) };
                        (request_id, result.map(|_| ()))
                    }
                    Command::Change { request_id, id, revision, change } => (request_id, routines.change(&id, revision, &change)),
                    Command::Run { request_id, id, revision } => {
                        let result = routines.run_now(&id, revision, &request_id);
                        (request_id, result)
                    }
                    Command::List => (String::new(), Ok(())),
                };
                if !id.is_empty() {
                    emit(
                        &json!({"type":"routineResult","requestId":id,"ok":result.is_ok(),"message":result.err().map(|e|format!("{e:#}"))}),
                    );
                }
            }
            let now = Utc::now();
            if let Err(e) = routines.tick(now, previous) {
                eprintln!("Lantern routines: {e:#}");
            }
            previous = now;
            match routines.snapshot() {
                Ok(items) => emit(&json!({"type":"routines","items":items})),
                Err(e) => emit(&json!({"type":"routines","items":[],"error":format!("{e:#}")})),
            }
        }
    });
    tx
}

impl Routines {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS routines(
            id TEXT PRIMARY KEY, revision INTEGER NOT NULL, draft TEXT NOT NULL,
            enabled INTEGER NOT NULL, changed INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS routine_runs(
            id TEXT PRIMARY KEY, routine TEXT NOT NULL, occurrence TEXT NOT NULL,
            admitted INTEGER NOT NULL, state TEXT NOT NULL, launch TEXT, message TEXT,
            UNIQUE(routine, occurrence));
            CREATE TABLE IF NOT EXISTS routine_creations(id TEXT PRIMARY KEY,draft TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS routine_clock(id INTEGER PRIMARY KEY CHECK(id=1),tick INTEGER NOT NULL);",
        )?;
        // A committed admission without a receipt is intentionally never retried.
        db.execute("UPDATE routine_runs SET state='unknown', message='Lantern restarted during admission. Check the task before starting another run.' WHERE state='admitting'", [])?;
        Ok(Self { db: Mutex::new(db) })
    }

    fn last_tick(&self) -> Option<DateTime<Utc>> {
        let millis: Option<i64> =
            self.db.lock().ok()?.query_row("SELECT tick FROM routine_clock WHERE id=1", [], |r| r.get(0)).optional().ok()?;
        millis.and_then(DateTime::from_timestamp_millis)
    }

    pub fn open_default() -> Result<Self> {
        Self::open(&crate::cards::dir().join("cards.db"))
    }

    pub fn create(&self, operation: &str, draft: Draft) -> Result<String> {
        uuid::Uuid::parse_str(operation).context("Invalid operation id")?;
        draft.check()?;
        let value = serde_json::to_string(&draft)?;
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let existing: Option<String> =
            tx.query_row("SELECT draft FROM routine_creations WHERE id=?1", [operation], |r| r.get(0)).optional()?;
        if let Some(existing) = existing {
            if existing != value {
                bail!("This operation was already used for another routine");
            }
            return Ok(operation.into());
        }
        tx.execute("INSERT INTO routine_creations VALUES(?1,?2)", params![operation, value])?;
        tx.execute("INSERT INTO routines VALUES(?1,1,?2,1,?3)", params![operation, value, Utc::now().timestamp_millis()])?;
        tx.commit()?;
        Ok(operation.into())
    }

    pub fn save(&self, id: Option<&str>, revision: Option<i64>, draft: Draft) -> Result<String> {
        draft.check()?;
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let now = Utc::now().timestamp_millis();
        let value = serde_json::to_string(&draft)?;
        let id = match id {
            Some(id) => {
                if tx.execute(
                    "UPDATE routines SET draft=?2, revision=revision+1, changed=?3 WHERE id=?1 AND revision=?4",
                    params![id, value, now, revision],
                )? == 0
                {
                    bail!("This routine changed; reopen it before saving");
                }
                id.to_string()
            }
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                tx.execute("INSERT INTO routines VALUES(?1,1,?2,1,?3)", params![id, value, now])?;
                id
            }
        };
        tx.commit()?;
        Ok(id)
    }

    pub fn change(&self, id: &str, revision: i64, action: &str) -> Result<()> {
        let db = self.db.lock().unwrap();
        let n = match action {
            "pause" | "enable" => db.execute(
                "UPDATE routines SET enabled=?3,revision=revision+1,changed=?4 WHERE id=?1 AND revision=?2",
                params![id, revision, action == "enable", Utc::now().timestamp_millis()],
            )?,
            "delete" => db.execute("DELETE FROM routines WHERE id=?1 AND revision=?2", params![id, revision])?,
            _ => bail!("Unknown routine action"),
        };
        if n == 0 {
            bail!("This routine changed; reopen it first");
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<Value> {
        let db = self.db.lock().unwrap();
        let mut q = db.prepare("SELECT id,revision,draft,enabled,changed FROM routines ORDER BY changed DESC")?;
        let rows = q.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, bool>(3)?, r.get::<_, i64>(4)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, revision, text, enabled, changed) = row?;
            let draft: Draft = serde_json::from_str(&text)?;
            let next = if enabled { draft.next(Utc::now()).ok().flatten().map(|t| t.to_rfc3339()) } else { None };
            let last: Option<Value> = db.query_row("SELECT id,state,launch,message,admitted FROM routine_runs WHERE routine=?1 ORDER BY admitted DESC LIMIT 1",[&id],|r|
                Ok(json!({"id":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"launch":r.get::<_,Option<String>>(2)?,"message":r.get::<_,Option<String>>(3)?,"at":r.get::<_,i64>(4)?}))).optional()?;
            out.push(json!({"id":id,"revision":revision,"draft":draft,"enabled":enabled,"changed":changed,"nextRun":next,"lastRun":last,"error":draft.check().err().map(|e|e.to_string())}));
        }
        Ok(json!(out))
    }

    /// Called on one scheduler worker. No network/launch runs under the SQLite lock.
    pub fn admit(&self, id: &str, revision: i64, occurrence: &str, operation: &str, at: DateTime<Utc>) -> Result<Option<Draft>> {
        if operation.is_empty() || operation.len() > 128 {
            bail!("Invalid operation id");
        }
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        if tx
            .query_row(
                "SELECT 1 FROM routine_runs WHERE id=?1 OR (routine=?2 AND occurrence=?3)",
                params![operation, id, occurrence],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Ok(None);
        }
        let (text, enabled): (String, bool) = tx
            .query_row("SELECT draft,enabled FROM routines WHERE id=?1 AND revision=?2", params![id, revision], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .context("This routine changed or was deleted")?;
        if !enabled && !occurrence.starts_with("manual:") {
            return Ok(None);
        }
        let unresolved: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM routine_runs WHERE routine=?1 AND state IN ('admitting','unknown','running'))",
            [id],
            |r| r.get(0),
        )?;
        let draft: Draft = serde_json::from_str(&text)?;
        draft.check()?;
        let (state, message) = if unresolved {
            ("skipped", Some("A previous run is still running or unconfirmed; this run was skipped"))
        } else {
            ("admitting", None)
        };
        tx.execute(
            "INSERT INTO routine_runs(id,routine,occurrence,admitted,state,message,launch) VALUES(?1,?2,?3,?4,?5,?6,?1)",
            params![operation, id, occurrence, at.timestamp_millis(), state, message],
        )?;
        tx.commit()?;
        Ok((!unresolved).then_some(draft))
    }

    fn receipt(&self, operation: &str, result: Result<crate::launch::Launched>) -> Result<()> {
        let (state, launch, message) = match result {
            Ok(r) if r.launch.is_some() => ("running", r.launch, r.bus_problem),
            Ok(_) => ("unknown", None, Some("The session started without a card receipt. Check it before starting another run.".into())),
            Err(e) => ("unknown", Some(operation.to_string()), Some(format!("{e:#}"))),
        };
        self.db.lock().unwrap().execute(
            "UPDATE routine_runs SET state=?2,launch=?3,message=?4 WHERE id=?1 AND state='admitting'",
            params![operation, state, launch, message],
        )?;
        Ok(())
    }

    pub fn run_now(&self, id: &str, revision: i64, operation: &str) -> Result<()> {
        let occurrence = format!("manual:{operation}");
        if let Some(draft) = self.admit(id, revision, &occurrence, operation, Utc::now())? {
            self.start(operation, draft)?;
        }
        Ok(())
    }

    fn start(&self, operation: &str, draft: Draft) -> Result<()> {
        let prompt = format!(
            "{}\n\nThis is a local routine, confirmed by the person. Use Lantern cards for questions and results.\nPreferences (instructions, not enforced restrictions):\n{}",
            draft.prompt, draft.preferences
        );
        let result = crate::launch::launch(
            draft.kind,
            &PathBuf::from(draft.cwd),
            Some(&prompt),
            crate::launch::Extras { ask_first: true, run_once: true, operation_id: Some(operation), ..Default::default() },
            draft.model.as_deref(),
            crate::launch::Mode::Background,
            std::time::Instant::now() + std::time::Duration::from_secs(120),
        );
        self.receipt(operation, result)
    }

    pub fn tick(&self, at: DateTime<Utc>, previous: DateTime<Utc>) -> Result<()> {
        {
            let db = self.db.lock().unwrap();
            db.execute("UPDATE routine_runs SET state='running' WHERE state='unknown' AND launch IN (SELECT id FROM launches WHERE agent IS NOT NULL AND ended IS NULL)",[])?;
            db.execute("UPDATE routine_runs SET state=CASE (SELECT json_extract(body,'$.outcome') FROM cards WHERE launch=routine_runs.launch AND kind='result' ORDER BY created DESC LIMIT 1) WHEN 'done' THEN 'reported_done' WHEN 'partial' THEN 'partial' WHEN 'failed' THEN 'failed' ELSE 'closed' END, message=COALESCE(message,'The owned session ended. Review its result; a closed session alone is not proof of completion.') WHERE state IN ('running','unknown') AND launch IN (SELECT id FROM launches WHERE ended IS NOT NULL)",[])?;
            db.execute("UPDATE routine_runs SET state='failed',message='No agent launch was admitted.' WHERE state='unknown' AND NOT EXISTS(SELECT 1 FROM launches WHERE id=routine_runs.launch)",[])?;
        }
        let rows = self.snapshot()?;
        for row in rows.as_array().into_iter().flatten() {
            if row["enabled"] != true {
                continue;
            }
            let draft: Draft = serde_json::from_value(row["draft"].clone())?;
            let Ok(zone) = draft.check() else {
                continue;
            };
            let changed = row["changed"].as_i64().and_then(DateTime::from_timestamp_millis).unwrap_or(previous);
            let since = previous.max(changed);
            let today = at.with_timezone(&zone).date_naive();
            let start = since.with_timezone(&zone).date_naive().max(today.checked_sub_days(chrono::Days::new(30)).unwrap_or(today));
            for n in 0..=30 {
                let Some(date) = start.checked_add_days(chrono::Days::new(n)) else {
                    break;
                };
                if date > today {
                    break;
                }
                if !draft.occurs(date) {
                    continue;
                }
                let due = draft.on_date(zone, date);
                if due.is_some_and(|due| due > at || due <= since) {
                    continue;
                }
                let gap = due.is_none();
                if gap
                    && date == today
                    && at.with_timezone(&zone).time() < chrono::NaiveTime::from_hms_opt(draft.hour, draft.minute, 0).unwrap()
                {
                    continue;
                }
                let id = row["id"].as_str().unwrap_or_default();
                let revision = row["revision"].as_i64().unwrap_or_default();
                let occurrence = format!("{date} {:02}:{:02}", draft.hour, draft.minute);
                let operation = uuid::Uuid::new_v4().to_string();
                if gap || due.is_some_and(|due| at.signed_duration_since(due).num_seconds() > 90) {
                    if self.admit(id, revision, &occurrence, &operation, at)?.is_some() {
                        let message = if gap {
                            "This local time does not exist because the clock changes; the run was skipped."
                        } else {
                            "This computer was unavailable at the scheduled time. Use Run now if needed."
                        };
                        self.db
                            .lock()
                            .unwrap()
                            .execute("UPDATE routine_runs SET state='missed',message=?2 WHERE id=?1", params![operation, message])?;
                    }
                } else if let Some(draft) = self.admit(id, revision, &occurrence, &operation, at)? {
                    self.start(&operation, draft)?;
                }
            }
        }
        self.db
            .lock()
            .unwrap()
            .execute("INSERT INTO routine_clock VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET tick=excluded.tick", [at.timestamp_millis()])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrying_create_after_delete_cannot_resurrect_a_schedule() {
        let r = Routines::open(Path::new(":memory:")).unwrap();
        let operation = uuid::Uuid::new_v4().to_string();
        r.create(&operation, draft()).unwrap();
        r.change(&operation, 1, "delete").unwrap();
        r.create(&operation, draft()).unwrap();
        assert_eq!(r.snapshot().unwrap(), json!([]));
        let mut changed = draft();
        changed.prompt = "A different job".into();
        assert!(r.create(&operation, changed).is_err());
    }

    #[test]
    fn every_saved_routine_is_scheduled_beyond_the_first_hundred() {
        let path = std::env::temp_dir().join(format!("lantern-routine-many-{}.db", uuid::Uuid::new_v4()));
        let cards = crate::cards::Cards::open(&path).unwrap();
        let routines = Routines::open(&path).unwrap();
        for _ in 0..101 {
            routines.create(&uuid::Uuid::new_v4().to_string(), draft()).unwrap();
        }
        routines.db.lock().unwrap().execute("UPDATE routines SET changed=0", []).unwrap();
        let previous = "2027-02-01T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let now = "2027-02-02T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        routines.tick(now, previous).unwrap();
        let snapshot = routines.snapshot().unwrap();
        let items = snapshot.as_array().unwrap();
        assert_eq!(items.len(), 101);
        assert!(items.iter().all(|item| item["lastRun"]["state"] == "missed"));
        drop(routines);
        drop(cards);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn downtime_records_missed_runs_without_a_catch_up_burst() {
        let path = std::env::temp_dir().join(format!("lantern-routine-clock-{}.db", uuid::Uuid::new_v4()));
        let cards = crate::cards::Cards::open(&path).unwrap();
        let r = Routines::open(&path).unwrap();
        let id = r.save(None, None, draft()).unwrap();
        r.db.lock().unwrap().execute("UPDATE routines SET changed=0 WHERE id=?1", [id]).unwrap();
        let previous = "2027-02-01T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let at = "2027-02-04T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        r.tick(at, previous).unwrap();
        r.tick(at, previous).unwrap();
        let count: i64 = r.db.lock().unwrap().query_row("SELECT COUNT(*) FROM routine_runs", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 3);
        let started: i64 =
            r.db.lock().unwrap().query_row("SELECT COUNT(*) FROM routine_runs WHERE state!='missed'", [], |r| r.get(0)).unwrap();
        assert_eq!(started, 0);
        assert_eq!(r.last_tick(), Some(at));
        drop(r);
        let reopened = Routines::open(&path).unwrap();
        assert_eq!(reopened.last_tick(), Some(at));
        drop(reopened);
        drop(cards);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn session_exit_releases_overlap_without_claiming_unreported_success() {
        let path = std::env::temp_dir().join(format!("lantern-routine-end-{}.db", uuid::Uuid::new_v4()));
        let cards = crate::cards::Cards::open(&path).unwrap();
        let routines = Routines::open(&path).unwrap();
        let routine = routines.save(None, None, draft()).unwrap();
        for (index, outcome, expected) in [(0, None, "closed"), (1, Some(crate::cards::Outcome::Done), "reported_done")] {
            let operation = uuid::Uuid::new_v4().to_string();
            let at = Utc::now();
            assert!(routines.admit(&routine, 1, &format!("manual:{index}"), &operation, at).unwrap().is_some());
            let (launch, key) = cards.register(Kind::October, Path::new(&draft().cwd), "Fixture", None, Some(&operation)).unwrap();
            routines
                .receipt(
                    &operation,
                    Ok(crate::launch::Launched {
                        session: Some("fixture".into()),
                        bus_problem: None,
                        launch: Some(launch.clone()),
                        shell_question: None,
                    }),
                )
                .unwrap();
            if let Some(outcome) = outcome {
                cards
                    .handle(
                        &launch,
                        &key,
                        crate::cards::Op::Result {
                            operation_id: uuid::Uuid::new_v4().to_string(),
                            summary: "Reported result".into(),
                            outcome,
                            files: vec![],
                        },
                    )
                    .unwrap();
            }
            cards.session_stopped(&launch).unwrap();
            routines.tick(at, at).unwrap();
            let state: String =
                routines.db.lock().unwrap().query_row("SELECT state FROM routine_runs WHERE id=?1", [operation], |r| r.get(0)).unwrap();
            assert_eq!(state, expected);
        }
        drop(routines);
        drop(cards);
        let _ = std::fs::remove_file(path);
    }

    fn draft() -> Draft {
        Draft {
            name: "Check".into(),
            prompt: "Report the time".into(),
            cwd: std::env::temp_dir().to_string_lossy().into(),
            kind: Kind::October,
            model: None,
            timezone: "America/Los_Angeles".into(),
            cadence: "daily".into(),
            hour: 1,
            minute: 30,
            weekday: 0,
            preferences: String::new(),
        }
    }
    #[test]
    fn fold_runs_once_at_the_earlier_time_and_gap_is_skipped() {
        let mut d = draft();
        let z = d.check().unwrap();
        let fold = d.on_date(z, NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()).unwrap();
        assert_eq!(fold.to_rfc3339(), "2026-11-01T08:30:00+00:00");
        d.hour = 2;
        assert!(d.on_date(z, NaiveDate::from_ymd_opt(2026, 3, 8).unwrap()).is_none());
    }
    #[test]
    fn admission_is_idempotent_and_unknown_admissions_prevent_overlap() {
        let r = Routines::open(Path::new(":memory:")).unwrap();
        let id = r.save(None, None, draft()).unwrap();
        let at = Utc::now();
        assert!(r.admit(&id, 1, "today", "a", at).unwrap().is_some());
        assert!(r.admit(&id, 1, "today", "b", at).unwrap().is_none());
        assert!(r.admit(&id, 1, "tomorrow", "c", at).unwrap().is_none());
        r.change(&id, 1, "pause").unwrap();
        assert!(r.change(&id, 1, "delete").is_err());
        assert!(r.admit(&id, 2, "next", "d", at).unwrap().is_none());
    }
}
