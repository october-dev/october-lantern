//! Show me's coordinates and what it tells October about the screen.
//!
//! Three spaces meet here, and mixing them points at the wrong control:
//! - **Screen**: Cua Driver's global space. On macOS that's points with the origin at the top
//!   left of the main display (measured: accessibility frames, window bounds and the agent cursor
//!   all use it). Windows and Linux are qualified separately.
//! - **Image**: pixels of the picture the app sends October, after it scaled the capture.
//! - **Display**: the screen rectangle that picture shows.
//!
//! A capture records the display rectangle and the image size, which is everything needed to
//! convert in both directions. Element frames go screen → image before they're sent; October's
//! answer comes back in image pixels and goes image → screen before the cursor moves.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    fn finite(&self) -> bool {
        [self.x, self.y, self.w, self.h].iter().all(|v| v.is_finite())
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn contains(&self, (px, py): (f64, f64)) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.w).min(other.x + other.w);
        let bottom = (self.y + self.h).min(other.y + other.h);
        (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
    }

    fn area(&self) -> f64 {
        self.w * self.h
    }
}

/// Smallest control Show me points at, in screen units: anything thinner is usually a separator or
/// a frame the app reports wrong.
const MIN_SIDE: f64 = 4.0;

/// One capture: the display it shows (screen units) and the size of the image sent (pixels).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Capture {
    pub display: Rect,
    pub image_w: u32,
    pub image_h: u32,
}

impl Capture {
    /// A capture October can use: a real display and a real image.
    pub fn valid(self) -> bool {
        self.display.finite() && self.display.w >= 1.0 && self.display.h >= 1.0 && self.image_w >= 1 && self.image_h >= 1
    }

    fn scale(self) -> (f64, f64) {
        (self.image_w as f64 / self.display.w, self.image_h as f64 / self.display.h)
    }

    /// A screen rectangle in image pixels, clipped to the display; `None` when too little of it
    /// shows (under half, or smaller than a control can be).
    pub fn to_image(self, r: Rect) -> Option<Rect> {
        if !r.finite() || r.w < MIN_SIDE || r.h < MIN_SIDE {
            return None;
        }
        let shown = r.intersection(&self.display)?;
        if shown.area() < r.area() / 2.0 || shown.w < MIN_SIDE || shown.h < MIN_SIDE {
            return None;
        }
        let (sx, sy) = self.scale();
        Some(Rect::new((shown.x - self.display.x) * sx, (shown.y - self.display.y) * sy, shown.w * sx, shown.h * sy))
    }

    /// An image rectangle (October's answer) on the screen. `None` unless it is a real box inside the
    /// image: an answer outside the picture isn't clamped into a plausible-looking target.
    pub fn to_screen(self, r: Rect) -> Option<Rect> {
        let (w, h) = (self.image_w as f64, self.image_h as f64);
        // Half a pixel of rounding at the edges is fine; more is a bad answer.
        let inside = r.x >= -0.5 && r.y >= -0.5 && r.x + r.w <= w + 0.5 && r.y + r.h <= h + 0.5;
        if !r.finite() || r.w < 1.0 || r.h < 1.0 || !inside {
            return None;
        }
        let (sx, sy) = self.scale();
        Some(Rect::new(self.display.x + r.x / sx, self.display.y + r.y / sy, r.w / sx, r.h / sy))
    }
}

/// A window the driver reported, front to back by `z` (higher is in front).
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub pid: i64,
    pub window_id: i64,
    pub app: String,
    pub title: String,
    pub bounds: Rect,
    pub z: i64,
}

/// An element of a window's accessibility tree, as the driver reported it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    pub role: String,
    pub label: Option<String>,
    pub value: Option<String>,
    pub frame: Option<Rect>,
}

/// What kind of control an element is, the same words on every system; `None` for elements Show
/// me never names to October (text, fields, containers, anything secure).
pub fn control_kind(role: &str) -> Option<&'static str> {
    let r = role.trim_start_matches("AX").to_ascii_lowercase().replace([' ', '_', '-'], "");
    Some(match r.as_str() {
        "button" | "pushbutton" => "button",
        "menubutton" | "popupbutton" | "combobox" | "splitbutton" => "popup",
        "menuitem" | "menubaritem" | "checkmenuitem" | "radiomenuitem" => "menu item",
        "checkbox" | "togglebutton" | "switch" => "checkbox",
        "radiobutton" => "radio",
        "tab" | "tabitem" | "pagetab" => "tab",
        "link" | "hyperlink" => "link",
        "disclosuretriangle" => "disclosure",
        "slider" => "slider",
        "incrementor" | "spinbutton" | "spinner" => "stepper",
        _ => return None,
    })
}

/// Longest label sent, in UTF-16 units (as October counts).
pub const LABEL_LIMIT: usize = 96;
/// Most controls sent with one capture.
pub const CONTROL_LIMIT: usize = 250;

/// A control October may point at: its number in this capture, its kind, its name when the name
/// can be sent, and where it is in the image. The screen frame and owning window stay local.
#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    pub id: u32,
    pub kind: &'static str,
    pub label: Option<String>,
    pub image: Rect,
    pub screen: Rect,
    pub window_id: i64,
}

/// A control's name, when it is safe to send: never a value (the driver falls back to an element's
/// value when it has no title, so a name equal to the value may be what someone typed), never
/// empty, and cut to `LABEL_LIMIT`.
fn sendable_label(e: &Element) -> Option<String> {
    let label = e.label.as_deref()?.trim();
    if label.is_empty() || e.value.as_deref().is_some_and(|v| v.trim() == label) {
        return None;
    }
    let mut out = String::new();
    let mut used = 0;
    for c in label.chars() {
        if c.is_control() {
            continue;
        }
        used += c.len_utf16();
        if used > LABEL_LIMIT {
            break;
        }
        out.push(c);
    }
    (!out.is_empty()).then_some(out)
}

/// The controls of `windows` (each with its elements) that are visible in `capture`: on the
/// display, in a window's bounds, and not under a window in front. Front windows first, at most
/// `CONTROL_LIMIT`.
pub fn project(capture: &Capture, windows: &[(Window, Vec<Element>)]) -> Vec<Control> {
    let mut order: Vec<&(Window, Vec<Element>)> = windows.iter().collect();
    order.sort_by_key(|w| std::cmp::Reverse(w.0.z));
    let mut out = Vec::new();
    for (i, (window, elements)) in order.iter().enumerate() {
        let in_front: Vec<Rect> = order[..i].iter().map(|(w, _)| w.bounds).collect();
        for e in elements {
            if out.len() >= CONTROL_LIMIT {
                return out;
            }
            let (Some(kind), Some(frame)) = (control_kind(&e.role), e.frame) else { continue };
            let center = frame.center();
            if !window.bounds.contains(center) || in_front.iter().any(|b| b.contains(center)) {
                continue;
            }
            let Some(image) = capture.to_image(frame) else { continue };
            out.push(Control {
                id: out.len() as u32 + 1,
                kind,
                label: sendable_label(e),
                image,
                screen: frame,
                window_id: window.window_id,
            });
        }
    }
    out
}

/// What October answered for one step, already parsed.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// One control by its number in the capture, or a box in image pixels; never both.
    Control(u32),
    Box(Rect),
}

/// Where to point: the screen rectangle of the answer, or why there's nowhere to point.
pub fn resolve(capture: &Capture, controls: &[Control], answer: &Answer) -> Result<Rect, &'static str> {
    match answer {
        Answer::Control(id) => controls.iter().find(|c| c.id == *id).map(|c| c.screen).ok_or("unknown control"),
        Answer::Box(r) => capture.to_screen(*r).ok_or("box outside the picture"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window at (400, 200) on a display whose top left is (0, 0), sent at half size.
    fn capture() -> Capture {
        Capture { display: Rect::new(0.0, 0.0, 1600.0, 1000.0), image_w: 800, image_h: 500 }
    }

    fn window(id: i64, z: i64, bounds: Rect) -> Window {
        Window { pid: 1, window_id: id, app: "App".into(), title: "T".into(), bounds, z }
    }

    fn button(label: &str, frame: Rect) -> Element {
        Element { role: "AXButton".into(), label: Some(label.into()), value: None, frame: Some(frame) }
    }

    #[test]
    fn screen_frames_become_image_pixels_and_back() {
        let c = capture();
        // A control at (500, 300) is (250, 150) in the half-size image, not (500, 300).
        let image = c.to_image(Rect::new(500.0, 300.0, 40.0, 20.0)).unwrap();
        assert_eq!(image, Rect::new(250.0, 150.0, 20.0, 10.0));
        assert_eq!(c.to_screen(image).unwrap(), Rect::new(500.0, 300.0, 40.0, 20.0));
    }

    #[test]
    fn a_display_left_of_or_above_the_main_one_converts_too() {
        // A second display above and to the left: negative origin, 1:1 pixels.
        let c = Capture { display: Rect::new(-240.0, -1080.0, 1920.0, 1080.0), image_w: 1920, image_h: 1080 };
        let image = c.to_image(Rect::new(874.0, -696.0, 57.0, 26.0)).unwrap();
        assert_eq!(image, Rect::new(1114.0, 384.0, 57.0, 26.0));
        assert_eq!(c.to_screen(image).unwrap(), Rect::new(874.0, -696.0, 57.0, 26.0));
    }

    #[test]
    fn rectangles_are_scaled_on_each_axis_separately() {
        // Rounding made the image 799 x 500: x and y scale differently.
        let c = Capture { display: Rect::new(0.0, 0.0, 1600.0, 1000.0), image_w: 799, image_h: 500 };
        let r = c.to_image(Rect::new(1600.0 - 40.0, 0.0, 40.0, 20.0)).unwrap();
        assert!((r.x + r.w - 799.0).abs() < 1e-9);
        assert!((r.h - 10.0).abs() < 1e-9);
    }

    #[test]
    fn controls_off_the_display_thin_or_mostly_hidden_are_left_out() {
        let c = capture();
        assert!(c.to_image(Rect::new(1700.0, 10.0, 40.0, 20.0)).is_none(), "on another display");
        assert!(c.to_image(Rect::new(10.0, 10.0, 40.0, 1.0)).is_none(), "a separator");
        assert!(c.to_image(Rect::new(1590.0, 10.0, 40.0, 20.0)).is_none(), "three quarters off screen");
        assert!(c.to_image(Rect::new(f64::NAN, 10.0, 40.0, 20.0)).is_none());
    }

    #[test]
    fn answers_outside_the_picture_are_refused_not_clamped() {
        let c = capture();
        assert!(c.to_screen(Rect::new(790.0, 10.0, 40.0, 20.0)).is_none());
        assert!(c.to_screen(Rect::new(-20.0, 10.0, 10.0, 10.0)).is_none());
        assert!(c.to_screen(Rect::new(10.0, 10.0, 0.0, 10.0)).is_none());
        assert!(c.to_screen(Rect::new(10.0, f64::INFINITY, 10.0, 10.0)).is_none());
        assert!(c.to_screen(Rect::new(0.0, 0.0, 800.4, 500.0)).is_some(), "half a pixel of rounding");
    }

    #[test]
    fn controls_under_a_window_in_front_are_not_offered() {
        let c = capture();
        let back = window(1, 1, Rect::new(0.0, 0.0, 1000.0, 800.0));
        let front = window(2, 2, Rect::new(300.0, 200.0, 400.0, 300.0));
        let windows = vec![
            (back, vec![button("Hidden", Rect::new(400.0, 300.0, 40.0, 20.0)), button("Visible", Rect::new(50.0, 50.0, 40.0, 20.0))]),
            (front, vec![button("Front", Rect::new(320.0, 220.0, 40.0, 20.0))]),
        ];
        let controls = project(&c, &windows);
        let names: Vec<_> = controls.iter().map(|c| c.label.clone().unwrap()).collect();
        assert_eq!(names, ["Front", "Visible"], "front window first; the covered button is left out");
        assert_eq!(controls[0].id, 1);
        assert_eq!(controls[1].window_id, 1);
    }

    #[test]
    fn what_someone_typed_is_never_sent_as_a_name() {
        let c = capture();
        let typed = Element {
            role: "AXButton".into(),
            label: Some("hunter2".into()),
            value: Some("hunter2".into()),
            frame: Some(Rect::new(10.0, 10.0, 40.0, 20.0)),
        };
        let field = Element {
            role: "AXTextField".into(),
            label: Some("Password".into()),
            value: None,
            frame: Some(Rect::new(10.0, 60.0, 40.0, 20.0)),
        };
        let secure = Element { role: "AXSecureTextField".into(), ..field.clone() };
        let controls = project(&c, &[(window(1, 1, Rect::new(0.0, 0.0, 800.0, 600.0)), vec![typed, field, secure])]);
        assert_eq!(controls.len(), 1, "fields are never offered");
        assert_eq!(controls[0].label, None, "a name equal to the value is dropped, the control stays");
    }

    #[test]
    fn roles_read_the_same_on_every_system() {
        assert_eq!(control_kind("AXButton"), Some("button"));
        assert_eq!(control_kind("Button"), Some("button"));
        assert_eq!(control_kind("push button"), Some("button"));
        assert_eq!(control_kind("TabItem"), Some("tab"));
        assert_eq!(control_kind("page tab"), Some("tab"));
        assert_eq!(control_kind("AXMenuBarItem"), Some("menu item"));
        assert_eq!(control_kind("Hyperlink"), Some("link"));
        assert_eq!(control_kind("AXTextField"), None);
        assert_eq!(control_kind("Edit"), None);
        assert_eq!(control_kind("AXGroup"), None);
    }

    #[test]
    fn long_names_are_cut_and_control_characters_dropped() {
        let e = Element { role: "AXButton".into(), label: Some(format!("a\u{7}{}", "é".repeat(200))), value: None, frame: None };
        let label = sendable_label(&e).unwrap();
        assert!(label.starts_with('a') && !label.contains('\u{7}'));
        assert_eq!(label.encode_utf16().count(), LABEL_LIMIT);
    }

    #[test]
    fn an_answer_names_a_known_control_or_a_box_in_the_picture() {
        let c = capture();
        let controls =
            project(&c, &[(window(1, 1, Rect::new(0.0, 0.0, 800.0, 600.0)), vec![button("Gear", Rect::new(500.0, 300.0, 40.0, 20.0))])]);
        assert_eq!(resolve(&c, &controls, &Answer::Control(1)), Ok(Rect::new(500.0, 300.0, 40.0, 20.0)));
        assert_eq!(resolve(&c, &controls, &Answer::Control(2)), Err("unknown control"));
        assert_eq!(resolve(&c, &controls, &Answer::Box(Rect::new(250.0, 150.0, 20.0, 10.0))), Ok(Rect::new(500.0, 300.0, 40.0, 20.0)));
    }
}
