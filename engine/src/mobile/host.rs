//! Lantern as an October host computer for the October phone app.
//!
//! One thread owns the relay WebSocket (`wss://relay.afteroctober.xyz/v1/host/{hostId}`) and
//! every phone session on it, like October Desktop's `remote-relay-client.ts` + `remote-pairing.ts`:
//! relay frames are acknowledged, each phone connection runs a Noise_XX handshake (Lantern
//! responds, the phone's key is pinned to October's records), pairing shows a 6-digit code to
//! compare, and paired phones authenticate with a credential Lantern issued and then send
//! October core requests, answered from Lantern's agents (`api.rs`). Replies are typed by the
//! serve loop's delivery workers (the one place that delivers). The phone hears delivered, not
//! delivered (canceled before typing), or not confirmed; see `Delivery`.

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

use super::api;
use super::control::{self, Binding, ControlError};
use super::frames::{self, Assembler, Kind};
use super::noise::{self, Channel, Responder, Step};
use super::push::{self, Attention};
use super::store::{self, HostIdentity};
use crate::actions::{Outcome, Ticket};
use crate::model::Agent;
use crate::serve::{Deliver, Incoming};

#[derive(Clone)]
pub enum Cmd {
    /// A fresh October access token (after sign-in or refresh).
    Token(String),
    /// Start pairing a phone: creates a pairing and shows its QR.
    Pair,
    /// Stop pairing (the QR stops working; a phone mid-pairing is dropped).
    CancelPair,
    /// Allow or deny the phone showing the pairing code.
    Decide {
        intent_id: String,
        bind: String,
        code: String,
        allow: bool,
    },
    Retry,
    Revoke(String),
    Stop,
}

pub struct MobileHost {
    tx: Sender<Cmd>,
    agents: Arc<Mutex<Vec<Agent>>>,
}

/// How long a phone's reply may wait to start typing (less when the phone's own deadline is
/// sooner). Replies that haven't started by then are canceled.
const DELIVER_TIMEOUT: Duration = Duration::from_secs(20);
/// Once typing has started, how long to wait for its outcome before calling it uncertain.
const STARTED_GRACE: Duration = Duration::from_secs(15);
/// Connecting to the relay, its TLS and WebSocket handshakes, and any single write.
const CONNECT_LIMIT: Duration = Duration::from_secs(10);
const MAX_PENDING_DELIVERIES: usize = 16;
/// Answers remembered per (phone, idempotency key), so a retried mutation isn't typed twice.
const REMEMBERED_ANSWERS: usize = 256;

impl MobileHost {
    /// `deliver` reaches the serve loop, which types replies. A host that can't load its
    /// identity reports that once and stops.
    pub fn start(access_token: String, emit: fn(&Value), deliver: Sender<Incoming>) -> MobileHost {
        let (tx, rx) = channel();
        let agents = Arc::new(Mutex::new(Vec::new()));
        let shared = agents.clone();
        std::thread::spawn(move || match Host::new(access_token, rx, shared, emit, deliver) {
            Ok(host) => host.run(),
            Err(e) => emit(&json!({"type": "phone", "status": "error", "message": format!("{e:#}"), "errorCode": "host_unavailable",
                    "busy": false, "hostId": null, "devices": [], "pairing": null, "installUrl": install_url()})),
        });
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

#[allow(clippy::large_enum_variant)] // One per phone connection; the handshake state is the big one.
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

#[derive(Clone)]
struct Acknowledgement {
    bind: String,
    version: i64,
    at: Instant,
    credential: String,
}

struct PairingState {
    intent_id: String,
    qr: String,
    expires_at: u64,
    /// The phone that finished the handshake: (bind, 6-digit code, label).
    code: Option<(String, String, String)>,
    /// After Allow: waiting for the phone to confirm it stored its credential.
    awaiting_ack: Option<Acknowledgement>,
    ack_received: bool,
}

impl PairingState {
    fn acknowledge(&mut self, bind: &str) -> bool {
        if self.awaiting_ack.as_ref().is_some_and(|ack| ack.bind == bind) {
            self.ack_received = true;
            true
        } else {
            false
        }
    }
}

struct Host {
    id: HostIdentity,
    token: String,
    rx: Receiver<Cmd>,
    agents: Arc<Mutex<Vec<Agent>>>,
    emit: fn(&Value),
    deliver: Sender<Incoming>,
    ws: Option<WebSocket<MaybeTlsStream<TcpStream>>>,
    /// devices.json's modification time when last read, and whether it listed any phone.
    devices_seen: (Option<std::time::SystemTime>, bool),
    sessions: HashMap<String, Session>,
    pairing: Option<PairingState>,
    status: &'static str,
    message: Option<String>,
    error_code: Option<&'static str>,
    outcome: Option<&'static str>,
    busy: bool,
    retry: Option<Cmd>,
    next_attempt: Instant,
    backoff: u32,
    /// Don't reconnect until something changes (e.g. the plan doesn't include mobile).
    parked: bool,
    cursor: u64,
    instance_id: String,
    process_start: String,
    last_digest: String,
    last_heartbeat: Instant,
    /// Answers already given to mutations, by (phone, idempotency key).
    answered: api::Answers,
    pending: Vec<PendingReply>,
    /// Which agents' attention has been pushed to phones.
    attention: Attention,
}

impl Host {
    fn new(token: String, rx: Receiver<Cmd>, agents: Arc<Mutex<Vec<Agent>>>, emit: fn(&Value), deliver: Sender<Incoming>) -> Result<Host> {
        Ok(Self::with_identity(HostIdentity::load_or_create()?, token, rx, agents, emit, deliver))
    }

    fn with_identity(
        id: HostIdentity,
        token: String,
        rx: Receiver<Cmd>,
        agents: Arc<Mutex<Vec<Agent>>>,
        emit: fn(&Value),
        deliver: Sender<Incoming>,
    ) -> Host {
        Host {
            id,
            token,
            rx,
            agents,
            emit,
            deliver,
            ws: None,
            devices_seen: (None, false),
            sessions: HashMap::new(),
            pairing: None,
            status: "offline",
            message: None,
            error_code: None,
            outcome: None,
            busy: false,
            retry: None,
            next_attempt: Instant::now(),
            backoff: 0,
            parked: false,
            cursor: crate::hooks::now_ms(),
            instance_id: uuid::Uuid::new_v4().hyphenated().to_string(),
            process_start: control::now_iso(),
            last_digest: String::new(),
            last_heartbeat: Instant::now(),
            answered: api::Answers::new(REMEMBERED_ANSWERS),
            pending: Vec::new(),
            attention: Attention::new(crate::hooks::now_ms()),
        }
    }

    fn publish(&self) {
        let devices: Vec<Value> = store::load_devices()
            .iter()
            .map(|d| {
                json!({"bind": d.bind, "label": d.label, "platform": d.platform, "pairedAt": d.paired_at,
                "lastSeenAt": d.last_seen_at, "removalPending": d.removal_pending})
            })
            .collect();
        let pairing = self.pairing.as_ref().map(|p| {
            json!({
                "intentId": p.intent_id, "bind": p.code.as_ref().map(|(b, _, _)| b.clone()),
                "qr": if p.code.is_none() { Some(&p.qr) } else { None }, "expiresAt": p.expires_at,
                "code": p.code.as_ref().map(|(_, c, _)| c.clone()),
                "label": p.code.as_ref().map(|(_, _, l)| l.clone()),
                "platform": p.code.as_ref().and_then(|(b, _, _)| self.sessions.get(b).map(|s| s.binding.platform.clone())),
                "finishing": p.awaiting_ack.is_some(),
            })
        });
        (self.emit)(&json!({
            "type": "phone", "status": self.status, "message": self.message, "hostId": self.id.host_id,
            "devices": devices, "pairing": pairing, "errorCode": self.error_code,
            "outcome": self.outcome, "busy": self.busy,
            "installUrl": install_url(),
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
                if matches!(cmd, Cmd::Token(_)) {
                    let _ = self.command(cmd);
                    self.publish();
                    continue;
                }
                let retry = if matches!(cmd, Cmd::Retry) { self.retry.clone() } else { Some(cmd.clone()) };
                self.busy = true;
                self.error_code = None;
                self.message = None;
                self.publish();
                if let Err(e) = self.command(cmd) {
                    self.problem(&e);
                    self.retry = retry;
                } else {
                    self.retry = None;
                }
                self.busy = false;
                self.publish();
            }
            self.finish_deliveries();
            self.expire_pairing();
            self.push_attention();
            // Connect while there is a phone to serve or one being paired.
            let wanted = !self.parked && ((self.id.registered && self.has_devices()) || self.pairing.is_some());
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

    /// Whether any phone is paired; the file is only read again when it changes.
    fn has_devices(&mut self) -> bool {
        let modified = std::fs::metadata(store::devices_path()).and_then(|m| m.modified()).ok();
        if self.devices_seen.0 != modified || modified.is_none() {
            self.devices_seen = (modified, !store::load_devices().is_empty());
        }
        self.devices_seen.1
    }

    /// Pushes agents that started wanting the person to paired phones (connected to the relay
    /// or not: the push goes through Expo).
    fn push_attention(&mut self) {
        if !self.has_devices() {
            return;
        }
        let agents = self.agents.lock().map(|a| a.clone()).unwrap_or_default();
        for episode in self.attention.due(&agents, Instant::now(), crate::hooks::now_ms()) {
            push::send(episode, self.id.host_id.clone(), self.id.canvas_id.clone());
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
            Cmd::CancelPair => {
                self.clear_pairing();
                self.outcome = None;
                self.message = None;
            }
            Cmd::Decide { intent_id, bind, code, allow } => {
                if !self
                    .pairing
                    .as_ref()
                    .is_some_and(|p| p.intent_id == intent_id && p.code.as_ref().is_some_and(|(b, c, _)| b == &bind && c == &code))
                {
                    bail!("This pairing request is no longer active. Start again.");
                }
                self.decide(allow)?;
            }
            Cmd::Retry => {
                if let Some(command) = self.retry.take() {
                    return self.command(command);
                }
                if let Some(bind) =
                    self.pairing.as_ref().filter(|p| p.ack_received).and_then(|p| p.awaiting_ack.as_ref().map(|ack| ack.bind.clone()))
                {
                    self.finish_pairing(&bind)?;
                } else if self.pairing.is_none() {
                    self.start_pairing()?;
                }
                self.parked = false;
                self.next_attempt = Instant::now();
            }
            Cmd::Revoke(bind) => {
                self.cancel_deliveries(Some(&bind));
                let body = json!({"hostId": self.id.host_id, "bind": bind});
                // Block locally first, including push, even if October is unreachable. Retain
                // the row until cloud removal succeeds so the user has a visible Retry action.
                store::block_device(&bind)?;
                self.close_session(&bind, frames::CLOSE_REVOKED);
                control::signed_post("mobile-device-revoke", "device-revoke", &body, &self.id, &self.token)?;
                store::remove_device(&bind)?;
                self.outcome = Some("removed");
            }
            Cmd::Stop => {}
        }
        Ok(())
    }

    fn start_pairing(&mut self) -> Result<()> {
        self.clear_pairing();
        self.outcome = None;
        let machine = if machine_model().contains("Book") { "laptop" } else { "desktop" };
        let body = json!({
            "hostId": self.id.host_id, "hostSignPub": self.id.sign_public(), "hostStaticPub": self.id.static_public,
            "name": computer_name(), "platform": host_platform(), "machine": machine,
        });
        let r = control::signed_post("mobile-pair-create", "pair-create", &body, &self.id, &self.token)?;
        let (intent_id, qr, expires_at) = pairing_link(&self.id, &r, crate::hooks::now_ms())?;
        self.pairing = Some(PairingState { intent_id, qr, expires_at, code: None, awaiting_ack: None, ack_received: false });
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
        if p.expires_at <= crate::hooks::now_ms() || p.awaiting_ack.is_some() {
            bail!("This pairing request is no longer awaiting approval.");
        }
        let Some((bind, _, _)) = p.code.clone() else { bail!("no phone is waiting") };
        let Some(session) = self.sessions.get(&bind) else { bail!("the phone disconnected") };
        let b = session.binding.clone();
        let body = json!({
            "intentId": p.intent_id, "bind": b.bind, "deviceStaticPub": b.device_static_raw, "deviceSignPub": b.device_sign_pub,
            "decision": if allow { "approve" } else { "deny" },
            "stateVersion": b.state_version.ok_or_else(|| anyhow::anyhow!("pairing is missing its server state version"))?,
        });
        let r = match control::signed_post("mobile-pair-decide", "pair-decide", &body, &self.id, &self.token) {
            Ok(r) => r,
            Err(error) => {
                if error.downcast_ref::<ControlError>().is_some_and(|c| c.status < 500 && c.status != 409) {
                    return Err(error);
                }
                let state = control::pairing_state(&self.id.host_id, &p.intent_id, &self.token).ok().flatten();
                match state.filter(|s| decision_committed(s, &b, allow)) {
                    Some(s) => json!({"stateVersion": s["state_version"]}),
                    None => return Err(error),
                }
            }
        };
        if !allow {
            self.pairing = None;
            self.outcome = Some("denied");
            self.close_session(&bind, frames::CLOSE_ENDED);
            return Ok(());
        }
        let state_version = r["stateVersion"].as_i64().ok_or_else(|| anyhow::anyhow!("invalid decision response"))?;
        let credential = store::new_credential()?;
        if let Some(p) = self.pairing.as_mut() {
            p.awaiting_ack =
                Some(Acknowledgement { bind: bind.clone(), version: state_version, at: Instant::now(), credential: credential.clone() });
        }
        self.send_frame(&bind, Kind::PairCredential, &serde_json::to_vec(&json!({"credential": credential}))?, 0);
        Ok(())
    }

    fn finish_pairing(&mut self, bind: &str) -> Result<()> {
        let Some(p) = self.pairing.as_ref() else { return Ok(()) };
        let Some(ack) = p.awaiting_ack.clone() else { return Ok(()) };
        if ack.bind != bind || !p.ack_received {
            return Ok(());
        }
        let body = json!({"intentId": p.intent_id, "stateVersion": ack.version});
        let r = control::signed_post("mobile-pair-finalize", "pair-finalize", &body, &self.id, &self.token)?;
        if r["state"] != "active" {
            bail!("pairing did not become active");
        }
        let final_version = r["stateVersion"].as_i64().ok_or_else(|| anyhow::anyhow!("invalid finalization response"))?;
        let session = self.sessions.get(bind).ok_or_else(|| anyhow::anyhow!("the phone disconnected before finalization"))?;
        store::add_device(bind, &session.binding.label, &session.binding.platform, &ack.credential)?;
        self.send_frame(bind, Kind::PairActive, &serde_json::to_vec(&json!({"bind": bind, "stateVersion": final_version}))?, 0);
        if self.ws.is_none() {
            store::remove_device(bind)?;
            bail!("the phone disconnected before pairing completed");
        }
        self.pairing = None;
        self.message = None;
        self.error_code = None;
        self.outcome = Some("paired");
        Ok(())
    }

    fn clear_pairing(&mut self) {
        if let Some(p) = self.pairing.take() {
            let binds: Vec<_> = self
                .sessions
                .iter()
                .filter(|(_, s)| s.binding.intent_id.as_deref() == Some(&p.intent_id))
                .map(|(b, _)| b.clone())
                .collect();
            for bind in binds {
                self.close_session(&bind, frames::CLOSE_ENDED);
            }
        }
    }

    fn problem(&mut self, error: &anyhow::Error) {
        let (code, message) = phone_problem(error);
        self.error_code = Some(code);
        self.message = Some(message.into());
        if code == "plan_required" {
            self.parked = true;
            self.status = "plan-required";
        }
        if code == "SESSION_REQUIRED" {
            self.parked = true;
            self.status = "signed-out";
        }
    }

    /// Pairings run out whether or not the relay is connected.
    fn expire_pairing(&mut self) {
        let Some(p) = &self.pairing else { return };
        let ack_timed_out = p.awaiting_ack.as_ref().is_some_and(|ack| ack.at.elapsed() >= Duration::from_secs(60));
        // The QR expires at the advertised deadline. An approved phone has its separate
        // 60-second acknowledgement/finalization deadline, as on October Desktop.
        if !ack_timed_out && (p.awaiting_ack.is_some() || crate::hooks::now_ms() < p.expires_at) {
            return;
        }
        self.clear_pairing();
        self.outcome = Some("expired");
        self.error_code = Some("PAIRING_EXPIRED");
        self.message = Some(
            if ack_timed_out { "The phone didn't finish pairing in time. Create a new code." } else { "This pairing code has expired." }
                .into(),
        );
        self.publish();
    }

    // MARK: Relay connection

    fn connect(&mut self) {
        self.status = "connecting";
        let result = (|| -> Result<WebSocket<MaybeTlsStream<TcpStream>>> {
            let ticket = control::relay_ticket(&self.id, &self.token)?;
            let mut request = format!("{}/v1/host/{}", control::RELAY, self.id.host_id).into_client_request()?;
            request.headers_mut().insert("Sec-WebSocket-Protocol", format!("october-ticket.{ticket}").parse()?);
            // Every step has a time limit: a relay that accepts the connection and then says
            // nothing must not hold up the host.
            let uri = request.uri();
            let host = uri.host().ok_or_else(|| anyhow::anyhow!("relay address has no host"))?;
            let port = uri.port_u16().unwrap_or(443);
            let addr = std::net::ToSocketAddrs::to_socket_addrs(&(host, port))?
                .next()
                .ok_or_else(|| anyhow::anyhow!("couldn't resolve {host}"))?;
            let stream = TcpStream::connect_timeout(&addr, CONNECT_LIMIT)?;
            stream.set_read_timeout(Some(CONNECT_LIMIT))?;
            stream.set_write_timeout(Some(CONNECT_LIMIT))?;
            let (mut ws, _) = tungstenite::client_tls(request, stream).map_err(|e| anyhow::anyhow!("connecting to the relay: {e}"))?;
            // Short reads keep the loop responsive; writes that stall past the limit drop the socket.
            let tcp = match ws.get_mut() {
                MaybeTlsStream::Plain(s) => s,
                MaybeTlsStream::Rustls(s) => s.get_mut(),
                _ => bail!("unexpected relay stream"),
            };
            tcp.set_read_timeout(Some(Duration::from_millis(200)))?;
            tcp.set_write_timeout(Some(CONNECT_LIMIT))?;
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
                        Some("plan_required") => return self.park("plan-required", "Your plan does not currently include mobile access."),
                        Some("SESSION_REQUIRED") => return self.park("signed-out", "Sign in to October again to reconnect your phone."),
                        Some("HOST_INACTIVE") => return self.park("offline", "Pair a phone to connect."),
                        _ => {}
                    }
                }
                self.status = "offline";
                self.problem(&e);
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
        self.cancel_deliveries(None);
        self.sessions.clear();
        self.clear_confirmation(None);
        self.status = status;
    }

    /// The socket is gone: every phone session with it. Reconnect after a pause.
    fn lost(&mut self, message: String) {
        self.ws = None;
        self.cancel_deliveries(None);
        self.sessions.clear();
        self.clear_confirmation(None);
        self.status = "connecting";
        self.message = Some(message);
        self.schedule_reconnect();
        self.publish();
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
                self.cancel_deliveries(None);
                self.sessions.clear();
                self.clear_confirmation(None);
                match code {
                    4409 => self.park("offline", "Another copy of Lantern connected with this identity."),
                    4403 | 4401 => self.park("offline", "October closed the connection. Pair the phone again."),
                    4408 => self.park("plan-required", "Your plan does not currently include mobile access."),
                    _ => {
                        self.status = "connecting";
                        self.schedule_reconnect();
                    }
                }
                self.publish();
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => self.lost(format!("Lost the connection: {e}")),
        }
    }

    fn send_outer(&mut self, bytes: Vec<u8>) {
        if let Some(ws) = self.ws.as_mut()
            && let Err(e) = ws.send(Message::binary(bytes))
        {
            self.lost(format!("Lost the connection: {e}"));
        }
    }

    fn clear_confirmation(&mut self, bind: Option<&str>) {
        if self.pairing.as_ref().is_some_and(|p| p.code.as_ref().is_some_and(|(b, _, _)| bind.is_none_or(|bind| b == bind))) {
            self.pairing = None;
            self.retry = None;
            self.error_code = Some("PAIRING_DISCONNECTED");
            self.message = Some("The phone disconnected before pairing finished. Create a new code.".into());
        }
    }

    fn close_session(&mut self, bind: &str, code: u16) {
        self.clear_confirmation(Some(bind));
        self.cancel_deliveries(Some(bind));
        if let Some(s) = self.sessions.remove(bind)
            && let Ok(f) = frames::encode_outer(frames::OUTER_CLOSE, bind, s.connection_id, &code.to_be_bytes())
        {
            self.send_outer(f);
        }
    }

    fn receive_outer(&mut self, bytes: &[u8]) -> Result<()> {
        let outer = frames::decode_outer(bytes)?;
        self.send_outer(frames::host_ack(outer.connection_id, bytes.len()));
        match outer.kind {
            frames::OUTER_OPEN => {
                self.cancel_deliveries(Some(&outer.bind));
                self.clear_confirmation(Some(&outer.bind));
                // Who is this? Ask October, never the phone.
                let Some(binding) = control::resolve_binding(&self.id.host_id, &outer.bind, &self.token)? else {
                    self.sessions.remove(&outer.bind);
                    let f = frames::encode_outer(
                        frames::OUTER_CLOSE,
                        &outer.bind,
                        outer.connection_id,
                        &frames::CLOSE_AUTH_FAILED.to_be_bytes(),
                    )?;
                    self.send_outer(f);
                    return Ok(());
                };
                let p = noise::prologue(&self.id.host_id, &outer.bind, outer.connection_id)?;
                let responder = Responder::new(&self.id.static_secret(), &p)?;
                self.sessions.insert(
                    outer.bind.clone(),
                    Session {
                        connection_id: outer.connection_id,
                        binding,
                        stage: Some(Stage::Handshake(responder)),
                        assembler: Assembler::default(),
                        last: Instant::now(),
                    },
                );
            }
            frames::OUTER_CLOSE => {
                if self.sessions.get(&outer.bind).is_some_and(|s| s.connection_id == outer.connection_id) {
                    self.cancel_deliveries(Some(&outer.bind));
                    self.sessions.remove(&outer.bind);
                    self.clear_confirmation(Some(&outer.bind));
                    self.publish();
                }
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
                            if !self.pairing.as_mut().is_some_and(|p| p.acknowledge(bind)) {
                                bail!("no credential is awaiting acknowledgement from this phone");
                            }
                            if let Err(e) = self.finish_pairing(bind) {
                                self.problem(&e);
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
            store::seen(bind, crate::hooks::now_ms())?;
            self.publish();
            return Ok(());
        };
        if !store::credential_ok(bind, &credential) {
            bail!("this phone's access was removed");
        }
        store::seen(bind, crate::hooks::now_ms())?;
        match kind {
            Kind::Req => {
                let request: Value = serde_json::from_slice(&data)?;
                let remembered = request["idempotencyKey"].as_str().map(|k| format!("{bind}|{k}"));
                // A retry gets the first answer (in a fresh envelope for this request) and nothing
                // is typed again; the same key on a different request is refused.
                if let Some((status, body)) = remembered.as_ref().and_then(|k| self.answered.replay(k, &request)) {
                    let res = frames::encode_response(status, crate::hooks::now_ms(), &serde_json::to_vec(&body)?);
                    self.send_frame(bind, Kind::Res, &res, id);
                    return Ok(());
                }
                let connection_id = self.sessions.get(bind).map(|s| s.connection_id).unwrap_or(0);
                if let Some(pending) =
                    self.pending.iter_mut().find(|p| p.bind == bind && remembered.is_some() && p.remembered == remembered)
                {
                    if pending.request["method"] == request["method"] && pending.request["payload"] == request["payload"] {
                        if !pending.waiters.iter().any(|w| w.connection_id == connection_id && w.frame == id) {
                            if pending.waiters.len() >= 8 {
                                bail!("too many retries for one pending reply");
                            }
                            pending.waiters.push(ReplyTo { connection_id, frame: id, request_id: request["requestId"].clone() });
                        }
                        return Ok(());
                    }
                    let (status, body) =
                        api::err(&request["requestId"], "INVALID_ARGUMENT", "this idempotency key belongs to a different pending request");
                    let res = frames::encode_response(status, crate::hooks::now_ms(), &serde_json::to_vec(&body)?);
                    self.send_frame(bind, Kind::Res, &res, id);
                    return Ok(());
                }
                if self
                    .pending
                    .iter()
                    .any(|p| p.bind == bind && p.waiters.iter().any(|w| w.connection_id == connection_id && w.frame == id))
                {
                    bail!("request id already in flight");
                }
                let agents = self.agents.lock().map(|a| a.clone()).unwrap_or_default();
                let push = |change: api::PushChange| match change {
                    api::PushChange::Register(token) => store::register_push(bind, &token),
                    api::PushChange::Unregister => store::unregister_push(bind),
                    api::PushChange::Mute(muted) => store::mute_push(bind, muted),
                };
                let ctx = api::Context {
                    agents: &agents,
                    canvas_id: &self.id.canvas_id,
                    host_id: &self.id.host_id,
                    instance_id: &self.instance_id,
                    process_start: &self.process_start,
                    credential: &credential,
                    push: &push,
                };
                let mut to_deliver = None;
                let mut typed = |a: &Agent, text: &str, deadline_ms: Option<u64>| {
                    to_deliver = Some((a.clone(), text.to_string(), deadline_ms));
                    Outcome::Queued
                };
                let (mut status, mut body) = api::handle(&ctx, &request, &mut typed);
                if let Some((agent, text, deadline)) = to_deliver {
                    if self.pending.len() >= MAX_PENDING_DELIVERIES {
                        (status, body) = api::err(&request["requestId"], "BUSY", "Too many replies are pending. Nothing was sent.");
                    } else {
                        self.pending.push(PendingReply {
                            bind: bind.to_string(),
                            waiters: vec![ReplyTo { connection_id, frame: id, request_id: request["requestId"].clone() }],
                            request,
                            remembered,
                            delivery: Delivery::start(&self.deliver, &agent, &text, deadline),
                        });
                        return Ok(());
                    }
                }
                let res = frames::encode_response(status, crate::hooks::now_ms(), &serde_json::to_vec(&body)?);
                if let Some(key) = remembered {
                    self.answered.remember(key, &request, status, body);
                }
                self.send_frame(bind, Kind::Res, &res, id);
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
            Kind::Cancel => {
                let connection_id = self.sessions.get(bind).map(|s| s.connection_id);
                for p in self
                    .pending
                    .iter()
                    .filter(|p| p.bind == bind && p.waiters.iter().any(|w| Some(w.connection_id) == connection_id && w.frame == id))
                {
                    p.delivery.ticket.cancel();
                }
            }
            _ => bail!("frame not allowed in an active session"),
        }
        Ok(())
    }

    fn cancel_deliveries(&self, bind: Option<&str>) {
        for p in self.pending.iter().filter(|p| bind.is_none_or(|b| b == p.bind)) {
            p.delivery.ticket.cancel();
        }
    }

    fn finish_deliveries(&mut self) {
        let mut completed = Vec::new();
        for (index, pending) in self.pending.iter_mut().enumerate() {
            if let Some(outcome) = pending.delivery.poll(Instant::now()) {
                completed.push((index, outcome));
            }
        }
        for (index, outcome) in completed.into_iter().rev() {
            let pending = self.pending.remove(index);
            let (status, body) = api::delivery_result(&pending.request["requestId"], outcome);
            if let Some(key) = pending.remembered {
                self.answered.remember(key, &pending.request, status, body.clone());
            }
            for waiter in pending.waiters {
                if self.sessions.get(&pending.bind).is_none_or(|s| s.connection_id != waiter.connection_id) {
                    continue;
                }
                let mut response = body.clone();
                response["requestId"] = waiter.request_id;
                if let Ok(bytes) = serde_json::to_vec(&response) {
                    let res = frames::encode_response(status, crate::hooks::now_ms(), &bytes);
                    self.send_frame(&pending.bind, Kind::Res, &res, waiter.frame);
                }
            }
        }
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

    /// Periodic work while connected: tell phones when agents change, heartbeats, idle sessions.
    fn tick(&mut self) {
        let agents = self.agents.lock().map(|a| a.clone()).unwrap_or_default();
        let digest =
            agents.iter().map(|a| format!("{}|{:?}|{:?}|{:?}", a.id, a.state, a.state_since, a.title)).collect::<Vec<_>>().join(";");
        if digest != self.last_digest {
            self.last_digest = digest;
            let canvas = self.id.canvas_id.clone();
            self.broadcast("bus.changed", &canvas, json!({"operation": "snapshot"}));
            self.broadcast("facts.changed", &canvas, json!({"operation": "needs-input", "canvasId": canvas}));
        }
        if self.last_heartbeat.elapsed() >= Duration::from_secs(15) {
            self.last_heartbeat = Instant::now();
            self.broadcast("cursor.heartbeat", "", json!({}));
            self.publish();
        }
        let idle: Vec<String> =
            self.sessions.iter().filter(|(_, s)| s.last.elapsed() >= Duration::from_secs(60)).map(|(b, _)| b.clone()).collect();
        for b in idle {
            self.close_session(&b, frames::CLOSE_ENDED);
        }
    }
}

struct ReplyTo {
    connection_id: u64,
    frame: u32,
    request_id: Value,
}

struct PendingReply {
    bind: String,
    waiters: Vec<ReplyTo>,
    request: Value,
    remembered: Option<String>,
    delivery: Delivery,
}

/// Polled by the socket owner. No wait for a terminal operation runs on that thread.
struct Delivery {
    ticket: Arc<Ticket>,
    result: Receiver<Outcome>,
    until: Instant,
    started_grace: bool,
}

impl Delivery {
    fn start(deliver: &Sender<Incoming>, a: &Agent, text: &str, deadline_ms: Option<u64>) -> Self {
        let now = crate::hooks::now_ms();
        let wait_for = deadline_ms.map(|d| Duration::from_millis(d.saturating_sub(now))).unwrap_or(DELIVER_TIMEOUT).min(DELIVER_TIMEOUT);
        let ticket = Ticket::new();
        let (done, result) = channel();
        let _ = deliver.send(Incoming::Deliver(Deliver {
            agent_id: a.id.clone(),
            text: text.to_string(),
            deadline: Instant::now() + wait_for,
            ticket: ticket.clone(),
            done,
        }));
        Self { ticket, result, until: Instant::now() + wait_for + Duration::from_millis(500), started_grace: false }
    }

    fn poll(&mut self, now: Instant) -> Option<Outcome> {
        match self.result.try_recv() {
            Ok(outcome) => return Some(outcome),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Some(if self.ticket.cancel() {
                    Outcome::failed("send_failed", "Lantern's delivery worker stopped before typing")
                } else {
                    Outcome::Uncertain("Lantern's delivery worker stopped after typing began".into())
                });
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        if self.ticket.canceled() {
            return Some(Outcome::failed("canceled", "Canceled before Lantern typed it."));
        }
        if now < self.until {
            return None;
        }
        if self.started_grace {
            return Some(Outcome::Uncertain("typing started but didn't finish in time".into()));
        }
        if self.ticket.cancel() {
            return Some(Outcome::failed("expired", "Lantern couldn't type it in time. Nothing was typed."));
        }
        self.started_grace = true;
        self.until = now + STARTED_GRACE;
        None
    }
}

impl Drop for Delivery {
    fn drop(&mut self) {
        self.ticket.cancel();
    }
}

#[cfg(test)]
pub(crate) fn deliver_and_wait(deliver: &Sender<Incoming>, a: &Agent, text: &str, deadline_ms: Option<u64>) -> Outcome {
    let mut pending = Delivery::start(deliver, a, text, deadline_ms);
    loop {
        if let Some(outcome) = pending.poll(Instant::now()) {
            return outcome;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// One shared, non-secret app install destination for both native UIs.
fn install_url() -> Value {
    serde_json::from_str::<Value>(include_str!("../../../protocol/phone-app.json"))
        .ok()
        .and_then(|v| v.get("installUrl").cloned())
        .unwrap_or(Value::Null)
}

fn host_platform() -> &'static str {
    if cfg!(windows) {
        "win"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "mac"
    }
}

/// The fixed build-37 phone contract. Never substitute a guessed expiry or log this URL.
fn pairing_link(id: &HostIdentity, response: &Value, now: u64) -> Result<(String, String, u64)> {
    let intent = response["intentId"].as_str().filter(|s| canonical_uuid(s)).ok_or_else(|| anyhow::anyhow!("invalid pairing intent"))?;
    let secret = response["secret"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| anyhow::anyhow!("missing pairing secret"))?;
    let expiry = response["expiresAt"]
        .as_u64()
        .filter(|n| *n > now && *n <= 9_007_199_254_740_991)
        .ok_or_else(|| anyhow::anyhow!("invalid pairing expiry"))?;
    if !canonical_uuid(&id.host_id) || store::decode_key(&id.static_public).is_none() {
        bail!("invalid host identity");
    }
    let payload =
        json!({"v": 2, "hostId": id.host_id, "hostStatic": id.static_public, "intentId": intent, "secret": secret, "exp": expiry});
    Ok((intent.into(), format!("https://october.dev/pair#{}", B64.encode(serde_json::to_vec(&payload)?)), expiry))
}

fn decision_committed(state: &Value, b: &Binding, allow: bool) -> bool {
    state["state"] == if allow { "approved" } else { "denied" }
        && state["bind"] == b.bind
        && state["device_static_pub"] == b.device_static_raw
        && state["device_sign_pub"] == b.device_sign_pub
        && state["state_version"].as_i64().is_some_and(|version| b.state_version.is_some_and(|old| version > old))
}

fn canonical_uuid(raw: &str) -> bool {
    uuid::Uuid::parse_str(raw).is_ok_and(|id| {
        id.hyphenated().to_string() == raw && matches!(id.get_version_num(), 1..=5) && id.get_variant() == uuid::Variant::RFC4122
    })
}

fn phone_problem(e: &anyhow::Error) -> (&'static str, &'static str) {
    if let Some(c) = e.downcast_ref::<ControlError>() {
        return match c.code.as_deref() {
            Some("plan_required") => ("plan_required", "Your plan does not currently include mobile access."),
            Some("SESSION_REQUIRED") => ("SESSION_REQUIRED", "Sign in to October again to connect your phone."),
            Some("PAIRING_REJECTED") => ("PAIRING_REJECTED", "Use the same October account on your phone and this computer."),
            Some("PAIRING_EXPIRED") => ("PAIRING_EXPIRED", "This pairing code has expired."),
            Some("PAIRING_USED") => ("PAIRING_USED", "A phone already used this pairing code."),
            Some("PAIRING_DENIED") => ("PAIRING_DENIED", "This pairing request was denied."),
            Some("plan_authority_unavailable") => ("plan_authority_unavailable", "October couldn't check your plan right now. Try again."),
            _ if c.status == 503 || c.status >= 500 => {
                ("service_unavailable", "October's phone service is unavailable right now. Try again.")
            }
            _ => ("request_failed", "October couldn't complete this request. Try again."),
        };
    }
    ("service_unavailable", "Couldn't connect to October's phone service. Check your connection and try again.")
}

fn computer_name() -> String {
    let mut cmd = std::process::Command::new(if cfg!(target_os = "macos") { "/usr/sbin/scutil" } else { "hostname" });
    if cfg!(target_os = "macos") {
        cmd.args(["--get", "ComputerName"]);
    }
    crate::run::output(&mut cmd, Duration::from_secs(3))
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| format!("{s} (Lantern)"))
        .unwrap_or_else(|| format!("{} (Lantern)", std::env::consts::OS))
}

fn machine_model() -> String {
    if cfg!(windows) {
        // No `uname` on Windows; the architecture is what `uname -m` gives elsewhere.
        return std::env::consts::ARCH.into();
    }
    let mut cmd = std::process::Command::new(if cfg!(target_os = "macos") { "/usr/sbin/sysctl" } else { "uname" });
    cmd.args(if cfg!(target_os = "macos") { &["-n", "hw.model"][..] } else { &["-m"][..] });
    crate::run::output(&mut cmd, Duration::from_secs(3)).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod delivery_tests {
    use super::*;

    fn host(deliver: Sender<Incoming>, agent: Agent) -> Host {
        let identity = serde_json::from_value(json!({"hostId": "host", "signSeed": B64.encode([0u8; 32]),
            "staticSecret": B64.encode([0u8; 32]), "staticPublic": B64.encode([0u8; 32]), "canvasId": "canvas"}))
        .unwrap();
        let (_, rx) = channel();
        let mut h = Host::with_identity(identity, String::new(), rx, Arc::new(Mutex::new(vec![agent])), |_| {}, deliver);
        for bind in ["phone-a", "phone-b"] {
            h.sessions.insert(
                bind.into(),
                Session {
                    connection_id: 1,
                    binding: Binding {
                        bind: bind.into(),
                        pairing: false,
                        device_static: [0; 32],
                        device_static_raw: String::new(),
                        device_sign_pub: String::new(),
                        label: String::new(),
                        platform: String::new(),
                        intent_id: None,
                        state_version: None,
                    },
                    stage: None,
                    assembler: Assembler::default(),
                    last: Instant::now(),
                },
            );
        }
        h
    }

    #[test]
    fn pending_retries_share_delivery_and_cancel_is_scoped_to_the_phone() {
        // This test now exercises the real stored-credential checks too. Run the fixture in
        // its own process/profile: never change process-wide environment during parallel tests
        // and never write a fake phone into the user's real pairing store.
        let profile = std::env::var_os("LANTERN_TEST_PROFILE").map(std::path::PathBuf::from);
        let isolated = profile.as_ref().is_some_and(|p| {
            p.starts_with(std::env::temp_dir())
                && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("lantern-phone-test-"))
                && std::env::var_os("LANTERN_PHONE_DELIVERY_TEST").as_deref() == Some(p.as_os_str())
        });
        if !isolated {
            let profile = std::env::temp_dir().join(format!("lantern-phone-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&profile).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "mobile::host::delivery_tests::pending_retries_share_delivery_and_cancel_is_scoped_to_the_phone"])
                .env("LANTERN_TEST_PROFILE", &profile)
                .env("LANTERN_PHONE_DELIVERY_TEST", &profile)
                .output();
            let _ = std::fs::remove_dir_all(&profile);
            let output = output.unwrap();
            assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return;
        }
        for bind in ["phone-a", "phone-b"] {
            store::add_device(bind, "Test phone", "ios", "credential").unwrap();
        }
        let (sender, incoming) = channel();
        let a = crate::tests::agent_for(123, 1, Some("ttys001"));
        let request = json!({"apiVersion": 2, "requestId": "first", "idempotencyKey": "k", "method": "bus.mutate",
            "payload": {"operation": "userSend", "args": ["canvas", {"id": api::node_id(&a.id), "kind": "terminal"}, "hello"]}});
        let mut h = host(sender, a);
        let send = |h: &mut Host, bind: &str, frame, request: &Value| {
            h.active_frame(bind, Some("credential".into()), Kind::Req, frame, serde_json::to_vec(request).unwrap()).unwrap();
        };
        send(&mut h, "phone-a", 1, &request);
        let Ok(Incoming::Deliver(first)) = incoming.try_recv() else { panic!("first not submitted") };
        let mut retry = request.clone();
        retry["requestId"] = json!("retry");
        send(&mut h, "phone-a", 2, &retry);
        assert_eq!(h.pending.len(), 1);
        assert_eq!(h.pending[0].waiters.len(), 2);
        assert!(incoming.try_recv().is_err(), "A pending retry must not type twice");
        send(&mut h, "phone-b", 1, &request);
        let Ok(Incoming::Deliver(second)) = incoming.try_recv() else { panic!("second not submitted") };
        h.active_frame("phone-b", Some("credential".into()), Kind::Cancel, 1, Vec::new()).unwrap();
        assert!(!second.ticket.start());
        assert!(first.ticket.start(), "Cancel must not affect a different phone using the same frame id");
        first.done.send(Outcome::Done).unwrap();
        h.finish_deliveries();
        assert!(h.pending.is_empty());
        let (_, body) = h.answered.replay("phone-a|k", &retry).unwrap();
        assert_eq!(body["requestId"], "retry");
        assert_eq!(body["result"]["delivery"], "delivered");
        let (_, body) = h.answered.replay("phone-b|k", &request).unwrap();
        assert_eq!(body["result"]["accepted"], false);
    }

    #[test]
    fn pending_delivery_is_polled_without_blocking_other_work() {
        let (sender, incoming) = channel();
        let a = crate::tests::agent_for(123, 1, Some("ttys001"));
        let mut pending = Delivery::start(&sender, &a, "test", None);
        let Ok(Incoming::Deliver(d)) = incoming.try_recv() else { panic!("not submitted") };
        assert!(pending.poll(Instant::now()).is_none());
        assert!(d.ticket.start());
        d.done.send(Outcome::Done).unwrap();
        assert_eq!(pending.poll(Instant::now()), Some(Outcome::Done));
    }

    #[test]
    fn cancel_and_disconnect_prevent_queued_typing() {
        let (sender, incoming) = channel();
        let a = crate::tests::agent_for(123, 1, Some("ttys001"));
        let mut pending = Delivery::start(&sender, &a, "test", None);
        let Ok(Incoming::Deliver(d)) = incoming.try_recv() else { panic!("not submitted") };
        assert!(pending.ticket.cancel());
        assert!(matches!(pending.poll(Instant::now()), Some(Outcome::Failed { code: "canceled", .. })));
        assert!(!d.ticket.start());
        let pending = Delivery::start(&sender, &a, "second", None);
        let Ok(Incoming::Deliver(d)) = incoming.try_recv() else { panic!("not submitted") };
        drop(pending);
        assert!(!d.ticket.start(), "Dropping a host must cancel deliveries that have not started");
    }

    #[test]
    fn a_started_delivery_expires_as_uncertain_not_failed() {
        let (sender, incoming) = channel();
        let a = crate::tests::agent_for(123, 1, Some("ttys001"));
        let mut pending = Delivery::start(&sender, &a, "test", None);
        let Ok(Incoming::Deliver(d)) = incoming.try_recv() else { panic!("not submitted") };
        assert!(d.ticket.start());
        let timeout = pending.until;
        assert!(pending.poll(timeout).is_none());
        assert!(matches!(pending.poll(timeout + STARTED_GRACE), Some(Outcome::Uncertain(_))));
    }
}

#[cfg(test)]
mod onboarding_tests {
    use super::*;
    fn identity() -> HostIdentity {
        serde_json::from_value(json!({"hostId":"11111111-1111-4111-8111-111111111111", "canvasId":"canvas",
            "signSeed":B64.encode([1;32]), "staticSecret":B64.encode([2;32]), "staticPublic":B64.encode([3;32])}))
        .unwrap()
    }
    fn response() -> Value {
        json!({"intentId":"22222222-2222-4222-8222-222222222222", "secret":B64.encode([4;32]), "expiresAt":300001})
    }
    fn host() -> Host {
        let (_, rx) = channel();
        let (deliver, _) = channel();
        Host::with_identity(identity(), String::new(), rx, Arc::new(Mutex::new(Vec::new())), |_| {}, deliver)
    }
    fn pairing(expiry: u64) -> PairingState {
        PairingState {
            intent_id: "intent".into(),
            qr: "in-memory-only".into(),
            expires_at: expiry,
            code: Some(("phone".into(), "123456".into(), "Test iPhone".into())),
            awaiting_ack: None,
            ack_received: false,
        }
    }
    #[test]
    fn qr_is_exactly_the_build_37_universal_link_contract() {
        let id = identity();
        let r = response();
        let (_, link, expiry) = pairing_link(&id, &r, 1).unwrap();
        let url = url::Url::parse(&link).unwrap();
        assert_eq!((url.scheme(), url.host_str(), url.path(), url.query()), ("https", Some("october.dev"), "/pair", None));
        let fragment = url.fragment().unwrap();
        assert!(!fragment.contains('='));
        let payload: Value = serde_json::from_slice(&B64.decode(fragment).unwrap()).unwrap();
        assert_eq!(
            payload,
            json!({"v":2,"hostId":id.host_id,"hostStatic":id.static_public,
            "intentId":r["intentId"],"secret":r["secret"],"exp":expiry})
        );
    }
    #[test]
    fn invalid_server_contract_is_refused_instead_of_guessing_an_expiry() {
        for (key, value) in [
            ("expiresAt", Value::Null),
            ("expiresAt", json!(1)),
            ("expiresAt", json!(9_007_199_254_740_992_u64)),
            ("intentId", json!("AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA")),
            ("intentId", json!("not-a-uuid")),
            ("secret", json!("")),
        ] {
            let mut r = response();
            r[key] = value;
            assert!(pairing_link(&identity(), &r, 1).is_err(), "invalid {key} accepted");
        }
    }
    #[test]
    fn errors_have_distinct_recovery_codes_and_never_echo_server_secrets() {
        for code in [
            "plan_required",
            "plan_authority_unavailable",
            "PAIRING_REJECTED",
            "PAIRING_EXPIRED",
            "PAIRING_USED",
            "PAIRING_DENIED",
            "SESSION_REQUIRED",
        ] {
            let e: anyhow::Error =
                ControlError { status: 403, code: Some(code.into()), message: "secret that must not appear".into() }.into();
            let (kind, message) = phone_problem(&e);
            assert_eq!(kind, code);
            assert!(!message.contains("secret"));
        }
        let e: anyhow::Error = ControlError { status: 503, code: None, message: "private".into() }.into();
        assert_eq!(phone_problem(&e).0, "service_unavailable");
    }
    #[test]
    fn code_expires_at_its_deadline_and_drops_the_pending_session() {
        let mut h = host();
        h.pairing = Some(pairing(0));
        h.expire_pairing();
        assert!(h.pairing.is_none());
        assert_eq!(h.error_code, Some("PAIRING_EXPIRED"));
        assert_eq!(h.outcome, Some("expired"));
    }
    #[test]
    fn only_the_approved_phone_can_acknowledge_its_credential() {
        let mut p = pairing(u64::MAX);
        assert!(!p.acknowledge("phone"), "approval must precede acknowledgement");
        p.awaiting_ack = Some(Acknowledgement { bind: "phone".into(), version: 2, at: Instant::now(), credential: "ephemeral".into() });
        assert!(!p.acknowledge("other-phone"));
        assert!(!p.ack_received);
        assert!(p.acknowledge("phone"));
        assert!(p.ack_received);
    }

    #[test]
    fn stale_approval_is_rejected_before_any_network_or_credential_write() {
        let mut h = host();
        h.pairing = Some(pairing(crate::hooks::now_ms() + 60_000));
        assert!(h.command(Cmd::Decide { intent_id: "intent".into(), bind: "phone".into(), code: "654321".into(), allow: true }).is_err());
        assert!(h.pairing.as_ref().unwrap().awaiting_ack.is_none());
    }
    #[test]
    fn another_phones_disconnect_cannot_clear_the_current_confirmation() {
        let mut h = host();
        h.pairing = Some(pairing(u64::MAX));
        h.clear_confirmation(Some("other-phone"));
        assert!(h.pairing.is_some());
        h.clear_confirmation(Some("phone"));
        assert!(h.pairing.is_none());
        assert_eq!(h.error_code, Some("PAIRING_DISCONNECTED"));
    }
    #[test]
    fn decision_recovery_requires_the_same_bind_keys_and_newer_server_version() {
        let b = Binding {
            bind: "phone".into(),
            pairing: true,
            device_static: [0; 32],
            device_static_raw: "static".into(),
            device_sign_pub: "sign".into(),
            label: "iPhone".into(),
            platform: "ios".into(),
            intent_id: Some("intent".into()),
            state_version: Some(1),
        };
        let state = json!({"state":"approved","state_version":2,"bind":"phone","device_static_pub":"static","device_sign_pub":"sign"});
        assert!(decision_committed(&state, &b, true));
        assert!(!decision_committed(&state, &b, false));
        for (key, value) in [
            ("bind", json!("other")),
            ("device_static_pub", json!("other")),
            ("device_sign_pub", json!("other")),
            ("state_version", json!(1)),
        ] {
            let mut bad = state.clone();
            bad[key] = value;
            assert!(!decision_committed(&bad, &b, true));
        }
    }
}
