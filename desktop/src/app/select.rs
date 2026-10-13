//! Point & Ask's windows (Swift `SelectionOverlay` and `PointAsk.showCard/fit`): a transparent
//! overlay over the whole monitor to drag a box (or click for the whole screen), then the card
//! beside the selection.
//!
//! The overlay is fullscreen so it covers the top bar and takes the keyboard (Enter, Esc). Wayland
//! gives Lantern no list of other apps' windows, so "click a window" is the whole screen here
//! (CONTRACT §8.5); the hint says so.
use super::Lantern;
use crate::model::PointAskPhase;
use crate::os::shell::{self as platform, CARD_TITLE, Role, SELECT_TITLE};
use crate::ui::{self, Ctx, Env, theme};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, ViewportBuilder, ViewportCommand, ViewportId, pos2,
    vec2,
};

/// A drag smaller than this is a click (SelectionOverlay.swift).
const MIN_SELECTION: f32 = 8.0;
const CARD_WIDTH: f32 = 400.0;
const CARD_GAP: f32 = 14.0;
const MARGIN: f32 = 8.0;

#[derive(Default)]
pub struct State {
    drag: Option<(Pos2, Pos2)>,
    overlay_adopted: bool,
    card_adopted: bool,
    /// The card's top-left once placed; it keeps its top as the answer grows.
    card_at: Option<Pos2>,
    card_height: f32,
}

/// Where the card goes (Swift `fit`): to the right of the selection, else the left, else inside
/// its right edge; tops aligned with the selection, all kept on the work area. Points.
pub fn card_frame(selection: Rect, height: f32, work: Rect) -> Rect {
    let height = height.clamp(160.0, work.height() - 2.0 * MARGIN);
    let mut x = selection.max.x + CARD_GAP;
    if x + CARD_WIDTH > work.max.x - MARGIN {
        x = selection.min.x - CARD_WIDTH - CARD_GAP;
    }
    if x < work.min.x + MARGIN {
        x = (selection.max.x - CARD_WIDTH - CARD_GAP).min(work.max.x - CARD_WIDTH - MARGIN);
    }
    x = x.max(work.min.x + MARGIN);
    let y = selection.min.y.min(work.max.y - height - MARGIN).max(work.min.y + MARGIN);
    Rect::from_min_size(pos2(x, y), vec2(CARD_WIDTH, height))
}

impl Lantern {
    /// The overlay while selecting, the card once there's a selection.
    pub(super) fn point_ask_windows(&mut self, ctx: &egui::Context, scale: f32, cleared: bool) {
        match self.model.point_ask.phase {
            PointAskPhase::Selecting if !cleared => self.overlay(ctx, scale),
            PointAskPhase::Card => self.card(ctx, scale, cleared),
            _ => {
                self.windows.select.drag = None;
                self.windows.select.overlay_adopted = false;
                if self.model.point_ask.phase == PointAskPhase::Closed {
                    self.windows.select.card_adopted = false;
                    self.windows.select.card_at = None;
                }
            }
        }
    }

    /// The monitor the pill is on (physical pixels), else the first.
    fn pointing_monitor(&self) -> Option<Rect> {
        let monitors = platform::monitors();
        let pill = self.windows.pill_rect();
        monitors.iter().copied().find(|m| pill.is_some_and(|p| m.contains(p.center()))).or_else(|| monitors.first().copied())
    }

    fn overlay(&mut self, ctx: &egui::Context, scale: f32) {
        let Some(monitor) = self.pointing_monitor() else { return };
        let points = Rect::from_min_max((monitor.min.to_vec2() / scale).to_pos2(), (monitor.max.to_vec2() / scale).to_pos2());
        let state = &mut self.windows.select;
        if !state.overlay_adopted {
            state.overlay_adopted = platform::adopt(SELECT_TITLE, Role::Card);
        }
        let builder = ViewportBuilder::default()
            .with_icon(crate::ui::assets::window_icon())
            .with_title(SELECT_TITLE)
            .with_position(points.min)
            .with_inner_size(points.size())
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_fullscreen(true);
        let window_hit_test = self.model.caps.window_hit_test;
        let mut outcome: Option<Option<Rect>> = None;
        ctx.show_viewport_immediate(ViewportId::from_hash_of("lantern-select"), builder, |ui, _| {
            let area = ui.max_rect();
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            let response = ui.interact(area, ui.id().with("select"), Sense::click_and_drag());
            if response.drag_started()
                && let Some(at) = response.interact_pointer_pos()
            {
                state.drag = Some((at, at));
            }
            if let (Some((_, now)), Some(at)) = (state.drag.as_mut(), response.interact_pointer_pos()) {
                *now = at;
            }
            let selection = state.drag.map(|(a, b)| Rect::from_two_pos(a, b));
            draw_overlay(ui, area, selection, window_hit_test);
            let whole = Some(area);
            if ui.input(|input| input.key_pressed(egui::Key::Escape)) || response.secondary_clicked() {
                outcome = Some(None);
            } else if ui.input(|input| input.key_pressed(egui::Key::Enter)) || response.clicked() {
                outcome = Some(whole);
            } else if response.drag_stopped() {
                let chosen = selection.filter(|r| r.width() >= MIN_SELECTION && r.height() >= MIN_SELECTION).or(whole);
                outcome = Some(chosen);
                state.drag = None;
            }
        });
        if let Some(chosen) = outcome {
            self.windows.select.drag = None;
            self.windows.select.overlay_adopted = false;
            // Overlay-local points → screen pixels (the capture and crop are in pixels).
            let to_screen = |r: Rect| Rect::from_min_max(monitor.min + r.min.to_vec2() * scale, monitor.min + r.max.to_vec2() * scale);
            self.point_ask_selected(chosen.map(to_screen));
        }
    }

    fn card(&mut self, ctx: &egui::Context, scale: f32, cleared: bool) {
        let Some(selection_px) = self.pointing.rect else { return };
        let selection = Rect::from_min_max((selection_px.min.to_vec2() / scale).to_pos2(), (selection_px.max.to_vec2() / scale).to_pos2());
        let areas = self.windows.work_areas();
        let work = crate::shell::work_area_at(&areas, selection.center()).unwrap_or(selection.expand(400.0));
        let state = &mut self.windows.select;
        let wanted = if state.card_height > 0.0 { state.card_height } else { 300.0 };
        let placed = card_frame(selection, wanted, work);
        // Grow and shrink with the answer, keeping the top edge where it is (and on screen).
        let frame = match state.card_at {
            Some(at) => {
                let top = at.y.min(work.max.y - placed.height() - MARGIN).max(work.min.y + MARGIN);
                Rect::from_min_size(pos2(at.x, top), placed.size())
            }
            None => placed,
        };
        state.card_at = Some(frame.min);
        if !state.card_adopted && !cleared {
            state.card_adopted = platform::adopt(CARD_TITLE, Role::Card);
        }
        let builder = ViewportBuilder::default()
            .with_icon(crate::ui::assets::window_icon())
            .with_title(CARD_TITLE)
            .with_inner_size(frame.size())
            .with_position(frame.min)
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false);
        let max_height = work.height() - 2.0 * MARGIN;
        let env = Env { now_ms: Self::now_ms(), reduce_motion: false, home: crate::settings::home().to_string_lossy().into_owned() };
        let (model, inputs, view, out) = (&self.model, &mut self.inputs, &mut self.view, &mut self.actions);
        let assets = self.assets.as_ref().expect("assets load before the first frame");
        let mut measured = None;
        ctx.show_viewport_immediate(ViewportId::from_hash_of("lantern-card"), builder, |ui, _| {
            keep(ui.ctx(), frame);
            if cleared {
                return; // a capture is waiting for Lantern's windows to clear
            }
            let mut cx = Ctx { model, inputs, view, out, assets, env: &env };
            let height =
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| ui::render_point_ask(ui, &mut cx, max_height)).inner;
            measured = Some(height);
        });
        if let Some(height) = measured {
            self.windows.select.card_height = height.min(max_height);
        }
    }
}

/// Keeps a window where and as big as `frame` says.
fn keep(ctx: &egui::Context, frame: Rect) {
    let (outer, inner) = ctx.input(|input| (input.viewport().outer_rect, input.viewport().inner_rect));
    if outer.is_none_or(|rect| (rect.min - frame.min).length() > 0.5) {
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(frame.min));
    }
    if inner.is_none_or(|rect| (rect.size() - frame.size()).length() > 0.5) {
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(frame.size()));
    }
}

/// The dimmed screen, the box being dragged with its size, and the hint (SelectionOverlay.swift).
fn draw_overlay(ui: &egui::Ui, area: Rect, selection: Option<Rect>, window_hit_test: bool) {
    let painter = ui.painter();
    match selection {
        None => {
            painter.rect_filled(area, 0.0, theme::black(0.28));
        }
        Some(sel) => {
            let dim = theme::black(0.42);
            for part in [
                Rect::from_min_max(area.min, pos2(area.max.x, sel.min.y)),
                Rect::from_min_max(pos2(area.min.x, sel.max.y), area.max),
                Rect::from_min_max(pos2(area.min.x, sel.min.y), pos2(sel.min.x, sel.max.y)),
                Rect::from_min_max(pos2(sel.max.x, sel.min.y), pos2(area.max.x, sel.max.y)),
            ] {
                painter.rect_filled(part, 0.0, dim);
            }
            painter.rect_stroke(sel, 0.0, Stroke::new(3.0, theme::black(0.35)), StrokeKind::Middle);
            painter.rect_stroke(sel, 0.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Middle);
            for corner in [sel.left_top(), sel.right_top(), sel.left_bottom(), sel.right_bottom()] {
                let handle = Rect::from_center_size(corner, Vec2::splat(8.0));
                painter.rect_filled(handle, 0.0, Color32::WHITE);
                painter.rect_stroke(handle, 0.0, Stroke::new(1.0, theme::black(0.4)), StrokeKind::Inside);
            }
            let scale = ui.ctx().pixels_per_point();
            let label = format!("{} × {}", (sel.width() * scale).round() as i32, (sel.height() * scale).round() as i32);
            let galley = painter.layout_no_wrap(label, theme::font(11.0, theme::Weight::Mono), Color32::WHITE);
            let size = galley.size() + vec2(12.0, 6.0);
            let badge = Rect::from_min_size(sel.right_bottom() - size - vec2(6.0, 6.0), size);
            painter.rect_filled(badge, 5.0, theme::black(0.75));
            painter.galley(Align2::CENTER_CENTER.align_size_within_rect(galley.size(), badge).min, galley, Color32::WHITE);
        }
    }
    let hint = if window_hit_test {
        "Drag to select an area · Click a window · Esc to cancel"
    } else {
        "Drag to select an area · Click for the whole screen · Esc to cancel"
    };
    let galley = painter.layout_no_wrap(hint.into(), theme::font(13.0, theme::Weight::Medium), Color32::WHITE);
    let size = galley.size() + vec2(32.0, 14.0);
    let bar = Rect::from_center_size(pos2(area.center().x, area.min.y + 90.0 + size.y / 2.0), size);
    painter.rect_filled(bar, size.y / 2.0, theme::black(0.78));
    painter.rect_stroke(bar, size.y / 2.0, Stroke::new(1.0, theme::white(0.18)), StrokeKind::Inside);
    painter.galley(Align2::CENTER_CENTER.align_size_within_rect(galley.size(), bar).min, galley, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work() -> Rect {
        Rect::from_min_size(pos2(0.0, 32.0), vec2(1920.0, 1048.0))
    }

    #[test]
    fn the_card_goes_right_of_the_selection_then_left_then_inside() {
        let middle = Rect::from_min_size(pos2(400.0, 300.0), vec2(300.0, 200.0));
        assert_eq!(card_frame(middle, 320.0, work()).min, pos2(714.0, 300.0));
        let right = Rect::from_min_size(pos2(1500.0, 300.0), vec2(300.0, 200.0));
        assert_eq!(card_frame(right, 320.0, work()).min.x, 1500.0 - 400.0 - 14.0);
        let wide = Rect::from_min_size(pos2(10.0, 300.0), vec2(1900.0, 200.0));
        assert_eq!(card_frame(wide, 320.0, work()).min.x, 1910.0 - 400.0 - 14.0, "inside its right edge");
    }

    #[test]
    fn the_card_stays_on_screen_with_a_minimum_and_maximum_height() {
        let low = Rect::from_min_size(pos2(400.0, 1000.0), vec2(100.0, 50.0));
        let card = card_frame(low, 320.0, work());
        assert_eq!(card.max.y, 1080.0 - 8.0);
        assert_eq!(card_frame(low, 20.0, work()).height(), 160.0);
        assert_eq!(card_frame(low, 5000.0, work()).height(), 1048.0 - 16.0);
    }
}
