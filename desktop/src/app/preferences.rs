//! Settings' effects (Swift `SettingsView` + `Preferences`): each change is saved and takes effect
//! right away, the way the Mac's does.
use super::Lantern;
use crate::model::{HookState, UpdateInfo};
use crate::os::Event;
use crate::ui::SettingChange;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const UPDATE_EVERY: Duration = Duration::from_secs(24 * 3600);

#[derive(Default)]
pub(super) struct Upkeep {
    /// When updates were last checked (automatic checks run daily).
    checked: Option<Instant>,
    /// Open the toolkit list once the engine has (re)built it.
    open_toolkit: bool,
    /// The newest published release, once checked.
    release: Option<crate::updates::Release>,
    installing: bool,
    /// The running check was the daily one: a newer release installs on its own.
    automatic: bool,
    /// Installed by the daily check: Lantern restarts into it once you're not using it.
    pub(super) installed: Option<std::path::PathBuf>,
}

impl Lantern {
    pub(super) fn change_setting(&mut self, change: SettingChange) {
        let prefs = &mut self.model.prefs;
        match change {
            SettingChange::OpenAtLogin(on) => {
                if let Err(error) = crate::settings::autostart(on) {
                    return self.show(format!("Couldn't change Open at login: {error:#}"));
                }
                prefs.open_at_login = on;
            }
            SettingChange::Edge(edge) => prefs.edge = edge,
            // Only the offered shortcuts (Swift's presets); each change rebinds both.
            SettingChange::MessageHotkey(keys) if crate::model::MESSAGE_HOTKEYS.contains(&keys.as_str()) => {
                prefs.message_hotkey = keys;
                crate::os::shortcuts(self.tx.clone(), prefs.message_hotkey.clone(), prefs.point_ask_hotkey.clone());
            }
            SettingChange::PointAskHotkey(keys) if crate::model::POINT_ASK_HOTKEYS.contains(&keys.as_str()) => {
                prefs.point_ask_hotkey = keys;
                crate::os::shortcuts(self.tx.clone(), prefs.message_hotkey.clone(), prefs.point_ask_hotkey.clone());
            }
            SettingChange::MessageHotkey(_) | SettingChange::PointAskHotkey(_) => return,
            SettingChange::AutomaticUpdates(on) => prefs.automatic_updates = on,
            SettingChange::ScreenshotNewSessions(on) => prefs.screenshot_new_sessions = on,
            SettingChange::BusForSessions(on) => prefs.bus_for_sessions = on,
            SettingChange::ShowMe(on) => {
                prefs.show_me = on;
                if !on {
                    self.stop_show();
                }
            }
            SettingChange::ComputerUse(on) => {
                prefs.computer_use = on;
                self.request(serde_json::json!({"type": "computer.prepare", "enabled": on}));
            }
            SettingChange::AskFirst(on) => prefs.ask_first = on,
            SettingChange::ToolkitInSessions(on) => prefs.toolkit_in_sessions = on,
            SettingChange::NotificationsEnabled(on) => {
                prefs.notifications = on;
                self.engine.notification_preferences(on, prefs.notify_finished);
            }
            SettingChange::NotifyOnFinish(on) => {
                prefs.notify_finished = on;
                self.engine.notification_preferences(prefs.notifications, on);
            }
            SettingChange::AgentMuted { kind, muted } => {
                prefs.muted.retain(|k| k != kind.as_str());
                if muted {
                    prefs.muted.push(kind.0);
                }
                self.model.inbox = self.turns.inbox(&self.model.agents, &self.model.prefs, &self.model.carded());
            }
        }
        self.save_prefs();
        self.publish_tray_menu();
    }

    /// Claude Code and Codex hooks: on, off, or installed by an older Lantern.
    pub(super) fn refresh_hooks(&self) {
        crate::os::task(self.tx.clone(), || crate::os::hooks("status"), Event::Hooks);
    }

    /// Whether the desktop lets notifications through (the Mac's authorization status).
    pub(super) fn refresh_notifications(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::NotificationsAllowed(crate::os::notifications_allowed()));
        });
    }

    pub(super) fn set_hooks(&mut self, enabled: bool) {
        let action = if enabled { "install" } else { "uninstall" };
        self.model.system.hooks_error = None;
        self.model.system.hooks_busy = true;
        crate::os::task(
            self.tx.clone(),
            move || {
                crate::os::hooks(action)?;
                crate::os::hooks("status")
            },
            Event::Hooks,
        );
    }

    pub(super) fn hooks_result(&mut self, result: Result<String, String>) {
        self.model.system.hooks_busy = false;
        match result.map(|status| hook_states(&status)) {
            Ok(states) => {
                self.model.system.hooks = states;
                self.model.system.hooks_error = None;
            }
            Err(error) => self.model.system.hooks_error = Some(error),
        }
    }

    /// Check for Updates: finds a newer release; once one is known, choosing it again installs it
    /// and restarts (the Mac's "Install Update" step). Already installed by the daily check, it
    /// restarts into it.
    pub(super) fn check_updates(&mut self) {
        if let Some(path) = self.upkeep.installed.take() {
            return self.restart_into(&path);
        }
        if let Some(release) = self.upkeep.release.clone().filter(crate::updates::Release::is_newer) {
            return self.install_update(release, false);
        }
        self.upkeep.automatic = false;
        self.start_update_check();
    }

    fn start_update_check(&mut self) {
        self.upkeep.checked = Some(Instant::now());
        self.model.system.checking_updates = true;
        crate::os::task(self.tx.clone(), crate::updates::check, Event::Update);
    }

    pub(super) fn update_result(&mut self, result: Result<crate::updates::Release, String>) {
        self.model.system.checking_updates = false;
        match result {
            Ok(release) => {
                let newer = release.is_newer();
                self.model.system.update = Some(UpdateInfo { newer, version: release.version.to_string() });
                self.upkeep.release = Some(release.clone());
                if newer && std::mem::take(&mut self.upkeep.automatic) {
                    // The daily check installs it quietly; Lantern restarts into it when idle.
                    self.install_update(release, true);
                } else if newer {
                    self.show(format!("Lantern {} is available. Choose Check for Updates again to install it.", release.version));
                }
                self.publish_tray_menu();
            }
            Err(error) => {
                self.upkeep.installing = false;
                self.show(error);
            }
        }
    }

    /// Downloads and checks the release and installs it beside this one (the Start menu shortcut
    /// and Open at login move to it). Then restarts into it, right away when you asked for it, or
    /// once you're not using Lantern when the daily check found it. Agents running in Lantern
    /// keep running: their sessions outlive the app.
    fn install_update(&mut self, release: crate::updates::Release, quietly: bool) {
        if std::mem::replace(&mut self.upkeep.installing, true) {
            return;
        }
        if !quietly {
            self.show(format!("Installing Lantern {}…", release.version));
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let installed = crate::updates::install(release).map_err(|error| format!("Update failed: {error:#}"));
            let _ = tx.send(Event::UpdateInstalled { result: installed, quietly });
        });
    }

    pub(super) fn update_installed(&mut self, result: Result<std::path::PathBuf, String>, quietly: bool) {
        self.upkeep.installing = false;
        match result {
            Ok(path) if quietly => self.upkeep.installed = Some(path),
            Ok(path) => self.restart_into(&path),
            Err(error) => self.show(error),
        }
    }

    fn restart_into(&mut self, path: &std::path::Path) {
        match std::process::Command::new(path)
            .env_remove("LANTERN_ENGINE")
            .args(["--restart-after", &std::process::id().to_string()])
            .spawn()
        {
            Ok(_) => self.windows.quit(),
            Err(error) => self.show(format!("Update failed: couldn't start the new version: {error}")),
        }
    }

    /// Nothing open, nothing being said or selected: a restart now goes unnoticed.
    fn idle(&self) -> bool {
        self.model.panel.is_none()
            && !self.model.pill_expanded
            && !self.model.dictating
            && self.model.point_ask.phase == crate::model::PointAskPhase::Closed
            && self.model.dictation_offer.is_none()
    }

    /// Daily, when automatic updates are on (the first check soon after start).
    pub(super) fn upkeep_tick(&mut self) {
        let due = self.upkeep.checked.is_none_or(|at| at.elapsed() >= UPDATE_EVERY);
        if due
            && self.model.prefs.automatic_updates
            && !self.model.system.checking_updates
            && self.started.elapsed() > Duration::from_secs(30)
            && self.upkeep.installed.is_none()
        {
            self.upkeep.automatic = true;
            self.start_update_check();
        }
        if self.idle()
            && let Some(path) = self.upkeep.installed.take()
        {
            self.restart_into(&path);
        }
    }

    pub(super) fn refresh_toolkit(&mut self, open: bool) {
        self.upkeep.open_toolkit |= open;
        self.request(json!({"type": "toolkit.refresh"}));
    }

    /// The engine rebuilt the toolkit list.
    pub(super) fn toolkit_result(&mut self, event: &Value) {
        match (event["ok"] == true, event["path"].as_str()) {
            (true, Some(path)) if std::mem::take(&mut self.upkeep.open_toolkit) => {
                if let Err(error) = crate::os::open(path) {
                    self.show(format!("Couldn't open the toolkit list: {error:#}"));
                }
            }
            (true, _) => self.show("Toolkit list refreshed"),
            (false, _) => {
                self.upkeep.open_toolkit = false;
                self.show(event["message"].as_str().unwrap_or("Couldn't refresh the toolkit list").to_string());
            }
        }
    }

    /// The system's notification settings for Lantern.
    pub(super) fn open_notification_settings(&mut self) {
        let opened = if cfg!(windows) {
            crate::os::open("ms-settings:notifications").is_ok()
        } else {
            crate::os::command("gnome-control-center").arg("notifications").spawn().is_ok()
        };
        if !opened {
            self.show("Open your desktop's notification settings to allow Lantern's notifications.");
        }
    }
}

/// `lantern-engine hooks status` → per agent state.
pub fn hook_states(status: &str) -> std::collections::HashMap<String, HookState> {
    let value: Value = serde_json::from_str(status).unwrap_or_default();
    let state = |on: bool, outdated: bool| {
        if outdated {
            HookState::NeedsUpdate
        } else if on {
            HookState::On
        } else {
            HookState::Off
        }
    };
    [
        ("claude".to_string(), state(value["claude"] == true, value["claudeOutdated"] == true)),
        ("codex".to_string(), state(value["codex"] == true, value["codexOutdated"] == true)),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Agent, AgentKind, AgentState};

    #[test]
    fn hook_status_maps_to_on_off_and_needs_update() {
        let states = hook_states(r#"{"claude":true,"claudeOutdated":true,"codex":false}"#);
        assert_eq!(states["claude"], HookState::NeedsUpdate);
        assert_eq!(states["codex"], HookState::Off);
        assert_eq!(hook_states(r#"{"claude":true,"codex":true}"#)["claude"], HookState::On);
    }

    #[test]
    fn muting_an_agent_takes_it_out_of_waiting_at_once() {
        let (engine, _) = crate::sidecar::test_sender();
        let mut lantern = Lantern::new(engine, None);
        lantern.model.agents =
            vec![Agent { id: "a".into(), kind: AgentKind::new("codex"), state: AgentState::Waiting, ..Default::default() }];
        lantern.model.inbox = lantern.turns.inbox(&lantern.model.agents, &lantern.model.prefs, &lantern.model.carded());
        assert_eq!(lantern.model.badge(), 1);
        lantern.change_setting(SettingChange::AgentMuted { kind: AgentKind::new("codex"), muted: true });
        assert_eq!(lantern.model.badge(), 0);
        lantern.change_setting(SettingChange::AgentMuted { kind: AgentKind::new("codex"), muted: false });
        assert_eq!(lantern.model.badge(), 1);
        assert!(lantern.model.prefs.muted.is_empty());
    }
}
