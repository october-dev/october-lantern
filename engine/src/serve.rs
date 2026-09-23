//! The stdio protocol the app talks to. See `protocol/README.md`.

use std::io::{BufRead, Write};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::hooks::now_ms;
use crate::model::Agent;
use crate::scanner::Scanner;
use crate::tmux;

const SCAN_EVERY: Duration = Duration::from_millis(1500);
const HEARTBEAT: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Request {
    Refresh,
    #[serde(rename_all = "camelCase")]
    Reply { request_id: String, agent_id: String, text: String },
}

fn emit(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

pub fn run() -> Result<()> {
    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(l) => {
                    if tx.send(Some(l)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        // stdin closed: the app is gone.
        let _ = tx.send(None);
    });

    emit(&json!({"type": "hello", "protocol": 1, "version": env!("CARGO_PKG_VERSION")}));

    let mut scanner = Scanner::new();
    let mut agents: Vec<Agent> = Vec::new();
    let mut last_emit = Instant::now() - HEARTBEAT;
    let mut next_scan = Instant::now();

    loop {
        if Instant::now() >= next_scan {
            let fresh = scanner.scan();
            if fresh != agents || last_emit.elapsed() >= HEARTBEAT {
                agents = fresh;
                emit(&json!({"type": "snapshot", "generatedAt": now_ms(), "agents": agents}));
                last_emit = Instant::now();
            }
            next_scan = Instant::now() + SCAN_EVERY;
        }

        let wait = next_scan.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(Some(line)) => match serde_json::from_str::<Request>(&line) {
                Ok(Request::Refresh) => next_scan = Instant::now(),
                Ok(Request::Reply { request_id, agent_id, text }) => {
                    let result = match agents.iter().find(|a| a.id == agent_id) {
                        None => Err(("unknown_agent", format!("no agent {agent_id}"))),
                        Some(a) => match &a.tmux {
                            None => Err(("not_reachable", "agent is not in a tmux pane".to_string())),
                            Some(pane) => tmux::send(pane, &text).map_err(|e| ("send_failed", format!("{e:#}"))),
                        },
                    };
                    match result {
                        Ok(()) => emit(&json!({"type": "replyResult", "requestId": request_id, "ok": true})),
                        Err((code, message)) => emit(&json!({
                            "type": "replyResult", "requestId": request_id, "ok": false,
                            "error": code, "message": message
                        })),
                    }
                    next_scan = Instant::now() + Duration::from_millis(300);
                }
                Err(e) => eprintln!("lantern-engine: bad request {line:?}: {e}"),
            },
            Ok(None) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}
