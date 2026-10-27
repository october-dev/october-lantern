//! Durable human control of Lantern's app-access channel and owned terminal sessions.
//! This is deliberately not a claim that arbitrary shell children are isolated or stopped.
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    pub request_id: String,
    pub action: String,
    pub launch_id: Option<String>,
}

pub fn worker(show: crate::show::Handle, emit: fn(&Value)) -> std::sync::mpsc::SyncSender<Command> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Command>(16);
    if crate::cards::service().is_none() {
        drop(rx);
        emit(&json!({"type":"computerHold","held":false,"message":"This engine does not own the task service; control is unavailable."}));
        return tx;
    }
    // Restore the durable gate before accepting any driver-start requests.
    let database = crate::cards::dir().join("cards.db");
    let opened = (|| -> Result<Connection> {
        let db = Connection::open(database)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS computer_hold(id INTEGER PRIMARY KEY CHECK(id=1),held INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS control_receipts(id TEXT PRIMARY KEY,body TEXT NOT NULL,result TEXT NOT NULL);",
        )?;
        let held: bool = db.query_row("SELECT held FROM computer_hold WHERE id=1", [], |r| r.get(0)).optional()?.unwrap_or(false);
        if held {
            crate::computer::hold();
        }
        emit(
            &json!({"type":"computerHold","held":held,"message":if held {Some("App access is paused. Background commands may still run.")}else{None}}),
        );
        Ok(db)
    })();
    std::thread::spawn(move || {
        let db = match opened {
            Ok(db) => db,
            Err(e) => {
                crate::computer::hold();
                emit(&json!({"type":"computerHold","held":true,"message":format!("Control unavailable: {e:#}")}));
                return;
            }
        };
        while let Ok(command) = rx.recv() {
            let body = json!({"action":command.action,"launchId":command.launch_id}).to_string();
            let result = (|| -> Result<Value> {
                uuid::Uuid::parse_str(&command.request_id).context("Invalid control operation id")?;
                if let Some((old, receipt)) = db
                    .query_row("SELECT body,result FROM control_receipts WHERE id=?1", [&command.request_id], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .optional()?
                {
                    if body != old {
                        bail!("This operation already names a different action");
                    }
                    return Ok(serde_json::from_str(&receipt)?);
                }
                let message = match command.action.as_str() {
                    "pause" | "stop" => {
                        db.execute("INSERT INTO computer_hold VALUES(1,1) ON CONFLICT(id) DO UPDATE SET held=1", [])?;
                        crate::computer::hold();
                        emit(
                            &json!({"type":"computerHold","held":true,"message":"Pausing app access… This affects all agents Lantern started."}),
                        );
                        let _ = show.send(crate::show::Cmd::DriverOff);
                        if let Some(cards) = crate::cards::service() {
                            cards.cancel_waits()?;
                        }
                        let revoked = crate::computer::revoke_and_stop()?;
                        if command.action == "stop" {
                            let launch = command.launch_id.as_deref().context("Choose a session to stop")?;
                            stop_session(launch)?;
                            "The owned terminal session ended. App access remains paused; background commands are not verified stopped."
                                .to_string()
                        } else if revoked {
                            "App access is paused. Background commands may still run; this is not protected takeover.".into()
                        } else {
                            "Lantern's driver exited, but revocation could not be confirmed. Background commands may still run.".into()
                        }
                    }
                    "continue" => {
                        crate::computer::revoke_and_stop()?;
                        let generation = crate::computer::prepare_continue();
                        match crate::computer::start_for_continue(generation) {
                            Ok(crate::computer::Started::Ready) => {}
                            Ok(_) => {
                                crate::computer::hold();
                                bail!("Continue was superseded; app access remains paused");
                            }
                            Err(e) => {
                                crate::computer::hold();
                                return Err(e.context("App access remains paused"));
                            }
                        }
                        db.execute("INSERT INTO computer_hold VALUES(1,0) ON CONFLICT(id) DO UPDATE SET held=0", [])?;
                        if !crate::computer::complete_continue(generation) {
                            db.execute("INSERT INTO computer_hold VALUES(1,1) ON CONFLICT(id) DO UPDATE SET held=1", [])?;
                            crate::computer::hold();
                            bail!("Continue was superseded; app access remains paused");
                        }
                        emit(&crate::computer::status_json("ready", None));
                        "App access is available through a fresh driver. Closed questions and old actions were not replayed.".into()
                    }
                    _ => bail!("Unknown control action"),
                };
                let receipt = json!({"ok":true,"message":message});
                db.execute("INSERT INTO control_receipts VALUES(?1,?2,?3)", params![command.request_id, body, receipt.to_string()])?;
                Ok(receipt)
            })();
            let receipt = match result {
                Ok(v) => v,
                Err(e) => json!({"ok":false,"message":format!("{e:#}")}),
            };
            emit(&json!({"type":"computerHold","held":crate::computer::held(),"message":receipt["message"]}));
            emit(&json!({"type":"controlResult","requestId":command.request_id,"ok":receipt["ok"],"message":receipt["message"]}));
        }
    });
    tx
}

fn stop_session(launch: &str) -> Result<()> {
    let cards = crate::cards::service().context("Cards are unavailable; cannot verify the session")?;
    let session = cards.session_for_control(launch)?;
    if session.is_empty() || !session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        bail!("Invalid owned session identity");
    }
    #[cfg(unix)]
    {
        let out = crate::run::output(
            crate::tmux::command().args(["-L", crate::launch::LANTERN_SOCKET, "kill-session", "-t", &format!("={session}")]),
            std::time::Duration::from_secs(5),
        )?;
        if !out.status.success() {
            bail!("The session did not confirm stopping: {}", String::from_utf8_lossy(&out.stderr));
        }
    }
    #[cfg(windows)]
    {
        let response = crate::pty::request(&session, &json!({"op":"stop"}))?;
        if response["ok"] != true {
            bail!("The session did not confirm stopping");
        }
    }
    cards.session_stopped(launch)
}
