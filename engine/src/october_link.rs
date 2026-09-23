//! Keeps a connection to October Desktop in the background: finds october-core, checks it,
//! reads its agents every few seconds, and runs pairing. The serve loop reads the shared state and
//! sends it commands; nothing here blocks the scan.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::hooks::support_dir;
use crate::october_core::{self, Client, OctoberAgent};

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkState {
    /// notInstalled | notRunning | readOnly | connected | pairing | error
    pub status: String,
    pub core_version: Option<String>,
    pub paired: bool,
    /// The 6-digit code October shows while it asks the user to allow Lantern.
    pub pairing_code: Option<String>,
    pub message: Option<String>,
    #[serde(skip)]
    pub agents: Vec<OctoberAgent>,
    pub agent_count: usize,
}

pub enum Command {
    Pair,
    CancelPair,
    Forget,
    Send { agent: OctoberAgent, text: String, done: Sender<Result<String, String>> },
    Focus { agent: OctoberAgent },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    client_id: String,
    credential: String,
}

fn saved_path() -> std::path::PathBuf {
    support_dir().join("october-client.json")
}

fn load_saved() -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(saved_path()).ok()?).ok()
}

fn save(s: &Saved) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::create_dir_all(support_dir());
    let path = saved_path();
    if std::fs::write(&path, serde_json::to_vec(s).unwrap_or_default()).is_ok() {
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
}

fn client_id() -> String {
    load_saved().map(|s| s.client_id).unwrap_or_else(|| {
        let mut b = [0u8; 16];
        unsafe { libc::arc4random_buf(b.as_mut_ptr().cast(), 16) };
        b[6] = (b[6] & 0x0f) | 0x40;
        b[8] = (b[8] & 0x3f) | 0x80;
        let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
        format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
    })
}

pub fn start() -> (Arc<Mutex<LinkState>>, Sender<Command>) {
    let state = Arc::new(Mutex::new(LinkState { status: "notRunning".into(), ..Default::default() }));
    let (tx, rx) = channel();
    let s = state.clone();
    std::thread::spawn(move || run(s, rx));
    (state, tx)
}

fn run(state: Arc<Mutex<LinkState>>, rx: Receiver<Command>) {
    let mut client: Option<Client> = None;
    let mut pairing: Option<(String, String, Instant)> = None; // (requestId, clientId, started)
    let mut next_poll = Instant::now();
    loop {
        // Commands first.
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Command::Pair => match client.as_ref() {
                    Some(c) => {
                        let id = client_id();
                        match c.pair_request(&id) {
                            Ok((req, code)) => {
                                pairing = Some((req, id, Instant::now()));
                                update(&state, |s| {
                                    s.status = "pairing".into();
                                    s.pairing_code = Some(code);
                                    s.message = None;
                                });
                            }
                            Err(e) => update(&state, |s| s.message = Some(format!("{e:#}"))),
                        }
                    }
                    None => update(&state, |s| s.message = Some("Open October Desktop first.".into())),
                },
                Command::CancelPair => {
                    pairing = None;
                    update(&state, |s| s.pairing_code = None);
                }
                Command::Forget => {
                    let _ = std::fs::remove_file(saved_path());
                    client = None;
                    next_poll = Instant::now();
                }
                Command::Send { agent, text, done } => {
                    let r = match client.as_mut() {
                        Some(c) => c.send(&agent, &text).map_err(|e| format!("{e:#}")),
                        None => Err("October isn't running".into()),
                    };
                    let _ = done.send(r);
                }
                Command::Focus { agent } => {
                    if let Some(c) = client.as_mut() {
                        let _ = c.focus(&agent);
                    }
                }
            }
        }

        // Pairing: poll until October answers.
        if let (Some((req, id, started)), Some(c)) = (pairing.clone(), client.as_ref()) {
            match c.pair_poll(&id, &req) {
                Ok((st, Some(credential))) if st == "approved" => {
                    save(&Saved { client_id: id, credential });
                    pairing = None;
                    client = None; // reconnect with the new credential
                    update(&state, |s| {
                        s.pairing_code = None;
                        s.message = None;
                    });
                    next_poll = Instant::now();
                }
                Ok((st, _)) if st == "denied" || st == "expired" => {
                    pairing = None;
                    update(&state, |s| {
                        s.pairing_code = None;
                        s.message = Some(if st == "denied" { "October declined the connection.".into() } else { "The request expired. Try again.".into() });
                    });
                }
                Err(e) if started.elapsed() > Duration::from_secs(150) => {
                    pairing = None;
                    update(&state, |s| {
                        s.pairing_code = None;
                        s.message = Some(format!("{e:#}"));
                    });
                }
                _ => {}
            }
        }

        if Instant::now() >= next_poll {
            next_poll = Instant::now() + Duration::from_secs(3);
            refresh(&state, &mut client, pairing.is_some());
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

fn refresh(state: &Arc<Mutex<LinkState>>, client: &mut Option<Client>, pairing: bool) {
    let Some(run) = october_core::discover() else {
        *client = None;
        let status = if october_core::installed() { "notRunning" } else { "notInstalled" };
        update(state, |s| {
            s.status = status.into();
            s.agents.clear();
            s.agent_count = 0;
            s.core_version = None;
        });
        return;
    };
    // (Re)connect when core restarted or we have no client.
    if client.as_ref().is_none_or(|c| c.run.instance_id != run.instance_id) {
        let saved = load_saved().map(|s| (s.client_id, s.credential));
        let mut c = Client::new(run.clone(), saved.clone());
        let hs = c.handshake();
        let hs = match hs {
            Err(e) if saved.is_some() && format!("{e:#}").contains("revoked") => {
                // October revoked Lantern: fall back to read-only.
                let _ = std::fs::remove_file(saved_path());
                c = Client::new(run.clone(), None);
                update(state, |s| s.message = Some("October disconnected Lantern. Connect again to reply through October.".into()));
                c.handshake()
            }
            other => other,
        };
        if let Err(e) = hs {
            *client = None;
            update(state, |s| {
                s.status = "error".into();
                s.message = Some(format!("{e:#}"));
            });
            return;
        }
        *client = Some(c);
    }
    let c = client.as_mut().unwrap();
    match c.agents() {
        Ok(agents) => {
            let paired = c.paired();
            let version = c.run.core_version.clone();
            update(state, |s| {
                s.status = if pairing { "pairing".into() } else if paired { "connected".into() } else { "readOnly".into() };
                s.paired = paired;
                s.core_version = Some(version);
                s.agent_count = agents.len();
                s.agents = agents;
            });
        }
        Err(e) => {
            let msg = format!("{e:#}");
            if msg.contains("revoked") {
                *client = None;
            }
            update(state, |s| s.message = Some(msg));
        }
    }
}

fn update(state: &Arc<Mutex<LinkState>>, f: impl FnOnce(&mut LinkState)) {
    if let Ok(mut s) = state.lock() {
        f(&mut s);
    }
}
