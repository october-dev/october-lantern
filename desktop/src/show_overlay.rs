//! Show me geometry in physical desktop pixels. One captured display, one transform; the
//! renderer converts to its own egui points only at paint time (never the pill's DPI).
use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    pub bounds: Rect,
    /// Physical pixels per OS point on this display, independent of Lantern's UI zoom.
    pub scale: f32,
}

impl Display {
    pub fn valid(self) -> bool {
        self.bounds.is_finite() && self.bounds.is_positive() && self.scale.is_finite() && (0.5..=8.0).contains(&self.scale)
    }

    pub fn target(self, image: [u32; 2], target: [f32; 4]) -> Option<Rect> {
        if !self.valid() || image.contains(&0) || !target.iter().all(|v| v.is_finite()) || target[2] <= 0.0 || target[3] <= 0.0 {
            return None;
        }
        let r = Rect::from_min_size(pos2(target[0], target[1]), vec2(target[2], target[3]));
        let picture = Rect::from_min_size(Pos2::ZERO, vec2(image[0] as f32, image[1] as f32));
        if !picture.expand(0.5).contains_rect(r) {
            return None;
        }
        let scale = self.bounds.size() / picture.size();
        Some(Rect::from_min_max(self.bounds.min + r.min.to_vec2() * scale, self.bounds.min + r.max.to_vec2() * scale))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub id: u64,
    pub bounds: Rect,
}

/// No titles, executable names or field contents: only the geometry the cue depends on.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub display: Display,
    pub foreground: Option<u64>,
    /// Front to back, excluding Lantern's windows.
    pub windows: Vec<Window>,
}

impl Screen {
    pub fn changed(&self, now: &Self) -> bool {
        self.display != now.display
            || self.windows != now.windows
            // Focusing Lantern's own card isn't a change to the observed app.
            || self.foreground.zip(now.foreground).is_some_and(|(a, b)| a != b)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub ring: Rect,
    pub tip: Pos2,
    pub tail: Pos2,
    pub label: Rect,
}

fn centered(bounds: Rect, center: Pos2, size: Vec2) -> Rect {
    let size = size.min(bounds.size());
    let min = (center - size / 2.0).clamp(bounds.min, bounds.max - size);
    Rect::from_min_size(min, size)
}

/// The entire pulsing ring stays inside the display, including very large targets. Arrow and
/// label approach from the screen-centre side, as on Mac; near an edge the arrow is shortened.
pub fn layout(display: Display, target: Rect, pulse: f32) -> Layout {
    let s = display.scale;
    let safe = display.bounds.shrink((9.0 * s).min(display.bounds.width().min(display.bounds.height()) / 8.0));
    let max_pulse = 1.08;
    let size = (target.size() + Vec2::splat(16.0 * s)).max(Vec2::splat(44.0 * s)).min(safe.size() / max_pulse);
    let envelope = centered(safe, target.center(), size * max_pulse);
    let ring = Rect::from_center_size(envelope.center(), size * (1.0 + 0.08 * pulse.clamp(0.0, 1.0)));
    let delta = display.bounds.center() - ring.center();
    let direction = if delta.length() < 40.0 * s { vec2(0.6, 0.8) } else { delta.normalized() };
    let reach = ring.width().max(ring.height()) / 2.0 + (14.0 + 10.0 * pulse) * s;
    let arrow_safe = display.bounds.shrink(30.0 * s);
    let tip = (ring.center() + direction * reach).clamp(arrow_safe.min, arrow_safe.max);
    let tail = (tip + direction * 170.0 * s).clamp(arrow_safe.min, arrow_safe.max);
    let label = centered(safe, tail + direction * vec2(40.0, 26.0) * s, vec2(220.0, 36.0) * s);
    Layout { ring, tip, tail, label }
}

pub fn hit(display: Display, target: Rect, point: Pos2) -> bool {
    point.is_finite() && layout(display, target, 0.0).ring.expand(16.0 * display.scale).contains(point)
}

pub fn paint(ui: &eframe::egui::Ui, display: Display, target: Rect, step: u64, pulse: f32) {
    use eframe::egui::{Align2, Color32, FontId, Shape, Stroke, StrokeKind};
    let physical = layout(display, target, pulse);
    let ppp = ui.ctx().pixels_per_point();
    let point = |p: Pos2| ui.max_rect().min + (p - display.bounds.min) / ppp;
    let rect = |r: Rect| Rect::from_min_max(point(r.min), point(r.max));
    let s = display.scale / ppp;
    let orange = Color32::from_rgb(255, 149, 0);
    let painter = ui.painter();
    let ring = rect(physical.ring);
    for width in [28.0, 20.0, 12.0] {
        painter.rect_stroke(ring, 12.0 * s, Stroke::new(width * s, Color32::from_rgba_unmultiplied(255, 149, 0, 13)), StrokeKind::Middle);
    }
    painter.rect_stroke(ring, 12.0 * s, Stroke::new(10.0 * s, Color32::from_black_alpha(140)), StrokeKind::Middle);
    painter.rect_stroke(ring, 12.0 * s, Stroke::new(6.0 * s, orange), StrokeKind::Middle);
    let (tail, tip) = (point(physical.tail), point(physical.tip));
    let direction = (tail - tip).normalized();
    let normal = vec2(-direction.y, direction.x);
    let head = 28.0 * s;
    for (width, color) in [(13.0, Color32::from_black_alpha(140)), (8.0, orange)] {
        let stroke = Stroke::new(width * s, color);
        painter.line_segment([tail, tip], stroke);
        painter.add(Shape::line(
            vec![tip + direction * head * 0.866 + normal * head * 0.5, tip, tip + direction * head * 0.866 - normal * head * 0.5],
            stroke,
        ));
    }
    let label = rect(physical.label);
    painter.rect_filled(label, label.height() / 2.0, orange);
    painter.rect_stroke(label, label.height() / 2.0, Stroke::new(s, Color32::from_black_alpha(90)), StrokeKind::Inside);
    painter.text(label.center(), Align2::CENTER_CENTER, format!("Lantern · Step {step}"), FontId::proportional(17.0 * s), Color32::BLACK);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display() -> Display {
        Display { bounds: Rect::from_min_size(pos2(-2560.0, -1440.0), vec2(2560.0, 1440.0)), scale: 2.0 }
    }

    #[test]
    fn image_coordinates_use_the_captured_monitors_origin_and_each_axis() {
        let d = display();
        let r = d.target([1280, 719], [100.0, 100.0, 40.0, 20.0]).unwrap();
        assert_eq!(r.min.x, -2360.0);
        assert!((r.min.y - (-1440.0 + 100.0 * 1440.0 / 719.0)).abs() < 0.01);
        assert_eq!(r.width(), 80.0);
        assert!(d.target([1280, 719], [1280.0, 0.0, 40.0, 20.0]).is_none());
        assert!(d.target([0, 719], [0.0, 0.0, 40.0, 20.0]).is_none());
        assert!(d.target([1280, 719], [f32::NAN, 0.0, 40.0, 20.0]).is_none());
    }

    #[test]
    fn small_corner_and_display_sized_targets_keep_the_whole_pulse_and_label_on_screen() {
        let d = display();
        for at in [d.bounds.min, d.bounds.max, d.bounds.center()] {
            for size in [vec2(1.0, 1.0), vec2(400.0, 50.0), d.bounds.size()] {
                for pulse in [0.0, 0.5, 1.0] {
                    let l = layout(d, Rect::from_min_size(at, size), pulse);
                    assert!(d.bounds.contains_rect(l.ring));
                    assert!(d.bounds.contains_rect(l.label));
                    assert!(d.bounds.contains(l.tip) && d.bounds.contains(l.tail));
                    assert!(l.ring.width() >= 44.0 * d.scale && l.ring.height() >= 44.0 * d.scale);
                }
            }
        }
    }

    #[test]
    fn click_tolerance_is_in_monitor_points_and_tests_the_visible_ring() {
        let d = display();
        let target = Rect::from_center_size(d.bounds.center(), Vec2::splat(4.0));
        let ring = layout(d, target, 0.0).ring;
        assert!(hit(d, target, ring.center()));
        assert!(hit(d, target, ring.right_center() + vec2(31.0, 0.0)));
        assert!(!hit(d, target, ring.right_center() + vec2(33.0, 0.0)));
        assert!(!hit(d, target, pos2(f32::NAN, 0.0)));
    }

    #[test]
    fn change_detection_covers_focus_stacking_geometry_membership_and_dpi() {
        let first = Screen {
            display: display(),
            foreground: Some(1),
            windows: vec![Window { id: 1, bounds: display().bounds }, Window { id: 2, bounds: display().bounds }],
        };
        assert!(!first.changed(&first));
        let mut now = first.clone();
        now.foreground = None;
        assert!(!first.changed(&now), "Lantern's card taking focus is excluded");
        now.foreground = Some(2);
        assert!(first.changed(&now));
        now = first.clone();
        now.windows.reverse();
        assert!(first.changed(&now));
        now = first.clone();
        now.windows[0].bounds.min.x += 1.0;
        assert!(first.changed(&now));
        now = first.clone();
        now.windows.pop();
        assert!(first.changed(&now));
        now = first.clone();
        now.windows.push(Window { id: 3, bounds: display().bounds });
        assert!(first.changed(&now));
        now = first.clone();
        now.display.scale = 1.5;
        assert!(first.changed(&now));
    }
}
