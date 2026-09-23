//! Connecting to a running October Desktop (its `october-core` runtime).
//!
//! Discovery: `~/Library/Application Support/October/core/runtime/core-run.json` (v3), written by
//! core with the loopback address and credentials. Transport: HTTP/1.1 on 127.0.0.1, `POST
//! /command` with a JSON envelope, `Host` equal to the address, no `Origin`, `Authorization:
//! Bearer <credential>`.
//!
//! Two modes:
//! - **Paired** (full): October approved Lantern ("October Lantern wants to connect") and issued
//!   it a revocable credential for the `local-client` principal. Lantern can list October's
//!   agents with names and attention, deliver replies through October's safe delivery
//!   (`bus.mutate userSend`), and focus agents on the canvas.
//! - **Read-only**: before pairing (or with an October version without pairing), Lantern uses
//!   the credential October gives its own command-line tool, and only lists terminals and agents.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, KeyInit, Mac};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::Sha256;

use crate::hooks::now_ms;
use crate::transcripts::home;

const API_VERSION: u64 = 2;

fn runtime_dir() -> PathBuf {
    home().join("Library/Application Support/October/core/runtime")
}

pub fn installed() -> bool {
    std::path::Path::new("/Applications/October.app").exists() || home().join("Library/Application Support/October").exists()
}

#[derive(Debug, Clone)]
pub struct RunFile {
    pub address: String,
    pub instance_id: String,
    pub process_start: String,
    pub core_version: String,
    pub generation: u64,
    cli_credential: String,
}

/// Reads and checks `core-run.json`; `None` when core isn't running.
pub fn discover() -> Option<RunFile> {
    let path = runtime_dir().join("core-run.json");
    let meta = std::fs::symlink_metadata(&path).ok()?;
    use std::os::unix::fs::PermissionsExt;
    if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 {
        return None;
    }
    let v: Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    if v["v"] != 3 || v["apiMin"].as_u64()? > API_VERSION || v["apiMax"].as_u64()? < API_VERSION {
        return None;
    }
    let address = v["address"].as_str()?.to_string();
    let port_ok = address.strip_prefix("127.0.0.1:").and_then(|p| p.parse::<u16>().ok()).is_some();
    let pid = v["pid"].as_i64()? as i32;
    // The process must still be alive.
    if !port_ok || unsafe { libc::kill(pid, 0) } != 0 {
        return None;
    }
    Some(RunFile {
        address,
        instance_id: v["instanceId"].as_str()?.to_string(),
        process_start: v["processStart"].as_str().unwrap_or("").to_string(),
        core_version: v["coreVersion"].as_str().unwrap_or("").to_string(),
        generation: v["generation"].as_u64().unwrap_or(0),
        cli_credential: v["cliCredential"].as_str()?.to_string(),
    })
}

#[derive(Debug, Clone)]
pub struct Principal {
    pub kind: &'static str,
    pub id: String,
    pub credential: String,
}

pub struct Client {
    pub run: RunFile,
    pub principal: Principal,
    agent: ureq2::Agent,
    next: u64,
}

impl Client {
    /// A paired client when a credential is given, otherwise read-only.
    pub fn new(run: RunFile, paired: Option<(String, String)>) -> Client {
        let principal = match paired {
            Some((id, credential)) => Principal { kind: "local-client", id, credential },
            None => Principal { kind: "cli", id: "october-lantern".into(), credential: run.cli_credential.clone() },
        };
        let agent = ureq2::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
        Client { run, principal, agent, next: 0 }
    }

    pub fn paired(&self) -> bool {
        self.principal.kind == "local-client"
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.run.address, path)
    }

    pub fn call(&mut self, method: &str, payload: Value, mutation: bool) -> Result<Value> {
        self.next += 1;
        let request_id = format!("lantern-{}-{}", now_ms(), self.next);
        let mut env = json!({
            "apiVersion": API_VERSION,
            "requestId": request_id,
            "deadlineAt": now_ms() + 10_000,
            "principal": {"kind": self.principal.kind, "id": self.principal.id},
            "method": method,
            "payload": payload,
        });
        if mutation {
            env["idempotencyKey"] = json!(format!("{request_id}-m"));
        }
        let resp = self
            .agent
            .post(&self.url("/command"))
            .set("Host", &self.run.address)
            .set("Authorization", &format!("Bearer {}", self.principal.credential))
            .set("Content-Type", "application/json")
            .send_string(&env.to_string());
        let body: Value = match resp {
            Ok(r) => r.into_json()?,
            Err(ureq2::Error::Status(code, r)) => {
                let v: Value = r.into_json().unwrap_or(json!({}));
                let e = &v["error"];
                if e["details"]["revoked"] == true || code == 401 {
                    bail!("revoked: {}", e["message"].as_str().unwrap_or("not authorized"));
                }
                bail!("{}: {}", e["code"].as_str().unwrap_or("error"), e["message"].as_str().unwrap_or("request failed"));
            }
            Err(e) => return Err(anyhow!(e)).context("October isn't reachable"),
        };
        if body["ok"] != true {
            bail!("{}: {}", body["error"]["code"].as_str().unwrap_or("error"), body["error"]["message"].as_str().unwrap_or(""));
        }
        Ok(body["result"].clone())
    }

    /// `core.handshake`, checking core's HMAC proof and that it's the instance in core-run.json.
    pub fn handshake(&mut self) -> Result<Value> {
        let mut challenge = [0u8; 32];
        getrandom(&mut challenge);
        let challenge = B64.encode(challenge);
        let r = self.call(
            "core.handshake",
            json!({"challenge": challenge, "clientVersion": format!("lantern-{}", env!("CARGO_PKG_VERSION")), "apiMin": API_VERSION, "apiMax": API_VERSION}),
            false,
        )?;
        let instance = r["instanceId"].as_str().unwrap_or("");
        let start = r["processStart"].as_str().unwrap_or("");
        if instance != self.run.instance_id
            || start != self.run.process_start
            || r["generation"].as_u64().is_some_and(|g| g != self.run.generation)
        {
            bail!("October core changed while connecting");
        }
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(self.principal.credential.as_bytes())
            .map_err(|_| anyhow!("bad key"))?;
        mac.update(format!("{challenge}:{instance}:{start}:{API_VERSION}").as_bytes());
        let proof = B64.decode(r["challengeProof"].as_str().unwrap_or("")).unwrap_or_default();
        mac.verify_slice(&proof).map_err(|_| anyhow!("October core failed the identity check"))?;
        Ok(r)
    }

    // ---------- pairing (needs an October version with companion pairing) ----------

    pub fn pair_request(&self, client_id: &str) -> Result<(String, String)> {
        let v = self.unauthenticated("/pair/request", json!({"clientId": client_id, "name": "October Lantern", "version": env!("CARGO_PKG_VERSION")}))?;
        Ok((
            v["requestId"].as_str().context("no requestId")?.to_string(),
            v["code"].as_str().context("no code")?.to_string(),
        ))
    }

    /// `(state, credential)`; the credential arrives once, with state "approved".
    pub fn pair_poll(&self, client_id: &str, request_id: &str) -> Result<(String, Option<String>)> {
        let v = self.unauthenticated("/pair/poll", json!({"clientId": client_id, "requestId": request_id}))?;
        Ok((v["state"].as_str().unwrap_or("pending").to_string(), v["credential"].as_str().map(String::from)))
    }

    fn unauthenticated(&self, path: &str, body: Value) -> Result<Value> {
        match self.agent.post(&self.url(path)).set("Host", &self.run.address).set("Content-Type", "application/json").send_string(&body.to_string()) {
            Ok(r) => Ok(r.into_json()?),
            Err(ureq2::Error::Status(404, _)) => bail!("unsupported: this version of October can't pair with Lantern yet"),
            Err(ureq2::Error::Status(code, r)) => {
                let v: Value = r.into_json().unwrap_or(json!({}));
                bail!("{} ({code})", v["error"]["message"].as_str().or(v["message"].as_str()).unwrap_or("pairing failed"))
            }
            Err(e) => Err(anyhow!(e)).context("October isn't reachable"),
        }
    }

    // ---------- reading agents ----------

    /// October's agents: named canvas nodes when paired, bare terminals when read-only.
    pub fn agents(&mut self) -> Result<Vec<OctoberAgent>> {
        let mut out = Vec::new();
        if self.paired() {
            let canvases = self.call("bus.query", json!({"operation": "listCanvases", "args": []}), false)?;
            for canvas in canvases.as_array().into_iter().flatten().filter_map(Value::as_str) {
                let snap = self.call("bus.query", json!({"operation": "currentSnapshot", "args": [canvas]}), false)?;
                for node in snap["nodes"].as_object().into_iter().flat_map(|m| m.values()) {
                    let state = node["execution"]["state"].as_str().unwrap_or(match node["status"].as_str() {
                        Some("live") => "working",
                        Some("idle") => "idle",
                        _ => "offline",
                    });
                    if state == "offline" {
                        continue;
                    }
                    out.push(OctoberAgent {
                        canvas_id: canvas.to_string(),
                        node_id: node["id"].as_str().unwrap_or("").to_string(),
                        kind: node["kind"].as_str().unwrap_or("terminal").to_string(),
                        name: node["displayName"].as_str().map(String::from),
                        harness: node["harness"].as_str().map(String::from),
                        cwd: node["cwd"].as_str().map(String::from),
                        state: state.to_string(),
                        attention: node["attention"]["message"].as_str().map(String::from),
                    });
                }
            }
        } else {
            let terminals = self.call("terminal.list", json!({}), false)?;
            for t in terminals.as_array().into_iter().flatten() {
                if t["state"].as_str().is_some_and(|s| s == "exited" || s == "dead") {
                    continue;
                }
                out.push(OctoberAgent {
                    canvas_id: t["canvasId"].as_str().unwrap_or("").to_string(),
                    node_id: t["id"].as_str().unwrap_or("").to_string(),
                    kind: "terminal".into(),
                    name: None,
                    harness: t["expectedHarness"].as_str().map(String::from),
                    cwd: t["cwd"].as_str().map(String::from),
                    state: "unknown".into(),
                    attention: None,
                });
            }
        }
        Ok(out)
    }

    /// Delivers a reply through October's own safe delivery (paired only).
    pub fn send(&mut self, a: &OctoberAgent, text: &str) -> Result<String> {
        if !self.paired() {
            bail!("connect Lantern to October first");
        }
        let text: String = text.chars().take(8000).collect();
        let node = json!({"id": a.node_id, "kind": "terminal", "displayName": a.name.clone().unwrap_or_default()});
        let r = self.call("bus.mutate", json!({"operation": "userSend", "args": [a.canvas_id, node, text]}), true)?;
        if r["accepted"] == false {
            bail!("October didn't accept it: {}", r["reason"].as_str().unwrap_or("unknown reason"));
        }
        Ok(r["delivery"].as_str().unwrap_or("queued").to_string())
    }

    /// Shows the agent on October's canvas (paired only), and brings October forward.
    pub fn focus(&mut self, a: &OctoberAgent) -> Result<()> {
        if self.paired() {
            let request_id = format!("lantern-focus-{}", now_ms());
            self.call(
                "ui.command.request",
                json!({"requestId": request_id, "canvasId": a.canvas_id, "action": "focus_node", "params": {"id": a.node_id}, "timeoutMs": 5000}),
                true,
            )?;
        }
        let _ = std::process::Command::new("/usr/bin/open").arg(format!("october://canvas/{}", a.canvas_id)).status();
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OctoberAgent {
    pub canvas_id: String,
    pub node_id: String,
    pub kind: String,
    pub name: Option<String>,
    pub harness: Option<String>,
    pub cwd: Option<String>,
    /// working | needs-user | idle | unknown
    pub state: String,
    pub attention: Option<String>,
}

fn getrandom(buf: &mut [u8]) {
    unsafe { libc::arc4random_buf(buf.as_mut_ptr().cast(), buf.len()) };
}
