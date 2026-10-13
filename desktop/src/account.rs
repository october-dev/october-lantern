//! October credentials stay in memory or the Linux Secret Service, never in settings or logs.
use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::{Value, json};
use sha2::Digest;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

const AUTH: &str = "https://latwxiqjgvluiddckvmj.supabase.co/auth/v1";
const KEY: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImxhdHd4aXFqZ3ZsdWlkZGNrdm1qIiwicm9sZSI6ImFub24iLCJpYXQiOjE3NTAzMDExMzEsImV4cCI6MjA2NTg3NzEzMX0.Aor5lE6ZSvv83Or_CxQNUdRzRUqit3fODkNSJpcDJ7E";
#[derive(Clone)]
pub struct Session {
    pub access: String,
    pub refresh: String,
    pub email: String,
    pub expires: Instant,
    pub plan: Value,
}

/// Authentication and refresh continue even when the compositor suspends UI frames.
pub struct Flow {
    pub generation: u64,
    pub current: Arc<AtomicU64>,
    pub engine: crate::sidecar::EngineSender,
    pub vault: std::sync::mpsc::Sender<VaultCommand>,
    pub remember: bool,
    pub events: std::sync::mpsc::Sender<crate::os::Event>,
}
impl Flow {
    fn active(&self) -> bool {
        self.current.load(Ordering::SeqCst) == self.generation
    }
    fn publish(&self, session: &Session) -> bool {
        let result = self.engine.credentials(self.generation, &self.current, Some((&session.access, session.expires)), || {
            if self.remember {
                let _ = self.vault.send(VaultCommand::Store(session.refresh.clone()));
            } else {
                let _ = self.vault.send(VaultCommand::Clear);
            }
        });
        if !self.active() {
            return false;
        }
        if let Err(error) = result {
            let _ = self.events.send(crate::os::Event::Status(format!("Phone connection: {error:#}")));
        }
        let _ = self.events.send(crate::os::Event::Account { generation: self.generation, result: Ok(session.clone()) });
        true
    }
    fn wait_until(&self, deadline: Instant) -> bool {
        while self.active() && Instant::now() < deadline {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()).min(Duration::from_secs(1)));
        }
        self.active()
    }
    pub fn start(self, work: impl FnOnce() -> Result<Session> + Send + 'static) {
        std::thread::spawn(move || self.run(work, refresh));
    }
    fn run(self, work: impl FnOnce() -> Result<Session>, mut renew: impl FnMut(String) -> Result<Session>) {
        let mut session = match work() {
            Ok(session) => session,
            Err(error) => {
                let _ = self.engine.credentials(self.generation, &self.current, None, || {});
                if self.active() {
                    let _ = self.events.send(crate::os::Event::Account { generation: self.generation, result: Err(format!("{error:#}")) });
                }
                return;
            }
        };
        if !self.publish(&session) {
            return;
        }
        loop {
            let now = Instant::now();
            let remaining = session.expires.saturating_duration_since(now);
            let deadline = now + remaining.saturating_sub(Duration::from_secs(120).min(remaining / 2));
            if !self.wait_until(deadline) {
                return;
            }
            match renew(session.refresh.clone()) {
                Ok(next) => {
                    if !self.publish(&next) {
                        return;
                    }
                    session = next;
                }
                Err(error) if Instant::now() >= session.expires => {
                    let _ = self.engine.credentials(self.generation, &self.current, None, || {});
                    if self.active() {
                        let _ = self.events.send(crate::os::Event::Account {
                            generation: self.generation,
                            result: Err(format!("Session expired; sign in again: {error:#}")),
                        });
                    }
                    return;
                }
                Err(_) => {
                    if !self.wait_until((Instant::now() + Duration::from_secs(15)).min(session.expires)) {
                        return;
                    }
                }
            }
        }
    }
}
pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(90))).build().into()
}
fn exchange(grant: &str, body: Value) -> Result<Session> {
    let agent = agent();
    let mut response = agent
        .post(&format!("{AUTH}/token?grant_type={grant}"))
        .header("apikey", KEY)
        .send_json(body)
        .context("October sign-in failed; check your credentials and connection")?;
    let value: Value = response.body_mut().with_config().limit(1024 * 1024).read_json()?;
    let access = value["access_token"].as_str().context("missing access token")?.to_owned();
    let refresh = value["refresh_token"].as_str().context("missing refresh token")?.to_owned();
    let email = value["user"]["email"].as_str().unwrap_or("October account").to_owned();
    let seconds = value["expires_in"].as_u64().unwrap_or(3600).clamp(60, 86400);
    let expires = Instant::now() + Duration::from_secs(seconds);
    let plan = agent
        .get("https://www.october.dev/api/plan")
        .header("Authorization", &format!("Bearer {access}"))
        .call()
        .ok()
        .and_then(|mut response| response.body_mut().with_config().limit(1024 * 1024).read_json::<Value>().ok())
        .unwrap_or(Value::Null);
    Ok(Session { access, refresh, email, expires, plan })
}
pub fn password(email: String, password: String) -> Result<Session> {
    exchange("password", json!({"email":email, "password":password}))
}
pub fn refresh(token: String) -> Result<Session> {
    exchange("refresh_token", json!({"refresh_token": token}))
}
pub fn oauth(provider: &str, generation: u64, current: Arc<AtomicU64>) -> Result<Session> {
    let listener = TcpListener::bind("127.0.0.1:54547").context("Sign-in callback port 54547 is already in use")?;
    listener.set_nonblocking(true)?;
    let verifier = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
    let mut url = url::Url::parse(&format!("{AUTH}/authorize"))?;
    url.query_pairs_mut()
        .append_pair("provider", provider)
        .append_pair("redirect_to", "http://localhost:54547/callback")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "s256");
    crate::os::open(url.as_str())?;
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline && current.load(Ordering::SeqCst) == generation {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                let mut bytes = [0u8; 8192];
                let count = stream.read(&mut bytes)?;
                let request = std::str::from_utf8(&bytes[..count])?;
                let target = request.lines().next().and_then(|line| line.strip_prefix("GET ")).and_then(|line| line.split(' ').next());
                let code = target.and_then(|target| callback_code(target).ok());
                let refused = if code.is_some() { None } else { target.and_then(callback_error) };
                let (status, page) = match (&code, &refused) {
                    (Some(_), _) => ("200 OK", callback_page(None)),
                    (None, Some(message)) => ("400 Bad Request", callback_page(Some(message))),
                    (None, None) => ("400 Bad Request", callback_page(Some("That wasn't Lantern's sign-in link."))),
                };
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                    page.len()
                )?;
                if let Some(code) = code {
                    anyhow::ensure!(current.load(Ordering::SeqCst) == generation, "Sign-in cancelled");
                    return exchange("pkce", json!({"auth_code":code,"code_verifier":verifier}));
                }
                if let Some(message) = refused {
                    bail!("{message}");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Sign-in cancelled or timed out")
}
fn callback_code(target: &str) -> Result<String> {
    anyhow::ensure!(target.starts_with('/'), "invalid callback target");
    let url = url::Url::parse(&format!("http://localhost:54547{target}"))?;
    anyhow::ensure!(url.path() == "/callback", "invalid callback path");
    let codes: Vec<_> = url.query_pairs().filter(|(key, _)| key == "code").collect();
    anyhow::ensure!(codes.len() == 1 && !codes[0].1.is_empty() && codes[0].1.len() <= 4096, "invalid callback code");
    Ok(codes[0].1.to_string())
}
/// Why the provider sent the browser back without a code (`error_description`, else `error`), for
/// a callback to `/callback`; `None` for anything else (a stray request such as a favicon).
fn callback_error(target: &str) -> Option<String> {
    let url = url::Url::parse(&format!("http://localhost:54547{target}")).ok()?;
    if url.path() != "/callback" {
        return None;
    }
    let value =
        |name: &str| url.query_pairs().find(|(key, value)| key == name && !value.trim().is_empty()).map(|(_, value)| value.into_owned());
    value("error_description").or_else(|| value("error"))
}

/// The page the browser shows after the sign-in step (Swift `CallbackListener.page`): the lantern,
/// what happened, and what to do now. `refused`: why it didn't finish.
pub(crate) fn callback_page(refused: Option<&str>) -> String {
    let (title, text) = match refused {
        None => ("You're signed in to October Lantern".to_string(), "Lantern is ready. You can close this tab.".to_string()),
        Some(message) => ("Sign-in didn't finish".to_string(), format!("{} Go back to Lantern and try again.", message.trim())),
    };
    let logo = logo_data_url()
        .map(|url| {
            format!("<img src=\"{url}\" width=\"88\" height=\"88\" alt=\"\" style=\"filter:drop-shadow(0 0 24px rgba(240,180,80,.45))\">")
        })
        .unwrap_or_default();
    format!(
        "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <title>October Lantern</title>\n\
         <body style=\"font:16px -apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,Ubuntu,Cantarell,sans-serif;background:#141414;color:#f3efe6;display:grid;place-items:center;min-height:100vh;margin:0\">\n\
         <main style=\"text-align:center;max-width:420px;padding:24px\">{logo}\n\
         <h1 style=\"font-size:22px;margin:18px 0 8px\">{}</h1>\n\
         <p style=\"color:#a8a29a;line-height:1.5;margin:0\">{}</p></main></body>\n",
        escape(&title),
        escape(&text)
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// The app's logo, 176 px (88 pt on a 2x screen), as a PNG data URL.
fn logo_data_url() -> Option<&'static str> {
    static URL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    URL.get_or_init(|| {
        let logo = image::load_from_memory(include_bytes!("../../logo.png")).ok()?;
        let small = logo.resize_exact(176, 176, image::imageops::FilterType::Lanczos3);
        let mut png = std::io::Cursor::new(Vec::new());
        small.write_to(&mut png, image::ImageFormat::Png).ok()?;
        Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png.into_inner())))
    })
    .as_deref()
}

pub enum VaultCommand {
    Store(String),
    Clear,
}
pub fn vault(events: std::sync::mpsc::Sender<crate::os::Event>) -> std::sync::mpsc::Sender<VaultCommand> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for command in rx {
            match command {
                VaultCommand::Store(token) => {
                    if let Err(error) = crate::os::secret_store(&token) {
                        let _ = events.send(crate::os::Event::Status(format!(
                            "Sign-in is active for this session; keyring could not save it: {error:#}"
                        )));
                    }
                }
                VaultCommand::Clear => crate::os::secret_clear(),
            }
        }
    });
    tx
}
pub fn restore() -> Result<Session> {
    refresh(crate::os::secret_lookup()?)
}
pub fn logout(access: String) {
    let _ = agent().post(&format!("{AUTH}/logout")).header("apikey", KEY).header("Authorization", &format!("Bearer {access}")).send_empty();
}
#[cfg(test)]
mod tests {
    use super::*;
    fn session(access: &str) -> Session {
        Session {
            access: access.into(),
            refresh: format!("refresh-{access}"),
            email: "test@example.test".into(),
            expires: Instant::now() + Duration::from_millis(20),
            plan: Value::Null,
        }
    }
    #[test]
    fn refresh_updates_engine_and_keyring_without_ui_frames() {
        let (engine, sender) = crate::sidecar::test_sender();
        let current = Arc::new(AtomicU64::new(1));
        let (vault, vault_rx) = std::sync::mpsc::channel();
        let (events, event_rx) = std::sync::mpsc::channel();
        let flow = Flow { generation: 1, current: current.clone(), engine: sender, vault, remember: true, events };
        let mut refreshed = false;
        flow.run(
            || Ok(session("initial")),
            |token| {
                if !refreshed {
                    assert_eq!(token, "refresh-initial");
                    refreshed = true;
                    Ok(session("renewed"))
                } else {
                    current.store(2, Ordering::SeqCst);
                    bail!("test cancellation")
                }
            },
        );
        // Only consume UI events after the complete background lifecycle has run.
        for access in ["initial", "renewed"] {
            let token = engine.events.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
            assert_eq!(token["type"], "phone.token");
            assert_eq!(token["accessToken"], access);
            // October Harness sessions get the same (refreshed) token.
            let october = engine.events.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
            assert_eq!(october["type"], "october.session");
            assert_eq!(october["accessToken"], access);
            assert!(
                matches!(vault_rx.recv_timeout(Duration::from_secs(3)).unwrap(),VaultCommand::Store(token) if token == format!("refresh-{access}"))
            );
            assert!(
                matches!(event_rx.recv_timeout(Duration::from_secs(3)).unwrap(),crate::os::Event::Account {result:Ok(session),..} if session.access == access)
            );
        }
        assert!(refreshed);
    }
    #[test]
    fn cancelled_sign_in_does_not_store_or_publish_credentials() {
        let (mut engine, sender) = crate::sidecar::test_sender();
        let current = Arc::new(AtomicU64::new(1));
        let (vault, vault_rx) = std::sync::mpsc::channel();
        let (events, event_rx) = std::sync::mpsc::channel();
        let flow = Flow { generation: 1, current: current.clone(), engine: sender, vault, remember: true, events };
        flow.run(
            || {
                current.store(2, Ordering::SeqCst);
                Ok(session("late"))
            },
            |_| panic!("cancelled refresh"),
        );
        assert!(vault_rx.try_recv().is_err());
        assert!(event_rx.try_recv().is_err());
        engine.send(&json!({"type":"marker"})).unwrap();
        assert_eq!(engine.events.recv_timeout(Duration::from_secs(3)).unwrap().unwrap()["type"], "marker");
    }
    #[test]
    fn session_only_sign_in_clears_previously_saved_credentials() {
        let (_engine, sender) = crate::sidecar::test_sender();
        let (vault, vault_rx) = std::sync::mpsc::channel();
        let (events, _event_rx) = std::sync::mpsc::channel();
        let flow = Flow { generation: 1, current: Arc::new(AtomicU64::new(1)), engine: sender, vault, remember: false, events };
        assert!(flow.publish(&session("temporary")));
        assert!(matches!(vault_rx.recv_timeout(Duration::from_secs(3)).unwrap(), VaultCommand::Clear));
    }
    #[test]
    fn callback_page_says_where_you_are_and_escapes_the_message() {
        let done = callback_page(None);
        assert!(done.contains("<h1 style=\"font-size:22px;margin:18px 0 8px\">You&#39;re signed in to October Lantern</h1>"));
        assert!(done.contains("Lantern is ready. You can close this tab."));
        assert!(done.contains("background:#141414;color:#f3efe6") && done.contains("color:#a8a29a") && done.contains("'Segoe UI'"));
        assert!(done.contains("<img src=\"data:image/png;base64,") && done.contains("width=\"88\""), "the lantern");
        assert!(done.contains("drop-shadow(0 0 24px rgba(240,180,80,.45))"));
        let failed = callback_page(Some("Access <b>denied</b> & \"gone\""));
        assert!(failed.contains("Sign-in didn&#39;t finish"));
        assert!(failed.contains("Access &lt;b&gt;denied&lt;/b&gt; &amp; &quot;gone&quot; Go back to Lantern and try again."));
        assert!(!failed.contains("<b>"));
        assert_eq!(
            callback_error("/callback?error=access_denied&error_description=The+user+denied+access").as_deref(),
            Some("The user denied access")
        );
        assert_eq!(callback_error("/callback?error=access_denied").as_deref(), Some("access_denied"));
        assert_eq!(callback_error("/callback?code=x"), None);
        assert_eq!(callback_error("/favicon.ico?error=x"), None);
    }

    #[test]
    fn callback_rejects_wrong_paths_and_duplicate_codes() {
        assert!(callback_code("/elsewhere?code=x").is_err());
        assert!(callback_code("/callback?code=a&code=b").is_err());
        assert!(callback_code("/callback?code=").is_err());
        assert_eq!(callback_code("/callback?code=a%2Bb").unwrap(), "a+b");
    }
}
