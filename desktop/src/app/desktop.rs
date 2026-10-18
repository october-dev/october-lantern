//! The desktop around the pill (Swift `AppDelegate` menu, hotkeys and notifications): the tray with
//! the Mac's status menu, global shortcuts, notifications for turns that wait on you, and a second
//! launch handing its request to the running Lantern.
use super::Lantern;
use crate::model::{PanelMode, hotkey_label};
use crate::ui::UiAction;
use std::sync::{Arc, RwLock};

impl Lantern {
    /// Starts the tray, the single-instance bridge (which also serves the GNOME edge extension),
    /// notifications and the global shortcuts.
    pub(super) fn start_desktop(&mut self) {
        // Screenshots older than a week go; whether Lantern opens at login is read from the system.
        std::thread::spawn(crate::os::prune_screenshots);
        self.model.prefs.open_at_login = crate::settings::autostart_enabled();
        if cfg!(target_os = "linux") {
            self.note_install_folder();
        }
        let snapshot = Arc::new(RwLock::new("[]".to_string()));
        crate::os::tray(self.tx.clone());
        crate::os::bridge(self.tx.clone(), snapshot.clone());
        self.engine.observe(self.tx.clone(), snapshot, self.model.prefs.notifications, self.model.prefs.notify_finished);
        crate::os::shortcuts(self.tx.clone(), self.model.prefs.message_hotkey.clone(), self.model.prefs.point_ask_hotkey.clone());
        self.publish_tray_menu();
    }

    /// The Mac's status menu, with live counts.
    pub(super) fn publish_tray_menu(&self) {
        let item = |label: String, id: &str| Some((label, id.to_string()));
        let shortcut = |keys: &str| if keys == "none" { String::new() } else { format!("    {}", hotkey_label(keys)) };
        let mut menu = vec![
            item(format!("Waiting ({})", self.model.inbox.new.len() + self.model.inbox.earlier.len()), "inbox"),
            item(format!("Agents ({})", self.model.agents.len()), "agents"),
            item("New Session or Task…".into(), "new"),
            item(format!("Point & Ask…{}", shortcut(&self.model.prefs.point_ask_hotkey)), "capture"),
            item(format!("Message an Agent…{}", shortcut(&self.model.prefs.message_hotkey)), "compose"),
            None,
            item(if self.windows.pill_hidden() { "Show Lantern".into() } else { "Hide Lantern".into() }, "toggle-pill"),
            item("Settings…".into(), "settings"),
        ];
        if self.model.system.update.as_ref().is_some_and(|update| update.newer) {
            menu.push(item("Check for Updates…".into(), "updates"));
        }
        menu.extend([
            None,
            item("Report a Problem…".into(), "report"),
            item("Send Logs…".into(), "logs"),
            item("Welcome Guide".into(), "welcome"),
            None,
            item("Quit October Lantern".into(), "quit"),
        ]);
        crate::os::set_tray_menu(menu);
    }

    /// A tray item, a shortcut, a notification (an agent id) or a second launch.
    pub(super) fn activate(&mut self, id: &str) {
        super::trace!("activate {id}");
        let action = match id {
            "show" => {
                self.windows.show_pill(true);
                return;
            }
            "toggle-pill" => {
                let hidden = self.windows.pill_hidden();
                if !hidden {
                    self.set_panel(None);
                }
                self.windows.show_pill(hidden);
                self.publish_tray_menu();
                return;
            }
            // A launch of the earlier app, or an old shortcut, still opens what it meant.
            "ask" => UiAction::Compose { agent_id: None },
            "connections" => UiAction::SetPanel(Some(PanelMode::October)),
            "help" => UiAction::ShowWelcome,
            "compact" => {
                self.windows.show_pill(true);
                return;
            }
            "inbox" => UiAction::SetPanel(Some(PanelMode::Inbox)),
            "agents" => UiAction::SetPanel(Some(PanelMode::Agents)),
            "new" => UiAction::StartNewSession,
            "capture" => UiAction::StartPointAsk,
            // The Point & Ask shortcut: held to point and speak (the menu and the pill's button
            // use "capture" and don't listen).
            "point-press" => return self.point_ask_pressed(),
            "point-release" => return self.point_ask_released(),
            "compose" => UiAction::Compose { agent_id: None },
            "settings" => UiAction::ShowSettings,
            "updates" => UiAction::CheckUpdates,
            "report" => UiAction::ReportProblem,
            "logs" => UiAction::SendLogs,
            "welcome" => UiAction::ShowWelcome,
            "quit" => UiAction::Quit,
            agent if self.model.agent(agent).is_some() => UiAction::OpenChat { agent_id: agent.into() },
            _ => return,
        };
        // Panels open on a visible pill.
        if matches!(action, UiAction::SetPanel(_) | UiAction::StartNewSession | UiAction::Compose { .. } | UiAction::OpenChat { .. }) {
            self.windows.show_pill(true);
        }
        self.apply(action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_read_like_the_menu_shows_them() {
        assert_eq!(hotkey_label("ctrl+alt+space"), "Ctrl+Alt+Space");
        assert_eq!(hotkey_label("ctrl+alt+p"), "Ctrl+Alt+P");
    }

    #[test]
    fn the_tray_menu_matches_the_macs_status_menu() {
        let (engine, _) = crate::sidecar::test_sender();
        let lantern = Lantern::new(engine, None);
        lantern.publish_tray_menu();
        let labels: Vec<String> = crate::os::tray_menu().into_iter().map(|item| item.map_or("—".into(), |(label, _)| label)).collect();
        assert_eq!(
            labels,
            [
                "Waiting (0)",
                "Agents (0)",
                "New Session or Task…",
                "Point & Ask…    Ctrl+Alt+P",
                "Message an Agent…    Ctrl+Alt+Space",
                "—",
                "Hide Lantern",
                "Settings…",
                "—",
                "Report a Problem…",
                "Welcome Guide",
                "—",
                "Quit October Lantern",
            ]
        );
    }

    #[test]
    fn a_notification_click_opens_that_agents_chat() {
        let (engine, _) = crate::sidecar::test_sender();
        let mut lantern = Lantern::new(engine, None);
        lantern.connected = true;
        lantern.model.agents = vec![crate::model::Agent { id: "claude:1:1".into(), ..Default::default() }];
        lantern.activate("claude:1:1");
        assert_eq!(lantern.model.chat.as_ref().map(|c| c.agent_id.as_str()), Some("claude:1:1"));
        assert_eq!(lantern.model.panel, Some(PanelMode::Inbox));
    }
}
