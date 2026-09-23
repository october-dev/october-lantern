//! Lantern as an October host computer for the October phone app.
//!
//! One thread owns the relay WebSocket (`wss://relay.afteroctober.xyz/v1/host/{hostId}`) and
//! every phone session on it, like October Desktop's `remote-relay-client.ts` + `remote-pairing.ts`:
//! relay frames are acknowledged, each phone connection runs a Noise_XX handshake (Lantern
//! responds, the phone's key is pinned to October's records), pairing shows a 6-digit code to
//! compare, and paired phones authenticate with a credential Lantern issued and then send
//! October core requests, answered from Lantern's agents (`api.rs`).

use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde_json::{Value, json};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use super::control::{self, Binding, ControlError};
use super::frames::{self, Assembler, Kind};
use super::noise::{self, Channel, Responder, Step};
use super::store::{self, HostIdentity};
use super::api;
use crate::model::Agent;

pub enum Cmd {
    /// A fresh October access token (after sign-in or refresh).
    Token(String),
    /// Start pairing a phone: creates a pairing and shows its QR.
    Pair,
    /// Allow or deny the phone showing the pairing code.
    Decide(bool),
    Revoke(String),
    Stop,
}

pub struct MobileHost {
    tx: Sender<Cmd>,
    agents: Arc<Mutex<Vec<Agent>>>,
}

impl MobileHost {
    pub fn start(access_token: String, emit: fn(&Value)) -> MobileHost {
        let (tx, rx) = channel();
        let agents = Arc::new(Mutex::new(Vec::new()));
        let shared = agents.clone();
        std::thread::spawn(move || Host::new(access_token, rx, shared, emit).run());
        MobileHost { tx, agents }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn update_agents(&self, agents: &[Agent]) {
        if let Ok(mut a) = self.agents.lock() {
            *a = agents.to_vec();
        }
    }
}

enum Stage {
    Handshake(Responder),
    Pairing(Channel),
    Active { channel: Channel, credential: Option<String>, subscribed: bool },
}

struct Session {
    connection_id: u64,
    binding: Binding,
    stage: Option<Stage>,
    assembler: Assembler,
    last: Instant,
}

struct PairingState {
    intent_id: String,
    qr: String,
    expires_at: u64,
    /// The phone that finished the handshake: (bind, 6-digit code, label).
    code: Option<(String, String, String)>,
    /// After Allow: waiting for the phone to confirm it stored its credential.
    awaiting_ack: Option<(String, i64, Instant)>,
}

struct Host {
    id: HostIdentity,
    token: String,
    rx: Receiver<Cmd>,
    agents: Arc<Mutex<Vec<Agent>>>,
    emit: fn(&Value),
    ws: Option<WebSocket<MaybeTlsStream<TcpStream>>>,
    sessions: HashMap<String, Session>,
    pairing: Option<PairingState>,
    status: &'static str,
    message: Option<String>,
    next_attempt: Instant,
    backoff: u32,
    /// Don't reconnect until something changes (e.g. the plan doesn't include mobile).
    parked: bool,
    cursor: u64,
    instance_id: String,
    process_start: String,
    last_digest: String,
    last_heartbeat: Instant,
}

impl Host {
    fn new(token: String, rx: Receiver<Cmd>, agents: Arc<Mutex<Vec<Agent>>>, emit: fn(&Value)) -> Host {
        let id = HostIdentity::load_or_create().expect("phone identity");
        Host {
            id,
            token,
            rx,
            agents,
            emit,
            ws: None,
            sessions: HashMap::new(),
            pairing: None,
            status: "offline",
            message: None,
            next_attempt: Instant::now(),
            backoff: 0,
            parked: false,
            cursor: crate::hooks::now_ms(),
            instance_id: uuid::Uuid::new_v4().hyphenated().to_string(),
            process_start: control::chrono_now(),
            last_digest: String::new(),
            last_heartbeat: Instant::now(),
        }
    }

    fn publish(&self) {
        let devices: Vec<Value> = store::load_devices()
            .iter()
            .map(|d| json!({"bind": d.bind, "label": d.label, "platform": d.platform, "pairedAt": d.paired_at}))
            .collect();
        let pairing = self.pairing.as_ref().map(|p| {
            json!({
                "qr": p.qr, "expiresAt": p.expires_at,
                "code": p.code.as_ref().map(|(_, c, _)| c.clone()),
                "label": p.code.as_ref().map(|(_, _, l)| l.clone()),
                "finishing": p.awaiting_ack.is_some(),
            })
        });
        (self.emit)(&json!({
            "type": "phone", "status": self.status, "message": self.message, "hostId": self.id.host_id,
            "devices": devices, "pairing": pairing,
        }));
    }

    fn run(mut self) {
        self.publish();
        loop {
            while let Ok(cmd) = self.rx.try_recv() {
                if matches!(cmd, Cmd::Stop) {
                    self.disconnect("offline");
                    self.publish();
                    return;
                }
                if let Err(e) = self.command(cmd) {
                    self.message = Some(describe(&e));
                }
                self.publish();
            }
            let wanted = !self.parked && (self.id.registered || self.pairing.is_some()) && !store::load_devices().is_empty() || self.pairing.is_some();
            if self.ws.is_none() && wanted && Instant::now() >= self.next_attempt {
                self.connect();
                self.publish();
            }
            if self.ws.is_some() {
                self.read_once();
                self.tick();
            } else {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }

    // MARK: Commands from the app

    fn command(&mut self, cmd: Cmd) -> Result<()> {
        match cmd {
            Cmd::Token(t) => {
                self.token = t;
                if self.parked && self.status == "signed-out" {
                    self.parked = false;
                }
            }
            Cmd::Pair => self.start_pairing()?,
            Cmd::Decide(allow) => self.decide(allow)?,
            Cmd::Revoke(bind) => {
                let body = json!({"hostId": self.id.host_id, "bind": bind});
                let result = control::signed_post("mobile-device-revoke", "device-revoke", &body, &self.id, &self.token);
                store::remove_device(&bind)?;
                self.close_session(&bind, frames::CLOSE_REVOKED);
                result?;
            }
            Cmd::Stop => {}
        }
        Ok(())
    }

    fn start_pairing(&mut self) -> Result<()> {
        let machine = if machine_model().contains("Book") { "laptop" } else { "desktop" };
        let body = json!({
            "hostId": self.id.host_id, "hostSignPub": self.id.sign_public(), "hostStaticPub": self.id.static_public,
            "name": computer_name(), "platform": "mac", "machine": machine,
        });
        let r = control::signed_post("mobile-pair-create", "pair-create", &body, &self.id, &self.token)?;
        let (Some(intent_id), Some(secret)) = (r["intentId"].as_str(), r["secret"].as_str()) else { bail!("pairing response is invalid") };
        let expires_at = r["expiresAt"].as_u64().unwrap_or(crate::hooks::now_ms() + 300_000);
        let qr_payload = json!({"v": 2, "hostId": self.id.host_id, "hostStatic": self.id.static_public, "intentId": intent_id, "secret": secret, "exp": expires_at});
        let qr = format!("https://october.dev/pair#{}", B64.encode(serde_json::to_vec(&qr_payload)?));
        self.pairing = Some(PairingState { intent_id: intent_id.into(), qr, expires_at, code: None, awaiting_ack: None });
        if !self.id.registered {
            self.id.registered = true;
            self.id.save()?;
        }
        self.parked = false;
        self.message = None;
        if self.ws.is_none() {
            self.next_attempt = Instant::now();
        }
        Ok(())
    }

    fn decide(&mut self, allow: bool) -> Result<()> {
        let Some(p) = self.pairing.as_mut() else { bail!("no phone is waiting") };
        let Some((bind, _, _)) = p.code.clone() else { bail!("no phone is waiting") };
        let Some(session) = self.sessions.get(&bind) else { bail!("the phone disconnected") };
        let b = session.binding.clone();
        let body = json!({
            "intentId": p.intent_id, "bind": b.bind, "deviceStaticPub": b.device_static_raw, "deviceSignPub": b.device_sign_pub,
            "decision": if allow { "approve" } else { "deny" }, "stateVersion": b.state_version.unwrap_or(0),
        });
        let r = control::signed_post("mobile-pair-decide", "pair-decide", &body, &self.id, &self.token)?;
        if !allow {
            self.pairing = None;
            self.close_session(&bind, frames::CLOSE_ENDED);
            return Ok(());
        }
        let state_version = r["stateVersion"].as_i64().unwrap_or(0);
        let credential = store::add_device(&bind, &b.label, &b.platform)?;
        if let Some(p) = self.pairing.as_mut() {
            p.awaiting_ack = Some((bind.clone(), state_version, Instant::now()));
        }
        self.send_frame(&bind, Kind::PairCredential, &serde_json::to_vec(&json!({"credential": credential}))?, 0);
        Ok(())
    }

    fn finish_pairing(&mut self, bind: &str) -> Result<()> {
        let Some(p) = self.pairing.as_ref() else { return Ok(()) };
        let Some((expected, state_version, _)) = p.awaiting_ack.clone() else { return Ok(()) };
        if expected != bind {
            return Ok(());
        }
        let body = json!({"intentId": p.intent_id, "stateVersion": state_version});
        let r = control::signed_post("mobile-pair-finalize", "pair-finalize", &body, &self.id, &self.token)?;
        let final_version = r["stateVersion"].as_i64().unwrap_or(state_version);
        self.send_frame(bind, Kind::PairActive, &serde_json::to_vec(&json!({"bind": bind, "stateVersion": final_version}))?, 0);
        self.pairing = None;
        self.message = None;
        Ok(())
    }

    // MARK: Relay connection

    fn connect(&mut self) {
        self.status = "connecting";
        let result = (|| -> Result<WebSocket<MaybeTlsStream<TcpStream>>> {
            let ticket = control::relay_ticket(&self.id, &self.token)?;
            let mut request = format!("{}/v1/host/{}", control::RELAY, self.id.host_id).into_client_request()?;
            request.headers_mut().insert("Sec-WebSocket-Protocol", format!("october-ticket.{ticket}").parse()?);
            let (mut ws, _) = tungstenite::connect(request)?;
            let timeout = Some(Duration::from_millis(200));
            match ws.get_mut() {
                MaybeTlsStream::Plain(s) => s.set_read_timeout(timeout)?,
                MaybeTlsStream::Rustls(s) => s.get_mut().set_read_timeout(timeout)?,
                _ => {}
            }
            Ok(ws)
        })();
        match result {
            Ok(ws) => {
                self.ws = Some(ws);
                self.status = "connected";
                self.message = None;
                self.backoff = 0;
            }
            Err(e) => {
                if let Some(c) = e.downcast_ref::<ControlError>() {
                    match c.code.as_deref() {
                        Some("plan_required") => return self.park("plan-required", "Your October plan doesn't include the phone app."),
                        Some("SESSION_REQUIRED") => return self.park("signed-out", "Sign in to October again to reconnect your phone."),
                        Some("HOST_INACTIVE") => return self.park("offline", "Pair a phone to connect."),
                        _ => {}
                    }
                }
                self.status = "offline";
                self.message = Some(describe(&e));
                self.schedule_reconnect();
            }
        }
    }

    fn park(&mut self, status: &'static str, message: &str) {
        self.status = status;
        self.message = Some(message.into());
        self.parked = true;
    }

    fn schedule_reconnect(&mut self) {
        let secs = (1u64 << self.backoff.min(5)).min(30);
        self.backoff += 1;
        self.next_attempt = Instant::now() + Duration::from_secs(secs);
    }

    fn disconnect(&mut self, status: &'static str) {
        if let Some(mut ws) = self.ws.take() {
            let _ = ws.close(None);
        }
        self.sessions.clear();
        self.status = status;
    }

    fn read_once(&mut self) {
        let Some(ws) = self.ws.as_mut() else { return };
        match ws.read() {
            Ok(Message::Binary(bytes)) => {
                if let Err(e) = self.receive_outer(&bytes) {
                    eprintln!("lantern phone: {e:#}");
                }
            }
            Ok(Message::Close(frame)) => {
                let code: u16 = frame.map(|f| f.code.into()).unwrap_or(1000);
                self.ws = None;
                self.sessions.clear();
                match code {
                    4409 => self.park("offline", "Another copy of Lantern connected with this identity."),
                    4403 | 4401 => self.park("offline", "October closed the connection. Pair the phone again."),
                    4408 => self.park("plan-required", "Your October plan doesn't include the phone app."),
                    _ => {
                        self.status = "connecting";
                        self.schedule_reconnect();
                    }
                }
                self.publish();
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => {
                self.ws = None;
                self.sessions.clear();
                self.status = "connecting";
                self.message = Some(format!("Lost the connection: {e}"));
                self.schedule_reconnect();
                self.publish();
            }
        }
    }

    fn send_outer(&mut self, bytes: Vec<u8>) {
        if let Some(ws) = self.ws.as_mut() {
            if ws.send(Message::binary(bytes)).is_err() {
                self.ws = None;
                self.schedule_reconnect();
            }
        }
    }

    fn close_session(&mut self, bind: &str, code: u16) {
        if let Some(s) = self.sessions.remove(bind) {
            if let Ok(f) = frames::encode_outer(frames::OUTER_CLOSE, bind, s.connection_id, &code.to_be_bytes()) {
                self.send_outer(f);
            }
        }
    }

    fn receive_outer(&mut self, bytes: &[u8]) -> Result<()> {
        let outer = frames::decode_outer(bytes)?;
        self.send_outer(frames::host_ack(outer.connection_id, bytes.len()));
        match outer.kind {
            frames::OUTER_OPEN => {
                // Who is this? Ask October, never the phone.
                let Some(binding) = control::resolve_binding(&self.id.host_id, &outer.bind, &self.token)? else {
                    self.sessions.remove(&outer.bind);
                    let f = frames::encode_outer(frames::OUTER_CLOSE, &outer.bind, outer.connection_id, &frames::CLOSE_AUTH_FAILED.to_be_bytes())?;
                    self.send_outer(f);
                    return Ok(());
                };
                let p = noise::prologue(&self.id.host_id, &outer.bind, outer.connection_id)?;
                let responder = Responder::new(&self.id.static_secret(), &p)?;
                self.sessions.insert(outer.bind.clone(), Session {
                    connection_id: outer.connection_id,
                    binding,
                    stage: Some(Stage::Handshake(responder)),
                    assembler: Assembler::default(),
                    last: Instant::now(),
                });
            }
            frames::OUTER_CLOSE => {
                self.sessions.remove(&outer.bind);
            }
            frames::OUTER_DATA => {
                if let Err(e) = self.receive_data(&outer.bind, outer.connection_id, &outer.payload) {
                    eprintln!("lantern phone session {}: {e:#}", outer.bind);
                    self.close_session(&outer.bind, frames::CLOSE_AUTH_FAILED);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn receive_data(&mut self, bind: &str, connection_id: u64, payload: &[u8]) -> Result<()> {
        let Some(session) = self.sessions.get_mut(bind) else { return Ok(()) };
        if session.connection_id != connection_id {
            return Ok(());
        }
        session.last = Instant::now();
        let stage = session.stage.take();
        match stage {
            Some(Stage::Handshake(r)) => match r.read(payload)? {
                Step::Reply(r, message) => {
                    session.stage = Some(Stage::Handshake(r));
                    let f = frames::encode_outer(frames::OUTER_DATA, bind, connection_id, &message)?;
                    self.send_outer(f);
                }
                Step::Done(channel, remote, hash) => {
                    if !noise::pinned(&remote, &session.binding.device_static) {
                        bail!("the phone's key doesn't match October's records");
                    }
                    if session.binding.pairing {
                        session.stage = Some(Stage::Pairing(channel));
                        let label = session.binding.label.clone();
                        match self.pairing.as_mut() {
                            Some(p) if session.binding.intent_id.as_deref() == Some(p.intent_id.as_str()) => {
                                p.code = Some((bind.to_string(), noise::pairing_code(&hash), label));
                                self.publish();
                            }
                            _ => bail!("no pairing is waiting for this phone"),
                        }
                    } else {
                        session.stage = Some(Stage::Active { channel, credential: None, subscribed: false });
                    }
                }
            },
            Some(Stage::Pairing(mut channel)) => {
                let plain = channel.decrypt(payload)?;
                session.stage = Some(Stage::Pairing(channel));
                if let Some((kind, id, _)) = session.assembler.push(&plain)? {
                    match kind {
                        Kind::Ping => self.send_frame(bind, Kind::Pong, &[], id),
                        Kind::PairOffer => {}
                        Kind::PairAck => {
                            if let Err(e) = self.finish_pairing(bind) {
                                self.message = Some(describe(&e));
                            }
                            self.publish();
                        }
                        _ => bail!("unexpected frame during pairing"),
                    }
                }
            }
            Some(Stage::Active { mut channel, credential, subscribed }) => {
                let plain = channel.decrypt(payload)?;
                let frame = session.assembler.push(&plain)?;
                session.stage = Some(Stage::Active { channel, credential: credential.clone(), subscribed });
                if let Some((kind, id, data)) = frame {
                    self.active_frame(bind, credential, kind, id, data)?;
                }
            }
            None => {}
        }
        Ok(())
    }

    fn active_frame(&mut self, bind: &str, credential: Option<String>, kind: Kind, id: u32, data: Vec<u8>) -> Result<()> {
        let Some(credential) = credential else {
            // The first frame must prove the phone holds the credential Lantern issued it.
            let presented = String::from_utf8(data).unwrap_or_default();
            if kind != Kind::Auth || presented.len() < 32 || presented.len() > 1024 || !store::credential_ok(bind, &presented) {
                bail!("authentication failed");
            }
            if let Some(Session { stage: Some(Stage::Active { credential, .. }), .. }) = self.sessions.get_mut(bind) {
                *credential = Some(presented);
            }
            return Ok(());
        };
        match kind {
            Kind::Req => {
                let request: Value = serde_json::from_slice(&data)?;
                let agents = self.agents.lock().map(|a| a.clone()).unwrap_or_default();
                let ctx = api::Context {
                    agents: &agents, canvas_id: &self.id.canvas_id, host_id: &self.id.host_id,
                    instance_id: &self.instance_id, process_start: &self.process_start, credential: &credential,
                };
                let (status, body, action) = api::handle(&ctx, &request);
                let res = frames::encode_response(status, crate::hooks::now_ms(), &serde_json::to_vec(&body)?);
                self.send_frame(bind, Kind::Res, &res, id);
                if let api::Action::Deliver { agent_id, text } = action {
                    if let Some(a) = agents.iter().find(|a| a.id == agent_id) {
                        if let Err(e) = crate::deliver::send_text(a, &text) {
                            eprintln!("lantern phone: couldn't deliver to {agent_id}: {e:#}");
                        }
                    }
                }
            }
            Kind::Sub => {
                let requested = serde_json::from_slice::<Value>(&data).ok().and_then(|v| v["cursor"].as_u64()).unwrap_or(0);
                if let Some(Session { stage: Some(Stage::Active { subscribed, .. }), .. }) = self.sessions.get_mut(bind) {
                    *subscribed = true;
                }
                // Lantern keeps no event history: always start the phone from a fresh snapshot.
                let reset = json!({"requested": requested, "oldest": self.cursor + 1, "snapshotTopics": ["bus.changed", "terminal.state", "ui.request", "facts.changed"]});
                self.event_to(bind, "cursor.reset", "", reset);
                self.event_to(bind, "core.lifecycle", "", json!({"state": "ready"}));
            }
            Kind::Unsub => {
                if let Some(Session { stage: Some(Stage::Active { subscribed, .. }), .. }) = self.sessions.get_mut(bind) {
                    *subscribed = false;
                }
            }
            Kind::Ping => self.send_frame(bind, Kind::Pong, &[], id),
            Kind::Cancel => {}
            _ => bail!("frame not allowed in an active session"),
        }
        Ok(())
    }

    /// Encrypts and sends one inner message (chunked) to a phone.
    fn send_frame(&mut self, bind: &str, kind: Kind, data: &[u8], message_id: u32) {
        let Some(session) = self.sessions.get_mut(bind) else { return };
        let connection_id = session.connection_id;
        let channel = match session.stage.as_mut() {
            Some(Stage::Pairing(c)) => c,
            Some(Stage::Active { channel, .. }) => channel,
            _ => return,
        };
        let mut out = Vec::new();
        for chunk in frames::encode_message(kind, data, message_id) {
            match channel.encrypt(&chunk).and_then(|c| frames::encode_outer(frames::OUTER_DATA, bind, connection_id, &c)) {
                Ok(f) => out.push(f),
                Err(_) => return,
            }
        }
        for f in out {
            self.send_outer(f);
        }
    }

    fn next_cursor(&mut self) -> u64 {
        self.cursor = (self.cursor + 1).max(crate::hooks::now_ms());
        self.cursor
    }

    fn event_to(&mut self, bind: &str, topic: &str, entity: &str, payload: Value) {
        let cursor = self.next_cursor();
        let line = json!({"apiVersion": 2, "cursor": cursor, "topic": topic, "entityId": entity, "generation": 1, "createdAt": crate::hooks::now_ms(), "payload": payload});
        self.send_frame(bind, Kind::Ev, format!("{line}\n").as_bytes(), 0);
    }

    fn broadcast(&mut self, topic: &str, entity: &str, payload: Value) {
        let binds: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, s)| matches!(s.stage, Some(Stage::Active { subscribed: true, .. })))
            .map(|(b, _)| b.clone())
            .collect();
        for b in binds {
            self.event_to(&b, topic, entity, payload.clone());
        }
    }

    /// Periodic work: tell phones when agents change, heartbeats, timeouts.
    fn tick(&mut self) {
        let agents = self.agents.lock().map(|a| a.clone()).unwrap_or_default();
        let digest = agents
            .iter()
            .map(|a| format!("{}|{:?}|{:?}|{:?}", a.id, a.state, a.state_since, a.title))
            .collect::<Vec<_>>()
            .join(";");
        if digest != self.last_digest {
            self.last_digest = digest;
            let canvas = self.id.canvas_id.clone();
            self.broadcast("bus.changed", &canvas, json!({"operation": "snapshot"}));
            self.broadcast("facts.changed", &canvas, json!({"operation": "needs-input", "canvasId": canvas}));
        }
        if self.last_heartbeat.elapsed() >= Duration::from_secs(15) {
            self.last_heartbeat = Instant::now();
            self.broadcast("cursor.heartbeat", "", json!({}));
        }
        let idle: Vec<String> = self.sessions.iter().filter(|(_, s)| s.last.elapsed() >= Duration::from_secs(60)).map(|(b, _)| b.clone()).collect();
        for b in idle {
            self.close_session(&b, frames::CLOSE_ENDED);
        }
        if let Some(p) = &self.pairing {
            let ack_timed_out = p.awaiting_ack.as_ref().is_some_and(|(_, _, at)| at.elapsed() >= Duration::from_secs(60));
            if ack_timed_out || crate::hooks::now_ms() > p.expires_at + 60_000 {
                if let Some((bind, _, _)) = p.awaiting_ack.clone() {
                    let _ = store::remove_device(&bind);
                }
                self.pairing = None;
                self.message = Some(if ack_timed_out { "The phone didn't finish pairing. Try again." } else { "The pairing code expired." }.into());
                self.publish();
            }
        }
    }
}

fn describe(e: &anyhow::Error) -> String {
    if let Some(c) = e.downcast_ref::<ControlError>() {
        return match c.code.as_deref() {
            Some("plan_required") => "Your October plan doesn't include the phone app.".into(),
            Some("SESSION_REQUIRED") => "Sign in to October again.".into(),
            _ => format!("October: {}", c.message),
        };
    }
    format!("{e:#}")
}

fn computer_name() -> String {
    std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| format!("{s} (Lantern)"))
        .unwrap_or_else(|| "Mac (Lantern)".into())
}

fn machine_model() -> String {
    std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.model"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}
