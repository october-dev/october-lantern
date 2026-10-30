//! The pill and the panel beside it (Swift `Windows.swift`), adapted per CONTRACT §6.
//! Geometry is in logical points, origin top-left, y down; the Mac's AppKit maths is y up, so
//! "top fraction measured from the bottom" is converted here once.

pub mod pill;

use crate::model::Edge;
use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

/// The pill's distance from the screen edge.
pub const EDGE_GAP: f32 = 10.0;
/// Windows stay this far inside the work area.
pub const SCREEN_MARGIN: f32 = 8.0;
/// Between the pill and the panel.
pub const PANEL_GAP: f32 = 10.0;
/// Pill sizes: collapsed (just the lantern) and expanded (lantern + 5 actions).
pub const PILL_COLLAPSED: Vec2 = vec2(50.0, 50.0);
pub const PILL_EXPANDED: Vec2 = vec2(50.0, 251.0);
/// Collapse this long after the pointer leaves.
pub const COLLAPSE_AFTER: f64 = 0.7;
/// After Welcome, the pill stays expanded this long so people can find it.
pub const FLASH_FOR: f64 = 3.0;

/// The tallest a panel may be in this work area.
pub fn panel_limit(work: Rect) -> f32 {
    work.height() - 2.0 * SCREEN_MARGIN
}

/// Where the pill goes: `EDGE_GAP` from the edge, its top at `top_fraction` of the work area's
/// height measured from the bottom (growing downward from there), kept inside the work area.
pub fn pill_frame(work: Rect, size: Vec2, edge: Edge, top_fraction: f32) -> Rect {
    let x = match edge {
        Edge::Right => work.max.x - size.x - EDGE_GAP,
        Edge::Left => work.min.x + EDGE_GAP,
    };
    let top = work.max.y - work.height() * top_fraction;
    Rect::from_min_size(pos2(x, clamp_top(top, size.y, work)), size)
}

/// Beside the pill (toward the screen's middle), tops aligned, as tall as its content allows.
pub fn panel_frame(pill: Rect, content: Vec2, edge: Edge, work: Rect) -> Rect {
    let size = vec2(content.x, content.y.min(panel_limit(work)));
    let x = match edge {
        Edge::Right => pill.min.x - size.x - PANEL_GAP,
        Edge::Left => pill.max.x + PANEL_GAP,
    };
    Rect::from_min_size(pos2(x, clamp_top(pill.min.y, size.y, work)), size)
}

fn clamp_top(top: f32, height: f32, work: Rect) -> f32 {
    top.min(work.max.y - height - SCREEN_MARGIN).max(work.min.y + SCREEN_MARGIN)
}

/// After a drag: the nearer edge, and the pill's top as a fraction measured from the bottom.
pub fn snap(pill: Rect, work: Rect) -> (Edge, f32) {
    let edge = if pill.center().x < work.center().x { Edge::Left } else { Edge::Right };
    let fraction = ((work.max.y - pill.min.y) / work.height()).clamp(0.1, 1.0);
    (edge, fraction)
}

/// The work area (of several) that holds a point, else the first.
pub fn work_area_at(areas: &[Rect], point: Pos2) -> Option<Rect> {
    areas.iter().copied().find(|area| area.contains(point)).or_else(|| areas.first().copied())
}

/// Expand while the pointer is over the pill; tuck back in `COLLAPSE_AFTER` seconds after it leaves
/// (Swift `checkHover`/`flash`). Times are seconds on any monotonic clock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hover {
    pub expanded: bool,
    outside_since: Option<f64>,
}

impl Hover {
    /// One hover check. `inside`: the pointer is over the pill (with its halo where the platform has one).
    pub fn update(&mut self, inside: bool, now: f64) {
        if inside {
            self.outside_since = None;
            self.expanded = true;
        } else if self.expanded {
            let since = *self.outside_since.get_or_insert(now);
            if now - since > COLLAPSE_AFTER {
                self.expanded = false;
                self.outside_since = None;
            }
        }
    }
    /// Expands for a few seconds so people can find it.
    pub fn flash(&mut self, now: f64) {
        self.expanded = true;
        self.outside_since = Some(now + FLASH_FOR);
    }
}

/// The pill shows its actions while hovered, while a panel is open, or while dictating.
pub fn pill_expanded(hover: &Hover, panel_open: bool, dictating: bool) -> bool {
    hover.expanded || panel_open || dictating
}

/// Whether losing focus closes the panel (CONTRACT §6): never while you're writing, dictating,
/// or in a dialog Lantern opened (folder picker, screenshot permission), or when focus went to
/// another of Lantern's own windows.
pub fn closes_on_focus_loss(draft_empty: bool, dictating: bool, native_dialog: bool, to_own_window: bool) -> bool {
    draft_empty && !dictating && !native_dialog && !to_own_window
}

#[cfg(test)]
mod tests {
    use super::*;

    // GNOME on this machine: 1920x1080 with a 32pt top bar (Phase 0).
    fn work() -> Rect {
        Rect::from_min_size(pos2(0.0, 32.0), vec2(1920.0, 1048.0))
    }

    #[test]
    fn pill_sits_at_the_right_edge_at_72_percent_from_the_bottom() {
        let pill = pill_frame(work(), PILL_EXPANDED, Edge::Right, 0.72);
        assert_eq!(pill.min.x, 1860.0);
        assert!((pill.min.y - (32.0 + 1048.0 * 0.28)).abs() < 0.001);
        assert_eq!(pill.size(), PILL_EXPANDED);
        let left = pill_frame(work(), PILL_EXPANDED, Edge::Left, 0.72);
        assert_eq!(left.min.x, 10.0);
    }

    #[test]
    fn pill_grows_downward_from_a_fixed_top() {
        let collapsed = pill_frame(work(), PILL_COLLAPSED, Edge::Right, 0.72);
        let expanded = pill_frame(work(), PILL_EXPANDED, Edge::Right, 0.72);
        assert_eq!(collapsed.min.y, expanded.min.y);
    }

    #[test]
    fn pill_stays_inside_the_work_area() {
        let low = pill_frame(work(), PILL_EXPANDED, Edge::Right, 0.1);
        assert_eq!(low.max.y, work().max.y - SCREEN_MARGIN);
        let high = pill_frame(work(), PILL_EXPANDED, Edge::Right, 1.0);
        assert_eq!(high.min.y, work().min.y + SCREEN_MARGIN);
    }

    #[test]
    fn panel_sits_beside_the_pill_with_tops_aligned_and_a_height_limit() {
        let pill = pill_frame(work(), PILL_EXPANDED, Edge::Right, 0.72);
        let panel = panel_frame(pill, vec2(380.0, 520.0), Edge::Right, work());
        assert_eq!(panel.min, pos2(1860.0 - 10.0 - 380.0, pill.min.y));
        let tall = panel_frame(pill, vec2(380.0, 5000.0), Edge::Right, work());
        assert_eq!(tall.height(), 1048.0 - 16.0);
        assert_eq!(tall.min.y, work().min.y + SCREEN_MARGIN);
        let left_pill = pill_frame(work(), PILL_EXPANDED, Edge::Left, 0.72);
        assert_eq!(panel_frame(left_pill, vec2(380.0, 520.0), Edge::Left, work()).min.x, 10.0 + 50.0 + 10.0);
    }

    #[test]
    fn a_panel_too_tall_for_the_space_below_moves_up() {
        let pill = pill_frame(work(), PILL_EXPANDED, Edge::Right, 0.2);
        let panel = panel_frame(pill, vec2(380.0, 700.0), Edge::Right, work());
        assert_eq!(panel.max.y, work().max.y - SCREEN_MARGIN);
    }

    #[test]
    fn snap_picks_the_nearer_edge_and_keeps_the_height() {
        let dropped = Rect::from_min_size(pos2(300.0, 500.0), PILL_EXPANDED);
        let (edge, fraction) = snap(dropped, work());
        assert_eq!(edge, Edge::Left);
        let again = pill_frame(work(), PILL_EXPANDED, edge, fraction);
        assert!((again.min.y - 500.0).abs() < 0.01);
        assert_eq!(snap(Rect::from_min_size(pos2(1500.0, 32.0), PILL_EXPANDED), work()), (Edge::Right, 1.0));
        assert_eq!(snap(Rect::from_min_size(pos2(1500.0, 1070.0), PILL_EXPANDED), work()).1, 0.1);
    }

    #[test]
    fn hover_collapses_only_after_the_grace_period() {
        let mut hover = Hover::default();
        hover.update(true, 0.0);
        assert!(hover.expanded);
        hover.update(false, 1.0);
        hover.update(false, 1.6);
        assert!(hover.expanded);
        hover.update(false, 1.71);
        assert!(!hover.expanded);
        hover.update(false, 1.0);
        hover.update(true, 1.5);
        hover.update(false, 2.0);
        assert!(hover.expanded, "coming back resets the grace period");
    }

    #[test]
    fn flash_holds_the_pill_open_for_three_seconds() {
        let mut hover = Hover::default();
        hover.flash(10.0);
        hover.update(false, 13.5);
        assert!(hover.expanded);
        hover.update(false, 13.71);
        assert!(!hover.expanded);
    }

    #[test]
    fn panel_and_dictation_keep_the_pill_expanded() {
        let hover = Hover::default();
        assert!(!pill_expanded(&hover, false, false));
        assert!(pill_expanded(&hover, true, false));
        assert!(pill_expanded(&hover, false, true));
    }

    #[test]
    fn focus_loss_closes_only_an_idle_panel() {
        assert!(closes_on_focus_loss(true, false, false, false));
        assert!(!closes_on_focus_loss(false, false, false, false), "a draft keeps it open");
        assert!(!closes_on_focus_loss(true, true, false, false), "dictation keeps it open");
        assert!(!closes_on_focus_loss(true, false, true, false), "Lantern's own dialog keeps it open");
        assert!(!closes_on_focus_loss(true, false, false, true), "moving to Lantern's own window keeps it open");
    }

    #[test]
    fn work_area_follows_the_point() {
        let second = Rect::from_min_size(pos2(1920.0, 0.0), vec2(2560.0, 1440.0));
        assert_eq!(work_area_at(&[work(), second], pos2(2000.0, 10.0)), Some(second));
        assert_eq!(work_area_at(&[work(), second], pos2(-50.0, 10.0)), Some(work()));
    }
}
