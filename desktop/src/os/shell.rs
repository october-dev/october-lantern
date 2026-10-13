//! What the pill and panel need from the system that egui/winit don't provide: work areas, the
//! window roles (on top, every workspace, out of the taskbar, a pill that never takes focus),
//! and, where it's reliable, the global pointer (CONTRACT §6–7).
#[cfg(windows)]
pub use super::shell_win::{adopt, borderless, cursor_position, foreground_app, monitors, work_areas};
#[cfg(target_os = "linux")]
pub use super::shell_x11::{adopt, cursor_position, foreground_app, monitors, work_areas, x11_available};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Never takes focus; clicks still arrive.
    Pill,
    /// Takes focus when clicked; losing it closes an idle panel.
    Panel,
    /// The Point & Ask card and selection overlay: like the panel.
    Card,
}

/// Window titles, so the platform layer can find each viewport's native window.
pub const PILL_TITLE: &str = "October Lantern";
pub const PANEL_TITLE: &str = "October Lantern panel";
pub const SELECT_TITLE: &str = "October Lantern · Select";
pub const CARD_TITLE: &str = "October Lantern · Point & Ask";

/// Windows takes the system frame off the menu and the tip; X11 never draws one on them.
#[cfg(target_os = "linux")]
pub fn borderless(_title: &str) -> bool {
    true
}

#[cfg(windows)]
pub fn x11_available() -> bool {
    false
}
