//! Linux desktop integration. Portals retain the user's choice of screen and shortcuts on
//! Wayland; Lantern never bypasses a denied portal request.
use super::Event;
use crate::settings::{Settings, data_dir, private_write};
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub const BUS_NAME: &str = "dev.october.Lantern";
struct Bridge {
    snapshot: Arc<RwLock<String>>,
    tx: Sender<Event>,
}
#[zbus::interface(name = "dev.october.Lantern")]
impl Bridge {
    fn get_snapshot(&self) -> String {
        self.snapshot.read().map(|value| value.clone()).unwrap_or_else(|_| "[]".into())
    }
    fn show(&self, agent: &str) {
        let _ = self.tx.send(Event::Activate(if agent.is_empty() { "show".into() } else { agent.into() }));
    }
    fn point_ask(&self) {
        let _ = self.tx.send(Event::Activate("capture".into()));
    }
}
pub fn bridge(tx: Sender<Event>, snapshot: Arc<RwLock<String>>) {
    std::thread::spawn(move || {
        let result = (|| -> Result<Connection> {
            Ok(zbus::blocking::connection::Builder::session()?
                .name(BUS_NAME)?
                .serve_at("/dev/october/Lantern", Bridge { tx: tx.clone(), snapshot })?
                .build()?)
        })();
        match result {
            Ok(_connection) => loop {
                std::thread::park();
            },
            Err(error) => {
                let _ = tx.send(Event::Status(format!("Desktop integration: {error:#}")));
            }
        }
    });
}
pub fn show_existing(action: &str) -> bool {
    (|| -> Result<()> {
        let connection = Connection::session()?;
        let proxy = Proxy::new(&connection, BUS_NAME, "/dev/october/Lantern", BUS_NAME)?;
        Ok(proxy.call("Show", &(action,))?)
    })()
    .is_ok()
}
/// Window focus isn't passed between apps on Linux; nothing to allow.
pub fn allow_focus(_pid: u32) {}
pub fn open(path: impl AsRef<std::ffi::OsStr>) -> Result<()> {
    let mut child = Command::new("xdg-open").arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
type Options = HashMap<String, OwnedValue>;
fn text(value: &str) -> OwnedValue {
    Value::from(value).try_to_owned().expect("string value")
}
fn token() -> String {
    format!("lantern{}", uuid::Uuid::new_v4().simple())
}
fn portal<'a>(connection: &'a Connection, interface: &str) -> Result<Proxy<'a>> {
    Ok(Proxy::new(connection, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", interface.to_owned())?)
}
fn connection() -> Result<Connection> {
    let connection = Connection::session()?;
    // Host applications must register before portal calls so the compositor knows their app ID.
    let registry = portal(&connection, "org.freedesktop.host.portal.Registry")?;
    let _: Result<(), _> = registry.call("Register", &("dev.october.lantern", Options::new()));
    drop(registry);
    Ok(connection)
}
fn request(connection: &Connection, token: &str, call: impl FnOnce() -> Result<OwnedObjectPath>) -> Result<Options> {
    let sender = connection.unique_name().context("no session bus name")?.as_str().trim_start_matches(':').replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let proxy = Proxy::new(connection, "org.freedesktop.portal.Desktop", path.clone(), "org.freedesktop.portal.Request")?;
    // Subscribe before making the call: a fast portal can finish before the method returns.
    let mut signals = proxy.receive_signal("Response")?;
    let returned = call()?;
    anyhow::ensure!(returned.as_str() == path, "portal returned an unexpected request handle");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = signals.next().context("portal disconnected").and_then(|message| {
            let (code, options): (u32, Options) = message.body().deserialize()?;
            // 1: the person cancelled; 2: denied or failed (e.g. no permission for a non-interactive capture).
            anyhow::ensure!(code == 0, "Desktop request cancelled or denied (code {code})");
            Ok(options)
        });
        let _ = tx.send(result);
    });
    match rx.recv_timeout(Duration::from_secs(120)) {
        Ok(result) => result,
        Err(_) => {
            let _: Result<(), _> = proxy.call("Close", &());
            // Wake the response reader even if a broken portal never emits cancellation.
            let _ = connection.clone().close();
            bail!("Desktop request timed out");
        }
    }
}
/// The whole screen, without the desktop's chooser (the portal asks once to allow it), for New
/// Session's "Include a screenshot of my screen" and Point & Ask's frozen frame.
pub fn capture_screen() -> Result<PathBuf> {
    // GNOME 50 denies a non-interactive capture to a host app registered under an app ID
    // (response 2, no dialog), but asks the person once for an unregistered one. So captures
    // without the chooser go out unregistered; everything else registers.
    let connection = Connection::session()?;
    let proxy = portal(&connection, "org.freedesktop.portal.Screenshot")?;
    let id = token();
    let options = Options::from([("handle_token".into(), text(&id)), ("interactive".into(), false.into())]);
    let mut result = request(&connection, &id, || Ok(proxy.call("Screenshot", &("", options))?))?;
    let uri = String::try_from(result.remove("uri").context("screenshot URI missing")?)?;
    let source = url::Url::parse(&uri)?.to_file_path().map_err(|_| anyhow::anyhow!("screenshot is not a local file"))?;
    let bytes = std::fs::read(&source)?;
    anyhow::ensure!(bytes.len() <= 20 * 1024 * 1024 && bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "screenshot must be a PNG under 20 MB");
    let dest = data_dir().join("screenshots").join(format!("{}.png", uuid::Uuid::new_v4().simple()));
    private_write(&dest, &bytes)?;
    Ok(dest)
}
pub fn diagnostics() -> Result<serde_json::Value> {
    let connection = connection()?;
    let screenshot = portal(&connection, "org.freedesktop.portal.Screenshot")?;
    let shortcuts = portal(&connection, "org.freedesktop.portal.GlobalShortcuts")?;
    let chooser = portal(&connection, "org.freedesktop.portal.FileChooser")?;
    let screenshot_version: u32 = screenshot.get_property("version")?;
    let shortcuts_version: u32 = shortcuts.get_property("version")?;
    let chooser_version: u32 = chooser.get_property("version")?;
    let id = token();
    let session_id = token();
    let options = Options::from([("handle_token".into(), text(&id)), ("session_handle_token".into(), text(&session_id))]);
    let mut response = request(&connection, &id, || Ok(shortcuts.call("CreateSession", &(options,))?))?;
    let handle = String::try_from(response.remove("session_handle").context("session missing")?)?;
    let session = Proxy::new(&connection, "org.freedesktop.portal.Desktop", handle, "org.freedesktop.portal.Session")?;
    let _: () = session.call("Close", &())?;
    Ok(
        serde_json::json!({"screenshot":screenshot_version,"globalShortcuts":shortcuts_version,"fileChooser":chooser_version,"shortcutSession":"created and closed","notifications":notifications_allowed()}),
    )
}
pub fn folder() -> Result<PathBuf> {
    let connection = connection()?;
    let proxy = portal(&connection, "org.freedesktop.portal.FileChooser")?;
    let id = token();
    let options = Options::from([("handle_token".into(), text(&id)), ("directory".into(), true.into()), ("multiple".into(), false.into())]);
    let mut result = request(&connection, &id, || Ok(proxy.call("OpenFile", &("", "Choose a project folder", options))?))?;
    let uris = Vec::<String>::try_from(result.remove("uris").context("folder URI missing")?)?;
    url::Url::parse(uris.first().context("no folder selected")?)?.to_file_path().map_err(|_| anyhow::anyhow!("Choose a local folder"))
}
/// The GlobalShortcuts session that holds Lantern's shortcuts now, closed when they change.
static SHORTCUT_SESSION: std::sync::Mutex<Option<(Connection, OwnedObjectPath)>> = std::sync::Mutex::new(None);
static SHORTCUT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn close_shortcut_session() {
    if let Some((connection, session)) = SHORTCUT_SESSION.lock().unwrap_or_else(|e| e.into_inner()).take() {
        // Best effort: a session the portal already dropped has nothing left to unbind.
        let _ = Proxy::new(&connection, "org.freedesktop.portal.Desktop", session, "org.freedesktop.portal.Session")
            .and_then(|session| session.call::<_, _, ()>("Close", &()));
    }
}

/// Binds the two shortcuts from the preferences through the GlobalShortcuts portal, replacing
/// the ones bound before. The keys are part of each shortcut's id, so the desktop treats a
/// changed choice as a new shortcut and offers its trigger again; activations report the
/// plain id ("compose", "capture").
pub fn shortcuts(tx: Sender<Event>, message: String, point_ask: String) {
    use std::sync::atomic::Ordering;
    let generation = SHORTCUT_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        close_shortcut_session();
        let bindings: Vec<(String, Options)> =
            [("compose", &message, "Lantern: Message an agent"), ("capture", &point_ask, "Lantern: Point & Ask")]
                .into_iter()
                .filter_map(|(id, keys, description)| {
                    let hotkey = super::Hotkey::parse(keys)?;
                    Some((
                        format!("{id}:{keys}"),
                        Options::from([("description".into(), text(description)), ("preferred_trigger".into(), text(&hotkey.trigger()))]),
                    ))
                })
                .collect();
        if bindings.is_empty() {
            let _ = tx.send(Event::Shortcuts(false));
            return;
        }
        let result = (|| -> Result<()> {
            let connection = connection()?;
            let proxy = portal(&connection, "org.freedesktop.portal.GlobalShortcuts")?;
            let id = token();
            let session_id = token();
            let options = Options::from([("handle_token".into(), text(&id)), ("session_handle_token".into(), text(&session_id))]);
            let mut result = request(&connection, &id, || Ok(proxy.call("CreateSession", &(options,))?))?;
            let session =
                OwnedObjectPath::try_from(String::try_from(result.remove("session_handle").context("missing shortcut session")?)?)?;
            *SHORTCUT_SESSION.lock().unwrap_or_else(|e| e.into_inner()) = Some((connection.clone(), session.clone()));
            // A newer change started meanwhile: it owns the shortcuts.
            if SHORTCUT_GENERATION.load(Ordering::SeqCst) != generation {
                close_shortcut_session();
                return Ok(());
            }
            let id = token();
            let mut signals = proxy.receive_all_signals()?;
            request(&connection, &id, || {
                Ok(proxy.call("BindShortcuts", &(&session, bindings, "", Options::from([("handle_token".into(), text(&id))])))?)
            })?;
            let _ = tx.send(Event::Shortcuts(true));
            for message in &mut signals {
                if SHORTCUT_GENERATION.load(Ordering::SeqCst) != generation {
                    break;
                }
                let down = match message.header().member().map(|member| member.as_str()) {
                    Some("Activated") => true,
                    Some("Deactivated") => false,
                    _ => continue,
                };
                let (handle, shortcut, _, _): (OwnedObjectPath, String, u64, Options) = message.body().deserialize()?;
                if handle != session {
                    continue;
                }
                // Point & Ask is held to point and speak, so it reports both edges.
                let activation = match (shortcut.split(':').next().unwrap_or_default(), down) {
                    ("capture", true) => "point-press",
                    ("capture", false) => "point-release",
                    ("compose", true) => "compose",
                    _ => continue,
                };
                if tx.send(Event::Activate(activation.into())).is_err() {
                    break;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = tx.send(Event::Shortcuts(false));
            let _ = tx.send(Event::Status(format!("Shortcuts: {error:#}")));
        }
    });
}
/// Whether notifications reach the person: no notification server means nobody shows them, and
/// GNOME has a per-app switch (Settings › Notifications › October Lantern). `None` when unknown.
pub fn notifications_allowed() -> Option<bool> {
    let connection = Connection::session().ok()?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(&connection).ok()?;
    let name = zbus::names::BusName::try_from("org.freedesktop.Notifications").ok()?;
    if !dbus.name_has_owner(name).ok()? {
        return None;
    }
    let output = super::command("gsettings")
        .args([
            "get",
            "org.gnome.desktop.notifications.application:/org/gnome/desktop/notifications/application/dev-october-lantern/",
            "enable",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success());
    // Not GNOME (or no gsettings): the server is there, and nothing else can block us.
    Some(output.is_none_or(|output| String::from_utf8_lossy(&output.stdout).trim() != "false"))
}
pub fn notify(tx: Sender<Event>, agent: String, title: String, body: String) {
    std::thread::spawn(move || {
        let result = (|| -> Result<()> {
            let connection = Connection::session()?;
            let proxy = Proxy::new(
                &connection,
                "org.freedesktop.Notifications",
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
            )?;
            let mut signals = proxy.receive_all_signals()?;
            let hints = Options::from([("desktop-entry".into(), text("dev.october.lantern"))]);
            let id: u32 = proxy.call(
                "Notify",
                &("October Lantern", 0u32, "dev.october.lantern", title, body, vec!["default", "Open session"], hints, 10000i32),
            )?;
            for signal in &mut signals {
                match signal.header().member().map(|member| member.as_str()) {
                    Some("ActionInvoked") => {
                        let (notification, _): (u32, String) = signal.body().deserialize()?;
                        if notification == id {
                            let _ = tx.send(Event::Activate(agent));
                            break;
                        }
                    }
                    Some("NotificationClosed") => {
                        let (notification, _): (u32, u32) = signal.body().deserialize()?;
                        if notification == id {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = tx.send(Event::Status(format!("Notification: {error:#}")));
        }
    });
}
/// The login item, launcher, icon and GNOME extension. Each failure says what's left.
pub fn remove_integrations() -> Vec<String> {
    let mut failures = Vec::new();
    let data = crate::settings::xdg("XDG_DATA_HOME", ".local/share");
    for path in [
        crate::settings::autostart_path(),
        data.join("applications/dev.october.lantern.desktop"),
        data.join("icons/hicolor/256x256/apps/dev.october.lantern.png"),
    ] {
        if path.exists()
            && let Err(error) = std::fs::remove_file(&path)
        {
            failures.push(format!("Removing {}: {error}. Delete it by hand.", path.display()));
        }
    }
    let _ = Command::new("gnome-extensions").args(["disable", "lantern@october.dev"]).output();
    let extension = data.join("gnome-shell/extensions/lantern@october.dev");
    if extension.exists()
        && let Err(error) = std::fs::remove_dir_all(&extension)
    {
        failures.push(format!("Removing the GNOME extension: {error}. Delete {} by hand.", extension.display()));
    }
    failures
}

/// Moves Lantern's app folders to the Trash: the one running and the one it was installed in
/// (they differ after an update, which runs from Lantern's data folder and went with it).
/// Only installed bundles, never a build in the repo.
pub fn remove_app(installed: Option<&Path>) -> std::result::Result<(), String> {
    let data = data_dir();
    let running = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    let mut folders: Vec<PathBuf> =
        [running, installed.map(Path::to_path_buf)].into_iter().flatten().filter(|dir| !dir.starts_with(&data) && dir.exists()).collect();
    folders.dedup();
    let mut problems = Vec::new();
    for folder in folders {
        if !super::installed_bundle(&folder) {
            problems.push(format!("{} isn't an installed Lantern (a development build?), so it was left in place.", folder.display()));
            continue;
        }
        match Command::new("gio").arg("trash").arg(&folder).output() {
            Ok(output) if output.status.success() => {}
            Ok(output) => problems.push(format!(
                "Couldn't move {} to the Trash: {}. Delete it by hand.",
                folder.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) => problems.push(format!("Couldn't move {} to the Trash: {error}. Delete it by hand.", folder.display())),
        }
    }
    if problems.is_empty() { Ok(()) } else { Err(problems.join("\n\n")) }
}
struct LanternTray {
    tx: Sender<Event>,
}
impl ksni::Tray for LanternTray {
    fn id(&self) -> String {
        "dev.october.lantern".into()
    }
    fn title(&self) -> String {
        "October Lantern".into()
    }
    fn icon_name(&self) -> String {
        "dev.october.lantern".into()
    }
    fn activate(&mut self, _: i32, _: i32) {
        let _ = self.tx.send(Event::Activate("show".into()));
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        super::tray_menu()
            .into_iter()
            .map(|item| match item {
                None => ksni::MenuItem::Separator,
                Some((label, id)) => StandardItem {
                    label,
                    activate: Box::new(move |tray: &mut Self| {
                        let _ = tray.tx.send(super::tray_event(&id));
                    }),
                    ..Default::default()
                }
                .into(),
            })
            .collect()
    }
}
fn tray_host() -> bool {
    (|| -> Result<bool> {
        let connection = Connection::session()?;
        let proxy = Proxy::new(&connection, "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher")?;
        Ok(proxy.get_property("IsStatusNotifierHostRegistered")?)
    })()
    .unwrap_or(false)
}
pub fn tray(tx: Sender<Event>) {
    std::thread::spawn(move || {
        use ksni::blocking::TrayMethods;
        let mut handle: Option<ksni::blocking::Handle<LanternTray>> = None;
        let mut last = None;
        let mut shown = 0;
        loop {
            if handle.as_ref().is_some_and(|handle| handle.is_closed()) {
                handle = None;
            }
            let host = tray_host();
            if host && handle.is_none() {
                handle = (LanternTray { tx: tx.clone() }).spawn().ok();
            }
            let available = host && handle.is_some();
            if last != Some(available) {
                if tx.send(Event::Tray(available)).is_err() {
                    break;
                }
                last = Some(available);
            }
            // Refresh the menu when the app changes it (counts); look for a tray host every 10 s.
            for _ in 0..10 {
                if let Some(handle) = &handle
                    && shown != super::tray_menu_version()
                {
                    shown = super::tray_menu_version();
                    handle.update(|_| {});
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    });
}
pub struct Recording {
    child: Child,
    path: PathBuf,
}
impl Recording {
    pub fn start() -> Result<Self> {
        let path = data_dir().join("dictation").join(format!("{}.wav", uuid::Uuid::new_v4().simple()));
        private_write(&path, b"")?;
        let child = Command::new("pw-record")
            .args(["--rate", "16000", "--channels", "1", "--format", "s16"])
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("PipeWire recorder is unavailable")?;
        Ok(Self { child, path })
    }
    pub fn finish(mut self, settings: Settings) -> Result<String> {
        // SIGINT lets pw-record finish the WAV header; kill() would leave an invalid recording.
        let _ = Command::new("kill").args(["-INT", &self.child.id().to_string()]).status();
        self.child.wait()?;
        let output = Command::new(&settings.whisper)
            .args(["-m", &settings.whisper_model, "-f"])
            .arg(&self.path)
            .args(["-l", &settings.speech_language, "-nt", "-np"])
            .output()
            .with_context(|| format!("Couldn't run the speech runtime ({})", settings.whisper))?;
        anyhow::ensure!(output.status.success(), "Transcription failed: {}", String::from_utf8_lossy(&output.stderr));
        let text = String::from_utf8(output.stdout)?.trim().to_owned();
        anyhow::ensure!(!text.is_empty(), "No speech detected");
        Ok(text)
    }
}
impl Drop for Recording {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.path);
    }
}
/// Whether the process is still running (for waiting on the instance being replaced).
pub fn process_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}
const SECRET: [&str; 4] = ["application", "dev.october.lantern", "credential", "refresh-token"];
/// Saves the October refresh token in the Secret Service keyring.
pub fn secret_store(token: &str) -> Result<()> {
    let mut child = Command::new("secret-tool")
        .args(["store", "--label=October Lantern"])
        .args(SECRET)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child.stdin.take().context("Secret Service input unavailable")?.write_all(token.as_bytes())?;
    anyhow::ensure!(child.wait()?.success(), "Secret Service could not save sign-in");
    Ok(())
}
pub fn secret_lookup() -> Result<String> {
    let output = Command::new("secret-tool").arg("lookup").args(SECRET).output()?;
    anyhow::ensure!(output.status.success(), "No saved October sign-in");
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
pub fn secret_clear() {
    let _ = Command::new("secret-tool").arg("clear").args(SECRET).output();
}
