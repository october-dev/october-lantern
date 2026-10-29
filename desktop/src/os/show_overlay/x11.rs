use crate::show_overlay::{Display, Screen, Window};
use anyhow::{Context, Result, bail};
use eframe::egui::{Pos2, Rect, pos2, vec2};
use std::collections::HashMap;
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        randr::ConnectionExt as _,
        shape::ConnectionExt as _,
        xinput::{ConnectionExt as _, Device, EventMask as XIEventMask, XIEventMask as XIMask},
        xproto::{AtomEnum, ConfigureWindowAux, ConnectionExt as _, MapState, PropMode, StackMode},
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

pub(super) struct Desktop {
    conn: RustConnection,
    root: u32,
    screen: usize,
    atoms: HashMap<&'static str, u32>,
}

impl Desktop {
    pub fn connect() -> Result<Self> {
        anyhow::ensure!(
            !wayland(std::env::var("XDG_SESSION_TYPE").ok().as_deref(), std::env::var_os("WAYLAND_DISPLAY").is_some()),
            "Wayland doesn't provide a portable global overlay and click observer"
        );
        let (conn, screen) = x11rb::connect(None)?;
        anyhow::ensure!(!conn.query_extension(b"XWAYLAND")?.reply()?.present, "XWayland can't observe the whole desktop");
        let root = conn.setup().roots[screen].root;
        Ok(Self { conn, root, screen, atoms: HashMap::new() })
    }

    fn atom(&mut self, name: &'static str) -> Result<u32> {
        if let Some(atom) = self.atoms.get(name) {
            return Ok(*atom);
        }
        let atom = self.conn.intern_atom(false, name.as_bytes())?.reply()?.atom;
        self.atoms.insert(name, atom);
        Ok(atom)
    }

    fn numbers(&mut self, window: u32, property: &'static str, kind: AtomEnum) -> Result<Vec<u32>> {
        let atom = self.atom(property)?;
        let reply = self.conn.get_property(false, window, atom, kind, 0, 4096)?.reply()?;
        anyhow::ensure!(reply.bytes_after == 0, "Window list is too large to observe");
        Ok(reply.value32().map(Iterator::collect).unwrap_or_default())
    }

    fn monitors(&self) -> Result<Vec<Rect>> {
        let reply = self.conn.randr_get_monitors(self.root, true)?.reply()?;
        Ok(reply
            .monitors
            .iter()
            .map(|m| Rect::from_min_size(pos2(m.x as f32, m.y as f32), vec2(m.width as f32, m.height as f32)))
            .collect())
    }

    pub fn display(&mut self, previous: Option<Display>, scale: f32) -> Result<Display> {
        let monitors = self.monitors()?;
        let pointer = self.conn.query_pointer(self.root)?.reply()?;
        let at = pos2(pointer.root_x as f32, pointer.root_y as f32);
        let bounds = match previous {
            Some(display) => monitors.iter().copied().find(|r| *r == display.bounds),
            None => monitors.iter().copied().find(|r| r.contains(at)).or_else(|| monitors.first().copied()),
        }
        .context("That display is no longer connected. Start Show me again.")?;
        let display = Display { bounds, scale };
        anyhow::ensure!(display.valid(), "Couldn't read this display's geometry");
        Ok(display)
    }

    pub fn capture(&mut self, display: Display) -> Result<image::RgbaImage> {
        use x11rb::image::{Image, PixelLayout};
        let bounds = display.bounds;
        anyhow::ensure!(bounds.width() as u64 * bounds.height() as u64 <= 64_000_000, "The display is too large to capture safely");
        let (image, visual) =
            Image::get(&self.conn, self.root, bounds.min.x as i16, bounds.min.y as i16, bounds.width() as u16, bounds.height() as u16)?;
        let visual = self.conn.setup().roots[self.screen]
            .allowed_depths
            .iter()
            .flat_map(|d| &d.visuals)
            .find(|v| v.visual_id == visual)
            .context("Unsupported display visual")?;
        let colors = PixelLayout::from_visual_type(*visual)?;
        Ok(image::RgbaImage::from_fn(image.width().into(), image.height().into(), |x, y| {
            let (r, g, b) = colors.decode(image.get_pixel(x as u16, y as u16));
            image::Rgba([(r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8, 255])
        }))
    }

    pub fn screen(&mut self, display: Display) -> Result<Screen> {
        let display = self.display(Some(display), display.scale)?;
        let front = self.numbers(self.root, "_NET_ACTIVE_WINDOW", AtomEnum::WINDOW)?.first().copied().filter(|w| *w != 0);
        let front = front.filter(|w| !self.own(*w));
        let mut clients = self.numbers(self.root, "_NET_CLIENT_LIST_STACKING", AtomEnum::WINDOW)?;
        // Unmanaged menus/tooltips can also invalidate the picture. Their X root child order is
        // bottom-to-top too; Lantern's override-redirect overlay is excluded by PID.
        for window in self.conn.query_tree(self.root)?.reply()?.children {
            if !clients.contains(&window) && self.conn.get_window_attributes(window)?.reply().is_ok_and(|a| a.override_redirect) {
                clients.push(window);
            }
        }
        let mut windows = Vec::new();
        for window in clients.into_iter().rev() {
            if self.own(window) {
                continue;
            }
            let Ok(attrs) = self.conn.get_window_attributes(window)?.reply() else { continue };
            if attrs.map_state != MapState::VIEWABLE {
                continue;
            }
            let Ok(size) = self.conn.get_geometry(window)?.reply() else { continue };
            let Ok(at) = self.conn.translate_coordinates(window, self.root, 0, 0)?.reply() else { continue };
            let bounds = Rect::from_min_size(pos2(at.dst_x as f32, at.dst_y as f32), vec2(size.width as f32, size.height as f32));
            if bounds.is_positive() && bounds.intersects(display.bounds) {
                windows.push(Window { id: window.into(), bounds });
            }
        }
        Ok(Screen { display, foreground: front.map(u64::from), windows })
    }

    fn own(&mut self, window: u32) -> bool {
        self.numbers(window, "_NET_WM_PID", AtomEnum::CARDINAL).ok().and_then(|n| n.first().copied()) == Some(std::process::id())
    }

    pub fn observe(&mut self) -> Result<()> {
        let selection = self.conn.intern_atom(false, format!("_NET_WM_CM_S{}", self.screen).as_bytes())?.reply()?.atom;
        anyhow::ensure!(self.conn.get_selection_owner(selection)?.reply()?.owner != 0, "This X11 desktop has no transparency compositor");
        anyhow::ensure!(
            !self.numbers(self.root, "_NET_SUPPORTING_WM_CHECK", AtomEnum::WINDOW)?.is_empty(),
            "This X11 desktop has no EWMH window manager"
        );
        self.conn.xinput_xi_query_version(2, 0)?.reply()?;
        self.conn
            .xinput_xi_select_events(
                self.root,
                &[XIEventMask { deviceid: Device::ALL_MASTER.into(), mask: vec![XIMask::RAW_BUTTON_PRESS] }],
            )?
            .check()?;
        self.conn.flush()?;
        Ok(())
    }

    pub fn clicks(&mut self) -> Result<Vec<Pos2>> {
        let mut clicks = Vec::new();
        while let Some(event) = self.conn.poll_for_event()? {
            if let Event::XinputRawButtonPress(event) = event
                && matches!(event.detail, 1 | 3)
            {
                let at = self.conn.query_pointer(self.root)?.reply()?;
                if at.same_screen {
                    clicks.push(pos2(at.root_x as f32, at.root_y as f32));
                }
            }
        }
        Ok(clicks)
    }

    pub fn reduce_motion() -> bool {
        // Version 2 of the Settings portal standardizes reduced-motion. Older GNOME portals
        // expose enable-animations. Unknown/unavailable preferences are drawn still.
        use zbus::blocking::{Proxy, connection::Builder};
        let read = || -> Option<bool> {
            let connection = Builder::session().ok()?.method_timeout(std::time::Duration::from_millis(500)).build().ok()?;
            let proxy = Proxy::new(
                &connection,
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.Settings",
            )
            .ok()?;
            let setting: Result<zbus::zvariant::OwnedValue, _> = proxy.call("ReadOne", &("org.freedesktop.appearance", "reduced-motion"));
            if let Some(value) = setting.ok().and_then(|v| u32::try_from(v).ok()) {
                return Some(value == 1);
            }
            let value: zbus::zvariant::OwnedValue = proxy.call("ReadOne", &("org.gnome.desktop.interface", "enable-animations")).ok()?;
            bool::try_from(value).ok().map(|animations| !animations)
        };
        read().unwrap_or(true)
    }

    fn find(&mut self, title: &str) -> Result<u32> {
        let mut windows = self.numbers(self.root, "_NET_CLIENT_LIST", AtomEnum::WINDOW)?;
        windows.extend(self.conn.query_tree(self.root)?.reply()?.children);
        let name = self.atom("_NET_WM_NAME")?;
        let utf8 = self.atom("UTF8_STRING")?;
        for window in windows {
            if !self.own(window) {
                continue;
            }
            if let Ok(reply) = self.conn.get_property(false, window, name, utf8, 0, 256)?.reply()
                && reply.value == title.as_bytes()
            {
                return Ok(window);
            }
        }
        bail!("Overlay window isn't ready")
    }

    pub fn place(title: &str, display: Display) -> bool {
        (|| -> Result<()> {
            let mut x = Self::connect()?;
            let window = x.find(title)?;
            // Override-redirect skips WM focus policy. The empty input shape guarantees all
            // mouse/touch input reaches the real app below it, not just transparent pixels.
            x.conn
                .shape_rectangles(
                    x11rb::protocol::shape::SO::SET,
                    x11rb::protocol::shape::SK::INPUT,
                    x11rb::protocol::xproto::ClipOrdering::UNSORTED,
                    window,
                    0,
                    0,
                    &[],
                )?
                .check()?;
            x.conn
                .change_property32(PropMode::REPLACE, window, AtomEnum::WM_HINTS, AtomEnum::WM_HINTS, &[1, 0, 0, 0, 0, 0, 0, 0, 0])?
                .check()?;
            x.conn
                .configure_window(
                    window,
                    &ConfigureWindowAux::new()
                        .x(display.bounds.min.x as i32)
                        .y(display.bounds.min.y as i32)
                        .width(display.bounds.width() as u32)
                        .height(display.bounds.height() as u32)
                        .stack_mode(StackMode::ABOVE),
                )?
                .check()?;
            x.conn.map_window(window)?.check()?;
            x.conn.flush()?;
            Ok(())
        })()
        .is_ok()
    }

    pub fn hide(title: &str) {
        if let Ok(mut x) = Self::connect()
            && let Ok(window) = x.find(title)
        {
            let _ = x.conn.unmap_window(window);
            let _ = x.conn.flush();
        }
    }
}

fn wayland(session: Option<&str>, socket: bool) -> bool {
    socket || session.is_some_and(|s| s.eq_ignore_ascii_case("wayland"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn xwayland_is_not_mistaken_for_a_complete_desktop_observer() {
        assert!(super::wayland(Some("wayland"), false));
        assert!(super::wayland(Some("x11"), true));
        assert!(super::wayland(None, true));
        assert!(!super::wayland(Some("x11"), false));
    }

    /// Runs only on the dedicated Xvfb/Openbox/compositor desktop in CI, never a user's desktop.
    #[test]
    #[ignore = "requires an isolated X11 desktop with a compositor and EWMH window manager"]
    fn native_x11_overlay_passes_clicks_through_and_observes_window_changes() -> anyhow::Result<()> {
        use super::*;
        use std::time::{Duration, Instant};
        use x11rb::{
            COPY_DEPTH_FROM_PARENT, CURRENT_TIME,
            protocol::{
                xproto::{CreateWindowAux, EventMask, InputFocus, WindowClass},
                xtest::ConnectionExt as _,
            },
        };
        let mut desktop = Desktop::connect()?;
        let display = desktop.display(None, 1.0)?;
        let target = desktop.conn.generate_id()?;
        desktop
            .conn
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                target,
                desktop.root,
                100,
                100,
                300,
                200,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().background_pixel(0x00808080).event_mask(EventMask::BUTTON_PRESS),
            )?
            .check()?;
        desktop.conn.map_window(target)?.check()?;
        desktop.conn.flush()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let baseline = loop {
            if desktop.observe().is_ok() {
                let screen = desktop.screen(display)?;
                if screen.windows.iter().any(|w| w.id == target as u64) {
                    break screen;
                }
            }
            assert!(Instant::now() < deadline, "X11 test desktop/window didn't become ready");
            std::thread::sleep(Duration::from_millis(50));
        };
        desktop.conn.set_input_focus(InputFocus::POINTER_ROOT, target, CURRENT_TIME)?.check()?;
        std::thread::sleep(Duration::from_millis(150));
        let focus = desktop.conn.get_input_focus()?.reply()?.focus;
        let title = format!("October Lantern · Show me native-{}", uuid::Uuid::new_v4());
        let overlay = desktop.conn.generate_id()?;
        let name = desktop.atom("_NET_WM_NAME")?;
        let utf8 = desktop.atom("UTF8_STRING")?;
        let pid = desktop.atom("_NET_WM_PID")?;
        desktop
            .conn
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                overlay,
                desktop.root,
                0,
                0,
                200,
                200,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().override_redirect(1).event_mask(EventMask::BUTTON_PRESS),
            )?
            .check()?;
        desktop.conn.change_property8(PropMode::REPLACE, overlay, name, utf8, title.as_bytes())?.check()?;
        desktop.conn.change_property32(PropMode::REPLACE, overlay, pid, AtomEnum::CARDINAL, &[std::process::id()])?.check()?;
        desktop.conn.flush()?;
        assert!(Desktop::place(&title, display));
        assert_eq!(desktop.conn.get_input_focus()?.reply()?.focus, focus, "overlay must not take focus");
        let shape = desktop.conn.shape_get_rectangles(overlay, x11rb::protocol::shape::SK::INPUT)?.reply()?;
        assert!(shape.rectangles.is_empty());
        let screen = desktop.screen(display)?;
        assert!(!screen.windows.iter().any(|w| w.id == overlay as u64), "Lantern's overlay must not invalidate itself");
        let bounds = screen.windows.iter().find(|w| w.id == target as u64).unwrap().bounds;
        let point = bounds.center();
        desktop.conn.xtest_fake_input(6, 0, CURRENT_TIME, desktop.root, point.x as i16, point.y as i16, 0)?.check()?;
        desktop.conn.xtest_fake_input(4, 1, CURRENT_TIME, desktop.root, 0, 0, 0)?.check()?;
        desktop.conn.xtest_fake_input(5, 1, CURRENT_TIME, desktop.root, 0, 0, 0)?.check()?;
        desktop.conn.flush()?;
        // clicks() pumps the same connection's events: verify the unconsumed ButtonPress at the
        // target first, saving the raw event for the production observer on another connection.
        let mut observer = Desktop::connect()?;
        observer.observe()?;
        desktop.conn.xtest_fake_input(4, 1, CURRENT_TIME, desktop.root, 0, 0, 0)?.check()?;
        desktop.conn.xtest_fake_input(5, 1, CURRENT_TIME, desktop.root, 0, 0, 0)?.check()?;
        desktop.conn.flush()?;
        let mut received = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            while let Some(event) = desktop.conn.poll_for_event()? {
                if let Event::ButtonPress(event) = event {
                    received |= event.event == target;
                }
            }
            if received && !observer.clicks()?.is_empty() {
                break;
            }
            assert!(Instant::now() < deadline, "click didn't reach both target and observer");
            std::thread::sleep(Duration::from_millis(10));
        }
        Desktop::hide(&title);
        assert_ne!(desktop.conn.get_window_attributes(overlay)?.reply()?.map_state, MapState::VIEWABLE);
        let image = desktop.capture(display)?;
        assert_eq!(image.dimensions(), (display.bounds.width() as u32, display.bounds.height() as u32));
        desktop.conn.configure_window(target, &ConfigureWindowAux::new().x(150).y(150))?.check()?;
        desktop.conn.flush()?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !baseline.changed(&desktop.screen(display)?) {
            assert!(Instant::now() < deadline, "moving the target wasn't observed");
            std::thread::sleep(Duration::from_millis(20));
        }
        desktop.conn.destroy_window(target)?.check()?;
        desktop.conn.destroy_window(overlay)?.check()?;
        Ok(())
    }
}
