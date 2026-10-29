//! The pill (root viewport) and the panel beside it (a second native window), positioned and
//! behaving like the Mac's (CONTRACT §6). Geometry is `shell`'s; platform roles are `os::shell`'s.
use super::Lantern;
use crate::os::Event;
use crate::os::shell::{self as platform, PANEL_TITLE, PILL_TITLE, Role};
use crate::shell::{self, Hover, PILL_COLLAPSED, PILL_EXPANDED};
use crate::ui::{self, Ctx, Env};
use eframe::egui::{self, Pos2, Rect, Vec2, ViewportBuilder, ViewportCommand, ViewportId, vec2};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// Swift: the panel fades in over .12 s; a snap animates over .18 s.
const FADE: f32 = 0.12;
const TIP_TITLE: &str = "October Lantern tip";
const SNAP: f32 = 0.18;
/// How often the work areas are re-read (monitors and panels change rarely).
const AREAS_EVERY: Duration = Duration::from_secs(2);

struct Drag {
    window_start: Pos2,
    pointer_start: Pos2,
}

struct Snap {
    from: Pos2,
    started: Instant,
}

#[derive(Default)]
pub struct Windows {
    hover: Hover,
    /// Work areas in points, and when they were read.
    areas: Vec<Rect>,
    areas_at: Option<Instant>,
    pill_adopted: bool,
    panel_adopted: bool,
    frames: u64,
    /// The pill's position and size as last sent to the window.
    pill_at: Option<Pos2>,
    pill_size: Option<Vec2>,
    drag: Option<Drag>,
    snap: Option<Snap>,
    panel_height: f32,
    panel_opened: Option<Instant>,
    panel_focused: bool,
    /// Bring the panel to the front on its next frame (back from signing in in the browser).
    raise_panel: bool,
    /// A dialog Lantern opened (folder picker, capture permission) is up: losing focus to it
    /// doesn't close the panel.
    pub native_dialog: bool,
    clipboard: Option<String>,
    quitting: bool,
    /// A screen capture waiting for Lantern's windows to be cleared, and how many frames they
    /// have been cleared for.
    capture: Option<Box<dyn FnOnce(Sender<Event>) + Send>>,
    cleared_frames: u32,
    capturing: bool,
    /// The pill's help text under the pointer, and since when (shown after a moment).
    tip: Option<(String, Rect, Vec2, f64)>,
    /// The tip window's system frame is off (Windows).
    tip_unframed: bool,
    /// Point & Ask's overlay and card.
    pub select: super::select::State,
    /// The lantern's right-click menu.
    pub menu: super::menu::State,
    /// Settings and Welcome.
    pub dialogs: super::dialogs::State,
    /// Expand the pill for a few seconds (after Welcome), on the next frame.
    flash: bool,
    /// Hidden from the tray ("Hide Lantern"), and whether the window has been told.
    hidden: bool,
    hidden_sent: bool,
}

impl Windows {
    pub fn animating(&self) -> bool {
        self.drag.is_some()
            || self.snap.is_some()
            || self.panel_opened.is_some_and(|at| at.elapsed().as_secs_f32() < FADE)
            || self.hover.expanded
    }
    pub fn flash(&mut self) {
        self.flash = true;
    }
    pub fn pill_hidden(&self) -> bool {
        self.hidden
    }
    pub fn show_pill(&mut self, visible: bool) {
        self.hidden = !visible;
    }
    /// Brings the panel (open, or about to open) to the front.
    pub fn raise_panel(&mut self) {
        self.raise_panel = true;
    }
    #[cfg(test)]
    pub fn raising_panel(&self) -> bool {
        self.raise_panel
    }
    /// The pill's window, in points, once placed.
    pub fn pill_rect(&self) -> Option<Rect> {
        self.pill_at.map(|at| Rect::from_min_size(at, self.pill_size.unwrap_or(PILL_COLLAPSED)))
    }
    pub fn work_areas(&self) -> Vec<Rect> {
        self.areas.clone()
    }
    pub fn copy(&mut self, text: String) {
        self.clipboard = Some(text);
    }
    pub fn quit(&mut self) {
        self.quitting = true;
    }
    #[cfg(test)]
    pub fn clipboard(&self) -> Option<&str> {
        self.clipboard.as_deref()
    }
    #[cfg(test)]
    pub fn quitting(&self) -> bool {
        self.quitting
    }
    /// Runs `start` once Lantern's windows have drawn nothing for a couple of frames, so the capture
    /// leaves them out (the Mac excludes its own windows; portals and GDI can't). The windows stay
    /// mapped (unmapping would reset their window-manager state) and draw again on `capture_done`.
    pub fn hide_for_capture(&mut self, start: impl FnOnce(Sender<Event>) + Send + 'static) {
        self.capture = Some(Box::new(start));
        self.cleared_frames = 0;
        self.capturing = true;
    }
    pub fn capture_done(&mut self) {
        self.capturing = false;
        self.capture = None;
    }
    fn cleared(&self) -> bool {
        self.capturing
    }
}

/// Work areas in points (the platform reports physical pixels).
fn areas_in_points(scale: f32) -> Vec<Rect> {
    platform::work_areas()
        .into_iter()
        .map(|area| Rect::from_min_max((area.min.to_vec2() / scale).to_pos2(), (area.max.to_vec2() / scale).to_pos2()))
        .collect()
}

/// The root viewport: the pill, collapsed, where the Mac would put it.
pub fn options() -> eframe::NativeOptions {
    let prefs = crate::model::Prefs::default(); // TODO(apollo): persisted edge/top
    let start = areas_in_points(1.0)
        .first()
        .map(|work| shell::pill_frame(*work, PILL_COLLAPSED, prefs.edge, prefs.pill_top).min)
        .unwrap_or(Pos2::new(100.0, 100.0));
    #[allow(unused_mut)]
    let mut viewport = ViewportBuilder::default()
        .with_icon(crate::ui::assets::window_icon())
        .with_title(PILL_TITLE)
        .with_app_id("dev.october.lantern")
        .with_inner_size(PILL_COLLAPSED)
        .with_position(start)
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top()
        .with_taskbar(false);
    #[cfg(target_os = "linux")]
    {
        // Proven in Phase 0: a dock window never takes focus; clicks still arrive.
        viewport = viewport.with_window_type(egui::X11WindowType::Dock);
    }
    eframe::NativeOptions {
        viewport,
        #[cfg(target_os = "linux")]
        event_loop_builder: platform::x11_available().then(|| -> eframe::EventLoopBuilderHook {
            Box::new(|builder| {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_x11();
            })
        }),
        ..Default::default()
    }
}

impl Lantern {
    /// One frame of the pill and, when open, the panel.
    pub(super) fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.show_overlay(&ctx);
        let w = &mut self.windows;
        w.frames += 1;
        let scale = ctx.native_pixels_per_point().unwrap_or(1.0);
        if w.areas_at.is_none_or(|at| at.elapsed() >= AREAS_EVERY) {
            w.areas = areas_in_points(scale);
            w.areas_at = Some(Instant::now());
        }
        if let Some(text) = w.clipboard.take() {
            ctx.copy_text(text);
        }
        // Hide / Show Lantern. A window shown again is a new map: its window-manager state
        // (above, every workspace, no taskbar) and position are given again.
        if w.hidden != w.hidden_sent {
            w.hidden_sent = w.hidden;
            ctx.send_viewport_cmd(ViewportCommand::Visible(!w.hidden));
            if !w.hidden {
                w.pill_adopted = false;
                w.pill_at = None;
                w.pill_size = None;
            }
        }
        if w.quitting {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        // Give the native windows their roles once they exist (on top, every workspace, no
        // taskbar; the pill never takes focus).
        if !w.pill_adopted && w.frames.is_multiple_of(5) {
            w.pill_adopted = platform::adopt(PILL_TITLE, Role::Pill);
        }

        // Hover: our own window's pointer events; on Windows also the polled 6pt halo.
        let now = ctx.input(|input| input.time);
        let outer = ctx.input(|input| input.viewport().outer_rect);
        let inside_window = ctx.input(|input| input.pointer.hover_pos().is_some());
        let in_halo = self.model.caps.hover_halo
            && platform::cursor_position()
                .zip(outer)
                .is_some_and(|(cursor, rect)| rect.expand(6.0).contains((cursor.to_vec2() / scale).to_pos2()));
        if std::mem::take(&mut w.flash) {
            w.hover.flash(now);
        }
        if w.drag.is_none() {
            let was = w.hover.expanded;
            w.hover.update(inside_window || in_halo, now);
            if was != w.hover.expanded {
                super::trace!("hover expanded={}", w.hover.expanded);
            }
        }
        let expanded = shell::pill_expanded(&w.hover, self.model.panel.is_some(), self.model.dictating);
        self.model.pill_expanded = expanded;

        // Where the pill goes.
        let size = if expanded { PILL_EXPANDED } else { PILL_COLLAPSED };
        let current = outer.map_or(Pos2::ZERO, |rect| rect.center());
        let work = shell::work_area_at(&w.areas, current).unwrap_or(Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0)));
        let home = shell::pill_frame(work, size, self.model.prefs.edge, self.model.prefs.pill_top).min;
        let at = match (&w.drag, &w.snap) {
            (Some(_), _) => w.pill_at.unwrap_or(home),
            (None, Some(snap)) => {
                let t = (snap.started.elapsed().as_secs_f32() / SNAP).min(1.0);
                let eased = 1.0 - (1.0 - t).powi(3);
                snap.from + (home - snap.from) * eased
            }
            (None, None) => home,
        };
        if w.snap.as_ref().is_some_and(|snap| snap.started.elapsed().as_secs_f32() >= SNAP) {
            w.snap = None;
        }
        if w.pill_size != Some(size) {
            // A fixed-size window: min and max follow the size, or the window manager may keep
            // enforcing the expanded size after the pill collapses.
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::MaxInnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            w.pill_size = Some(size);
        }
        if w.pill_at.is_none_or(|old| (old - at).length() > 0.5) {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(at));
            w.pill_at = Some(at);
        }

        // A capture waiting for Lantern's windows to clear: draw nothing, then start it.
        if w.cleared() {
            w.cleared_frames += 1;
            if w.cleared_frames >= 3
                && let Some(start) = w.capture.take()
            {
                start(self.tx.clone());
            }
            ctx.request_repaint();
            self.panel(&ctx, work, scale);
            self.point_ask_windows(&ctx, scale, true);
            return;
        }
        // The pill.
        let assets = self.assets.as_ref().expect("assets load before the first frame");
        let input = egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| shell::pill::show(ui, &self.model, assets, expanded, &mut self.actions))
            .inner;
        let w = &mut self.windows;
        if let (Some(local), Some(at)) = (input.menu_at, w.pill_at) {
            w.menu.open(at + local.to_vec2());
            w.tip = None;
        }
        w.tip = match (input.help, w.tip.take()) {
            (Some((text, rect, size)), Some((old, _, _, since))) if old == text => Some((text, rect, size, since)),
            (Some((text, rect, size)), _) => Some((text, rect, size, now)),
            (None, _) => None,
        };
        // Dragging by the lantern moves the pill (and the panel with it); letting go snaps it to
        // the nearer edge. Positions come from the window, never the global pointer (CONTRACT §8.2).
        let pointer = ctx.input(|input| input.pointer.interact_pos()).zip(outer).map(|(local, rect)| rect.min + local.to_vec2());
        if input.drag_started
            && let (Some(pointer), Some(window)) = (pointer, w.pill_at)
        {
            super::trace!("drag start at {window:?}");
            w.drag = Some(Drag { window_start: window, pointer_start: pointer });
            w.snap = None;
        }
        if input.dragging
            && let (Some(drag), Some(pointer)) = (&w.drag, pointer)
        {
            let to = drag.window_start + (pointer - drag.pointer_start);
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(to));
            w.pill_at = Some(to);
        }
        if input.drag_stopped && w.drag.take().is_some() {
            let dropped = Rect::from_min_size(w.pill_at.unwrap_or(home), size);
            let (edge, top) = shell::snap(dropped, work);
            super::trace!("drag stop at {:?} -> snap {edge:?} top {top:.3}", dropped.min);
            self.model.prefs.edge = edge;
            self.model.prefs.pill_top = top;
            w.snap = Some(Snap { from: dropped.min, started: Instant::now() });
            self.save_prefs();
        }

        self.tip(&ctx, size);
        self.panel(&ctx, work, scale);
        self.point_ask_windows(&ctx, scale, false);
        self.menu(&ctx);
        self.dialogs(&ctx);
    }

    /// The pill's help text, beside the pill, after half a second (a native tooltip on the Mac).
    fn tip(&mut self, ctx: &egui::Context, pill_size: Vec2) {
        let w = &self.windows;
        let now = ctx.input(|input| input.time);
        let Some((text, rect, text_size, since)) = w.tip.clone().filter(|tip| now - tip.3 >= 0.5 && w.drag.is_none()) else {
            self.windows.tip_unframed = false;
            if self.windows.tip.is_some() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            return;
        };
        let _ = since;
        let size = text_size + vec2(16.0, 10.0);
        let pill = w.pill_at.unwrap_or_default();
        let x = match self.model.prefs.edge {
            crate::model::Edge::Right => pill.x - size.x - 8.0,
            crate::model::Edge::Left => pill.x + pill_size.x + 8.0,
        };
        let at = egui::pos2(x, pill.y + rect.center().y - size.y / 2.0);
        #[allow(unused_mut)]
        let mut builder = ViewportBuilder::default()
            .with_icon(crate::ui::assets::window_icon())
            .with_title(TIP_TITLE)
            .with_inner_size(size)
            .with_position(at)
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_active(false)
            .with_mouse_passthrough(true);
        #[cfg(target_os = "linux")]
        {
            builder = builder.with_window_type(egui::X11WindowType::Tooltip);
        }
        if !self.windows.tip_unframed {
            self.windows.tip_unframed = platform::borderless(TIP_TITLE);
        }
        ctx.show_viewport_immediate(ViewportId::from_hash_of("lantern-tip"), builder, |ui, _| {
            let rect = ui.max_rect();
            ui.painter().rect_filled(rect, 6.0, ui::theme::black(0.85));
            ui.painter().rect_stroke(rect.shrink(0.5), 6.0, egui::Stroke::new(1.0, ui::theme::STROKE), egui::StrokeKind::Inside);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &text, shell::pill::help_font(), ui::theme::white(0.9));
        });
    }

    /// The panel window beside the pill, while a panel is open.
    fn panel(&mut self, ctx: &egui::Context, work: Rect, scale: f32) {
        let guide_target = (self.model.panel == Some(crate::model::PanelMode::ShowMe)).then(|| self.show_target()).flatten();
        let w = &mut self.windows;
        if self.model.panel.is_none() {
            w.panel_opened = None;
            w.panel_adopted = false;
            w.panel_focused = false;
            return;
        }
        let opened = *w.panel_opened.get_or_insert_with(Instant::now);
        let pill = Rect::from_min_size(w.pill_at.unwrap_or_default(), w.pill_size.unwrap_or(PILL_COLLAPSED));
        let edge = self.model.prefs.edge;
        let wanted = vec2(ui::theme::PANEL_WIDTH, if w.panel_height > 0.0 { w.panel_height } else { 200.0 });
        let mut frame = shell::panel_frame(pill, wanted, edge, work);
        if let Some(target) = guide_target {
            let target = Rect::from_min_max((target.min.to_vec2() / scale).to_pos2(), (target.max.to_vec2() / scale).to_pos2());
            frame = crate::show_overlay::panel_away(frame, work, target);
        }
        if !w.panel_adopted && w.frames.is_multiple_of(5) {
            w.panel_adopted = platform::adopt(PANEL_TITLE, Role::Panel);
        }
        let builder = ViewportBuilder::default()
            .with_icon(crate::ui::assets::window_icon())
            .with_title(PANEL_TITLE)
            .with_inner_size(frame.size())
            .with_position(frame.min)
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false);
        let max_height = shell::panel_limit(work);
        let env = Env { now_ms: Self::now_ms(), reduce_motion: false, home: crate::settings::home().to_string_lossy().into_owned() };
        let (model, inputs, view, out) = (&self.model, &mut self.inputs, &mut self.view, &mut self.actions);
        let assets = self.assets.as_ref().expect("assets load before the first frame");
        let mut measured = None;
        let mut focused = None;
        let cleared = self.windows.cleared();
        let raise = std::mem::take(&mut self.windows.raise_panel);
        ctx.show_viewport_immediate(ViewportId::from_hash_of("lantern-panel"), builder, |ui, _class| {
            ctx_commands(ui.ctx(), frame);
            if raise {
                ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
                ui.ctx().send_viewport_cmd(ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
            }
            if cleared {
                return; // a screen capture is waiting for Lantern's windows to clear
            }
            let fade = (opened.elapsed().as_secs_f32() / FADE).min(1.0);
            ui.set_opacity(fade);
            // The panel surface draws its own glass and handles Esc (ui::render_panel).
            let mut cx = Ctx { model, inputs, view, out, assets, env: &env };
            let height =
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| ui::render_panel(ui, &mut cx, max_height)).inner;
            measured = Some(height);
            focused = ui.input(|input| input.viewport().focused);
        });
        let w = &mut self.windows;
        if let Some(height) = measured
            && (height - w.panel_height).abs() > 0.5
        {
            w.panel_height = height.min(max_height);
        }
        // Losing focus closes an idle panel (a proxy for the Mac's outside click, CONTRACT §6).
        if let Some(now_focused) = focused {
            if w.panel_focused != now_focused {
                super::trace!("panel focused={now_focused}");
            }
            let lost_focus = w.panel_focused && !now_focused;
            w.panel_focused = now_focused;
            if lost_focus {
                self.panel_lost_focus();
            }
        }
    }

    pub(super) fn panel_lost_focus(&mut self) {
        // Doing the highlighted step means returning to the real app. Closing this panel
        // would call stop_show and cancel the click's pending Next before it could run.
        if self.model.panel == Some(crate::model::PanelMode::ShowMe) || self.model.panel == Some(crate::model::PanelMode::October) {
            return;
        }
        if shell::closes_on_focus_loss(self.inputs.drafts.is_empty(), self.model.dictating, self.windows.native_dialog, false) {
            self.actions.push(crate::ui::UiAction::SetPanel(None));
        }
    }
}

/// Keeps the panel window where and as big as `frame` says.
fn ctx_commands(ctx: &egui::Context, frame: Rect) {
    let (outer, inner) = ctx.input(|input| (input.viewport().outer_rect, input.viewport().inner_rect));
    if outer.is_none_or(|rect| (rect.min - frame.min).length() > 0.5) {
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(frame.min));
    }
    if inner.is_none_or(|rect| (rect.size() - frame.size()).length() > 0.5) {
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(frame.size()));
    }
}
