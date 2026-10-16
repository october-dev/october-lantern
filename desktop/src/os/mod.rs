//! Desktop integrations run outside the render loop. What every system shares lives here; the
//! tray, notifications, shortcuts, single-instance bridge, pickers and keyring are per system.
use anyhow::Result;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
pub mod shell;
pub use shell::foreground_app;
#[cfg(windows)]
mod shell_win;
#[cfg(target_os = "linux")]
mod shell_x11;

pub enum Event {
    Status(String),
    Screenshot {
        owner: String,
        generation: u64,
        result: Result<std::path::PathBuf, String>,
    },
    Folder(Result<std::path::PathBuf, String>),
    Transcript {
        owner: String,
        generation: u64,
        result: Result<String, String>,
    },
    /// Dictation's first-use setup (the speech model's download): how far it's got, then done.
    SpeechSetup(crate::speech::Setup),
    Activate(String),
    Quit,
    Tray(bool),
    Shortcuts(bool),
    Account {
        generation: u64,
        result: Result<crate::account::Session, String>,
    },
    Update(Result<crate::updates::Release, String>),
    /// A release finished installing (`quietly`: by the daily check, so Lantern restarts into it
    /// once you're not using it).
    UpdateInstalled {
        result: Result<std::path::PathBuf, String>,
        quietly: bool,
    },
    /// A piece of an Ask Lantern answer, for the question `generation` and its `turn`.
    Assistant {
        generation: u64,
        turn: u64,
        piece: crate::october_ai::Piece,
    },
    /// The answer finished; `error` is what to show instead when it failed.
    AssistantDone {
        generation: u64,
        turn: u64,
        error: Option<String>,
    },
    /// A piece of a Point & Ask answer, for the conversation `generation` and its `turn` (index).
    PointAsk {
        generation: u64,
        turn: usize,
        piece: crate::october_ai::Piece,
    },
    PointAskDone {
        generation: u64,
        turn: usize,
        error: Option<String>,
    },
    /// `lantern-engine hooks status` (JSON), after reading or changing the agent hooks.
    Hooks(Result<String, String>),
    /// Uninstalling removed everything but the app, except these (what's left, and how).
    Uninstalled(Vec<String>),
    /// Whether the desktop lets Lantern's notifications through (`None`: can't tell).
    NotificationsAllowed(Option<bool>),
    /// Windows: the display under the pointer, captured before Point & Ask's overlay shows (the
    /// overlay draws it frozen), for the selection `generation`.
    #[cfg(windows)]
    Frozen {
        generation: u64,
        result: Result<Frozen, String>,
    },
    /// Windows: the selection cropped from the frozen display, as PNG.
    #[cfg(windows)]
    PointAskShot {
        generation: u64,
        result: Result<Vec<u8>, String>,
    },
    /// Point & Ask's message for an agent is ready (its screenshot saved), or couldn't be.
    PointAskSent {
        generation: u64,
        agent_id: String,
        text: Result<String, String>,
    },
}
/// The tray's menu, as the app wants it: `(label, activation id)`, `None` for a separator. Labels
/// change with the app (counts), so trays re-read it when its version changes.
static TRAY_MENU: std::sync::RwLock<Vec<Option<(String, String)>>> = std::sync::RwLock::new(Vec::new());
static TRAY_VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn set_tray_menu(items: Vec<Option<(String, String)>>) {
    let mut menu = TRAY_MENU.write().unwrap_or_else(|e| e.into_inner());
    if *menu != items {
        *menu = items;
        TRAY_VERSION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
pub fn tray_menu() -> Vec<Option<(String, String)>> {
    TRAY_MENU.read().unwrap_or_else(|e| e.into_inner()).clone()
}
/// The Linux tray refreshes its menu when this changes (the Windows tray builds it on each open).
#[cfg(target_os = "linux")]
pub fn tray_menu_version() -> u64 {
    TRAY_VERSION.load(std::sync::atomic::Ordering::SeqCst)
}
/// What choosing a tray item means.
pub fn tray_event(id: &str) -> Event {
    match id {
        "quit" => Event::Quit,
        other => Event::Activate(other.into()),
    }
}

// Wake/map the native window before waiting for another frame. A hidden Wayland window may
// receive no frame callbacks; a repaint request alone cannot reliably activate it from the tray.
pub fn event_pump(wake: impl Fn(bool) + Send + 'static) -> (Sender<Event>, mpsc::Receiver<Event>) {
    let (tx, rx) = mpsc::channel();
    let (forward, events) = mpsc::channel();
    std::thread::spawn(move || {
        let mut tray = false;
        for event in rx {
            let show = matches!(event, Event::Activate(_)) || matches!(event, Event::Tray(false)) && tray;
            if let Event::Tray(available) = &event {
                tray = *available;
            }
            if forward.send(event).is_err() {
                break;
            }
            wake(show);
        }
    });
    (tx, events)
}
/// A global shortcut from Lantern's preferences ("ctrl+alt+space"): its modifiers and its key,
/// `' '` for Space or an uppercase letter or digit. `None` for "none" or anything unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: char,
}

impl Hotkey {
    pub fn parse(keys: &str) -> Option<Self> {
        let mut hotkey = Hotkey { ctrl: false, alt: false, shift: false, key: '\0' };
        for part in keys.split('+') {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" => hotkey.ctrl = true,
                "alt" => hotkey.alt = true,
                "shift" => hotkey.shift = true,
                "space" => hotkey.key = ' ',
                key if key.len() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric()) => {
                    hotkey.key = key.chars().next()?.to_ascii_uppercase();
                }
                _ => return None,
            }
        }
        (hotkey.key != '\0' && (hotkey.ctrl || hotkey.alt)).then_some(hotkey)
    }

    /// The XDG GlobalShortcuts trigger ("CTRL+ALT+space").
    #[cfg_attr(windows, allow(dead_code))]
    pub fn trigger(self) -> String {
        let mut parts = Vec::new();
        for (on, name) in [(self.ctrl, "CTRL"), (self.alt, "ALT"), (self.shift, "SHIFT")] {
            if on {
                parts.push(name.to_string());
            }
        }
        parts.push(if self.key == ' ' { "space".into() } else { self.key.to_ascii_lowercase().to_string() });
        parts.join("+")
    }
}

/// Why the microphone couldn't be opened (Windows), and what to say about it.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneProblem {
    /// No recording device at all.
    Missing,
    /// A device is there but Windows wouldn't open it: almost always the privacy setting that
    /// lets desktop apps use the microphone.
    Blocked,
}

impl MicrophoneProblem {
    /// After opening the microphone failed, with this many recording devices.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn after_failed_open(devices: u32) -> Self {
        if devices == 0 { Self::Missing } else { Self::Blocked }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Missing => "No microphone found. Connect one and try again.",
            Self::Blocked => {
                "Lantern can't use the microphone. In Settings › Privacy & security › Microphone, turn on \"Let desktop apps access your microphone\"."
            }
        }
    }
    /// The settings page that fixes it, opened once each time it happens.
    pub fn settings_page(self) -> Option<&'static str> {
        (self == Self::Blocked).then_some("ms-settings:privacy-microphone")
    }
}

impl std::fmt::Display for MicrophoneProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for MicrophoneProblem {}

pub fn task<T: Send + 'static>(
    tx: Sender<Event>,
    work: impl FnOnce() -> Result<T> + Send + 'static,
    event: impl FnOnce(Result<T, String>) -> Event + Send + 'static,
) {
    std::thread::spawn(move || {
        let result = work().map_err(|error| format!("{error:#}"));
        let _ = tx.send(event(result));
    });
}
pub fn prune_screenshots() {
    let cutoff = std::time::SystemTime::now() - Duration::from_secs(7 * 86400);
    for entry in std::fs::read_dir(crate::settings::data_dir().join("screenshots")).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "png")
            && entry.metadata().ok().is_some_and(|metadata| metadata.is_file() && metadata.modified().is_ok_and(|time| time < cutoff))
        {
            let _ = std::fs::remove_file(path);
        }
    }
}
/// A command for a helper program, without a console window flashing up on Windows.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(::windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    command
}
pub fn hooks(action: &'static str) -> Result<String> {
    let output = command(crate::sidecar::engine_path()?).args(["hooks", action]).output()?;
    anyhow::ensure!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
/// An installed Lantern: its app and engine side by side, and not a build inside this repo
/// (a `target` folder, or a checkout with `desktop/Cargo.toml`). Uninstalling only ever moves such
/// a folder to the Trash.
pub fn installed_bundle(dir: &Path) -> bool {
    let has = |name: &str| dir.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }).is_file();
    has("october-lantern")
        && has("lantern-engine")
        && !dir
            .ancestors()
            .any(|folder| folder.file_name().is_some_and(|name| name == "target") || folder.join("desktop/Cargo.toml").is_file())
}

/// Uninstalling, except for the app itself (Swift `Support.uninstall`): the hooks, restoring the
/// agents' own settings, and Lantern's data folder (the engine's `hooks remove-all`), the desktop
/// integrations, and the settings. Best effort; each failure says what's left and how to remove it.
pub fn uninstall_parts() -> Vec<String> {
    let mut failures = Vec::new();
    if let Err(error) = hooks("remove-all") {
        let home = crate::settings::home();
        failures.push(format!(
            "Removing the hooks: {error:#}. Remove the Lantern entries from {} (\"hooks\") and {} (\"notify\") by hand; backups ending in .lantern-backup-… are next to each file. Then delete {}.",
            home.join(".claude").join("settings.json").display(),
            home.join(".codex").join("config.toml").display(),
            crate::settings::data_dir().display()
        ));
    }
    failures.extend(remove_integrations());
    let config = crate::settings::config_dir();
    if config.exists()
        && let Err(error) = std::fs::remove_dir_all(&config)
    {
        failures.push(format!("Removing Lantern's settings: {error}. Delete {} by hand.", config.display()));
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_an_installed_bundle_counts_never_a_build_in_the_repo() {
        let root = std::env::temp_dir().join(format!("lantern-bundle-{}", uuid::Uuid::new_v4().simple()));
        let exe = |name: &str| if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
        let make = |dir: &Path, both: bool| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(exe("october-lantern")), b"").unwrap();
            if both {
                std::fs::write(dir.join(exe("lantern-engine")), b"").unwrap();
            }
        };
        let installed = root.join("Downloads").join("October-Lantern");
        make(&installed, true);
        assert!(installed_bundle(&installed));
        let half = root.join("half");
        make(&half, false);
        assert!(!installed_bundle(&half), "no engine beside it");
        let cargo = root.join("work").join("target").join("release");
        make(&cargo, true);
        assert!(!installed_bundle(&cargo), "a cargo build");
        let checkout = root.join("lantern");
        std::fs::create_dir_all(checkout.join("desktop")).unwrap();
        std::fs::write(checkout.join("desktop").join("Cargo.toml"), b"").unwrap();
        let packaged = checkout.join("build").join("linux");
        make(&packaged, true);
        assert!(!installed_bundle(&packaged), "a package built inside the repo");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_microphone_that_wont_open_is_missing_or_blocked() {
        let missing = MicrophoneProblem::after_failed_open(0);
        assert_eq!(missing.message(), "No microphone found. Connect one and try again.");
        assert_eq!(missing.settings_page(), None);
        let blocked = MicrophoneProblem::after_failed_open(2);
        assert!(blocked.message().contains("Let desktop apps access your microphone"));
        assert_eq!(blocked.settings_page(), Some("ms-settings:privacy-microphone"));
        // It travels as an error and is recognised as one.
        let error: anyhow::Error = blocked.into();
        assert_eq!(error.downcast_ref::<MicrophoneProblem>(), Some(&MicrophoneProblem::Blocked));
    }

    #[test]
    fn every_offered_shortcut_parses_and_none_unbinds() {
        for keys in crate::model::MESSAGE_HOTKEYS.iter().chain(&crate::model::POINT_ASK_HOTKEYS) {
            assert_eq!(super::Hotkey::parse(keys).is_none(), *keys == "none", "{keys}");
        }
        let hotkey = super::Hotkey::parse("ctrl+shift+space").unwrap();
        assert_eq!((hotkey.ctrl, hotkey.alt, hotkey.shift, hotkey.key), (true, false, true, ' '));
        assert_eq!(hotkey.trigger(), "CTRL+SHIFT+space");
        assert_eq!(super::Hotkey::parse("ctrl+alt+k").unwrap().trigger(), "CTRL+ALT+k");
        // A bare key or an unknown name never becomes a global shortcut.
        assert_eq!(super::Hotkey::parse("space"), None);
        assert_eq!(super::Hotkey::parse("ctrl+alt+f13"), None);
    }

    #[test]
    fn tray_activation_wakes_the_window_without_ui_polling() {
        let (wake_tx, wake_rx) = mpsc::channel();
        let (tx, _events) = event_pump(move |show| {
            wake_tx.send(show).unwrap();
        });
        tx.send(Event::Status("background result".into())).unwrap();
        assert!(!wake_rx.recv_timeout(Duration::from_secs(3)).unwrap());
        tx.send(Event::Activate("agent-a".into())).unwrap();
        assert!(wake_rx.recv_timeout(Duration::from_secs(3)).unwrap());
        tx.send(Event::Tray(true)).unwrap();
        assert!(!wake_rx.recv_timeout(Duration::from_secs(3)).unwrap());
        tx.send(Event::Tray(false)).unwrap();
        assert!(wake_rx.recv_timeout(Duration::from_secs(3)).unwrap());
    }
}
