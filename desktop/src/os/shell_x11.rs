//! The pill and panel on X11/XWayland (CONTRACT §7, proven in the Phase 0 spike on GNOME 50):
//! work areas from `_NET_WORKAREA` and RandR, and, once a window is mapped, EWMH state (above,
//! every workspace, out of the taskbar and pager) plus a no-input hint for the pill.
use super::shell::Role;
use eframe::egui::{Rect, pos2, vec2};
use std::collections::HashMap;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, PropMode, Window};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// Lantern's windows run on X11 (XWayland under Wayland) when an X server is there: Wayland gives
/// clients no position, stacking or workspace control, which the edge pill needs.
pub fn x11_available() -> bool {
    std::env::var_os("DISPLAY").is_some()
}

struct X {
    conn: RustConnection,
    root: Window,
    atoms: HashMap<&'static str, u32>,
}

impl X {
    fn connect() -> Option<Self> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots[screen].root;
        Some(Self { conn, root, atoms: HashMap::new() })
    }
    fn atom(&mut self, name: &'static str) -> u32 {
        if let Some(atom) = self.atoms.get(name) {
            return *atom;
        }
        let atom = self.conn.intern_atom(false, name.as_bytes()).ok().and_then(|c| c.reply().ok()).map_or(0, |r| r.atom);
        self.atoms.insert(name, atom);
        atom
    }
    fn cardinals(&mut self, window: Window, name: &'static str, kind: impl Into<u32>) -> Vec<u32> {
        self.cardinals_up_to(window, name, kind, 4096)
    }
    /// Up to `max` 32-bit values (icons need far more than other properties).
    fn cardinals_up_to(&mut self, window: Window, name: &'static str, kind: impl Into<u32>, max: u32) -> Vec<u32> {
        let atom = self.atom(name);
        self.conn
            .get_property(false, window, atom, kind, 0, max)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|reply| reply.value32().map(Iterator::collect))
            .unwrap_or_default()
    }
    fn title(&mut self, window: Window) -> String {
        let (name, utf8) = (self.atom("_NET_WM_NAME"), self.atom("UTF8_STRING"));
        self.conn
            .get_property(false, window, name, utf8, 0, 256)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|reply| String::from_utf8_lossy(&reply.value).into_owned())
            .unwrap_or_default()
    }
    /// One of this process's managed windows, by title.
    fn find(&mut self, title: &str) -> Option<Window> {
        let pid = std::process::id();
        let clients = self.cardinals(self.root, "_NET_CLIENT_LIST", AtomEnum::WINDOW);
        clients
            .into_iter()
            .find(|&window| self.cardinals(window, "_NET_WM_PID", AtomEnum::CARDINAL).first() == Some(&pid) && self.title(window) == title)
    }
    /// Asks the window manager to change a mapped window's state (EWMH client message).
    fn request(&mut self, window: Window, message: &'static str, data: [u32; 5]) {
        let kind = self.atom(message);
        let event = ClientMessageEvent::new(32, window, kind, data);
        let _ = self.conn.send_event(false, self.root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event);
    }
}

/// Whole monitors in physical pixels (RandR), for Point & Ask's selection overlay.
pub fn monitors() -> Vec<Rect> {
    let Some(x) = X::connect() else { return Vec::new() };
    let monitors: Vec<Rect> = x
        .conn
        .randr_get_monitors(x.root, true)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|reply| {
            reply
                .monitors
                .iter()
                .map(|m| Rect::from_min_size(pos2(m.x as f32, m.y as f32), vec2(m.width as f32, m.height as f32)))
                .collect()
        })
        .unwrap_or_default();
    if monitors.is_empty() {
        let screen = &x.conn.setup().roots[0];
        return vec![Rect::from_min_size(pos2(0.0, 0.0), vec2(screen.width_in_pixels as f32, screen.height_in_pixels as f32))];
    }
    monitors
}

/// Work areas in physical pixels, one per monitor: each RandR monitor clipped to `_NET_WORKAREA`
/// (which excludes panels like GNOME's top bar). Empty when there is no X server.
pub fn work_areas() -> Vec<Rect> {
    let Some(mut x) = X::connect() else { return Vec::new() };
    let root = x.root;
    let work = x.cardinals(root, "_NET_WORKAREA", AtomEnum::CARDINAL);
    let screen = &x.conn.setup().roots[0];
    let whole = Rect::from_min_size(pos2(0.0, 0.0), vec2(screen.width_in_pixels as f32, screen.height_in_pixels as f32));
    let work = if work.len() >= 4 {
        Rect::from_min_size(pos2(work[0] as f32, work[1] as f32), vec2(work[2] as f32, work[3] as f32))
    } else {
        whole
    };
    let monitors: Vec<Rect> = x
        .conn
        .randr_get_monitors(root, true)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|reply| {
            reply
                .monitors
                .iter()
                .map(|m| Rect::from_min_size(pos2(m.x as f32, m.y as f32), vec2(m.width as f32, m.height as f32)))
                .collect()
        })
        .unwrap_or_default();
    if monitors.is_empty() {
        return vec![work];
    }
    monitors.into_iter().map(|monitor| monitor.intersect(work)).filter(|area| area.is_positive()).collect()
}

/// Gives a mapped Lantern window its role (CONTRACT §7). Returns false until the window exists,
/// so the caller tries again next frame.
pub fn adopt(title: &str, role: Role) -> bool {
    let Some(mut x) = X::connect() else { return false };
    let Some(window) = x.find(title) else { return false };
    if role == Role::Pill {
        // WM_HINTS: flags = InputHint, input = false. Clicks still arrive; focus stays where it was.
        let hints = AtomEnum::WM_HINTS;
        let _ = x.conn.change_property32(PropMode::REPLACE, window, hints, hints, &[1, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
    let (above, sticky) = (x.atom("_NET_WM_STATE_ABOVE"), x.atom("_NET_WM_STATE_STICKY"));
    let (taskbar, pager) = (x.atom("_NET_WM_STATE_SKIP_TASKBAR"), x.atom("_NET_WM_STATE_SKIP_PAGER"));
    // _NET_WM_STATE_ADD = 1; source indication 1 = a normal application.
    x.request(window, "_NET_WM_STATE", [1, above, sticky, 1, 0]);
    x.request(window, "_NET_WM_STATE", [1, taskbar, pager, 1, 0]);
    x.request(window, "_NET_WM_DESKTOP", [0xFFFF_FFFF, 1, 0, 0, 0]);
    x.conn.flush().is_ok()
}

/// Apps where a Task makes no sense (a normal session instead), by WM_CLASS.
const TERMINALS: [&str; 14] = [
    "gnome-terminal",
    "org.gnome.ptyxis",
    "org.gnome.console",
    "kgx",
    "konsole",
    "kitty",
    "alacritty",
    "org.wezfurlong.wezterm",
    "wezterm",
    "xterm",
    "tilix",
    "terminator",
    "com.mitchellh.ghostty",
    "foot",
];

/// The app in front, when it's an X11 app (CONTRACT §8.5: Wayland-native apps can't be read).
/// Lantern's own windows never take focus, so this is still the person's app when they click the pill.
pub fn foreground_app() -> Option<crate::model::AppContext> {
    let mut x = X::connect()?;
    let root = x.root;
    let window = *x.cardinals(root, "_NET_ACTIVE_WINDOW", AtomEnum::WINDOW).first()?;
    if window == 0 || x.cardinals(window, "_NET_WM_PID", AtomEnum::CARDINAL).first() == Some(&std::process::id()) {
        return None;
    }
    let class = x.conn.get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256).ok()?.reply().ok()?;
    let mut parts = class.value.split(|&byte| byte == 0).map(|part| String::from_utf8_lossy(part).into_owned());
    let (instance, class) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
    if class.is_empty() && instance.is_empty() {
        return None; // XWayland's stand-in while a Wayland-native app has focus
    }
    let id = if instance.is_empty() { class.clone() } else { instance };
    let is_terminal = TERMINALS.iter().any(|terminal| terminal.eq_ignore_ascii_case(&id) || terminal.eq_ignore_ascii_case(&class));
    let title = x.title(window);
    let icon = app_icon(&mut x, window, &id);
    Some(crate::model::AppContext {
        name: if class.is_empty() { id.clone() } else { class },
        is_terminal,
        window: (!title.is_empty()).then_some(title),
        pid: x.cardinals(window, "_NET_WM_PID", AtomEnum::CARDINAL).first().map(|&pid| pid as i64),
        icon,
        id,
        ..Default::default()
    })
}

/// The window's `_NET_WM_ICON` closest to 64 px, saved as a PNG under Lantern's data folder.
fn app_icon(x: &mut X, window: Window, id: &str) -> Option<std::path::PathBuf> {
    let data = x.cardinals_up_to(window, "_NET_WM_ICON", AtomEnum::CARDINAL, 1 << 20);
    let (width, height, pixels) = pick_icon(&data, 64)?;
    let rgba: Vec<u8> = pixels.iter().flat_map(|&argb| [(argb >> 16) as u8, (argb >> 8) as u8, argb as u8, (argb >> 24) as u8]).collect();
    let image = image::RgbaImage::from_raw(width, height, rgba)?;
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    let path = crate::settings::data_dir().join("icons").join(format!("{safe}.png"));
    std::fs::create_dir_all(path.parent()?).ok()?;
    image.save(&path).ok()?;
    Some(path)
}

/// `_NET_WM_ICON` is width, height, then width*height ARGB pixels, repeated per size. Picks the
/// smallest at least `want` wide (else the largest).
fn pick_icon(data: &[u32], want: u32) -> Option<(u32, u32, &[u32])> {
    let mut icons = Vec::new();
    let mut rest = data;
    while rest.len() >= 2 {
        let (width, height) = (rest[0], rest[1]);
        let len = (width as usize).checked_mul(height as usize)?;
        if width == 0 || height == 0 || rest.len() < 2 + len {
            break;
        }
        icons.push((width, height, &rest[2..2 + len]));
        rest = &rest[2 + len..];
    }
    icons.iter().filter(|icon| icon.0 >= want).min_by_key(|icon| icon.0).or_else(|| icons.iter().max_by_key(|icon| icon.0)).copied()
}

/// Whether one of `any` keysyms and one of `and` keysyms are both held (X11's keymap). One
/// connection, kept; the keysym table is read once.
pub fn keys_down(any: &[u32], and: &[u32]) -> bool {
    /// The connection, and each keycode's keysyms.
    type Keyboard = (RustConnection, Vec<(u8, u32)>);
    static STATE: std::sync::Mutex<Option<Keyboard>> = std::sync::Mutex::new(None);
    if !x11_available() {
        return false;
    }
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.is_none() {
        let Ok((conn, _)) = x11rb::connect(None) else { return false };
        let (min, max) = (conn.setup().min_keycode, conn.setup().max_keycode);
        let Some(map) = conn.get_keyboard_mapping(min, max - min + 1).ok().and_then(|c| c.reply().ok()) else { return false };
        let per = map.keysyms_per_keycode.max(1) as usize;
        let table = map.keysyms.chunks(per).enumerate().flat_map(|(i, syms)| syms.iter().map(move |s| (min + i as u8, *s))).collect();
        *state = Some((conn, table));
    }
    let Some((conn, table)) = state.as_ref() else { return false };
    let Some(keys) = conn.query_keymap().ok().and_then(|c| c.reply().ok()).map(|r| r.keys) else { return false };
    let held = |sym: &u32| table.iter().any(|(code, s)| s == sym && keys[*code as usize / 8] & (1 << (code % 8)) != 0);
    any.iter().any(held) && and.iter().any(held)
}

/// The global pointer isn't reliable under XWayland (stale over Wayland-native windows), so hover
/// comes from Lantern's own windows (CONTRACT §8).
pub fn cursor_position() -> Option<eframe::egui::Pos2> {
    None
}

#[cfg(test)]
mod tests {
    use super::pick_icon;

    #[test]
    fn picks_the_smallest_icon_at_least_the_wanted_size() {
        let mut data = vec![2, 2, 0xff00_0000, 0, 0, 0];
        data.extend([4, 4]);
        data.extend([1u32; 16]);
        data.extend([128, 1]);
        data.extend([2u32; 128]);
        assert_eq!(pick_icon(&data, 3).map(|(w, h, _)| (w, h)), Some((4, 4)));
        assert_eq!(pick_icon(&data, 200).map(|(w, h, _)| (w, h)), Some((128, 1)), "else the largest");
        assert_eq!(pick_icon(&[5, 5, 1], 64), None, "truncated data is ignored");
    }
}
