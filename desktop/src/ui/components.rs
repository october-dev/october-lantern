//! Shared primitives with the Swift geometry, colors and interaction states.
use super::{
    assets::Assets,
    icons::Icon,
    theme::{self, Weight},
};
use crate::model::{Agent, AgentKind};
use eframe::egui::{self, Color32, Id, Label, Painter, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, Ui, Vec2};

pub use super::icons::paint_icon;

pub fn text(text: impl Into<String>, size: f32, weight: Weight, color: Color32) -> RichText {
    RichText::new(text).font(theme::font(size, weight)).color(color)
}

pub fn label(ui: &mut Ui, value: impl Into<String>, size: f32, weight: Weight, color: Color32) -> Response {
    ui.add(Label::new(text(value, size, weight, color)).wrap())
}

/// egui's consume_key(NONE, …) also matches Shift/Alt, which would submit a multiline
/// field on Shift-Return. Consume only genuinely unmodified presses for native plain Return.
pub fn consume_plain_key(ui: &mut Ui, key: egui::Key) -> bool {
    ui.input_mut(|input| {
        let mut pressed = false;
        input.events.retain(|event| {
            let matches = matches!(event, egui::Event::Key { key: event_key, modifiers, pressed: true, .. } if *event_key == key && *modifiers == egui::Modifiers::NONE);
            pressed |= matches;
            !matches
        });
        pressed
    })
}

/// Small native-style activity indicator. The shell supplies time/repaints; fixtures freeze it.
pub fn spinner(ui: &mut Ui, size: f32, color: Color32, env: &super::Env) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let phase = if env.reduce_motion { 0 } else { (env.now_ms.max(0.0) / 80.0) as usize % 12 };
    for i in 0..12 {
        let angle = i as f32 * std::f32::consts::TAU / 12.0;
        let direction = egui::vec2(angle.cos(), angle.sin());
        let alpha = 0.2 + 0.8 * ((i + 12 - phase) % 12) as f32 / 11.0;
        ui.painter().line_segment(
            [rect.center() + direction * size * 0.28, rect.center() + direction * size * 0.47],
            Stroke::new((size / 10.0).max(1.0), color.gamma_multiply(alpha)),
        );
    }
    response
}

/// Plain button with no egui chrome. `size` is the hit box, the glyph sits inside it.
pub fn icon_button(ui: &mut Ui, id: impl egui::AsIdSalt, icon: Icon, size: f32, tint: Color32, active: bool) -> Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let response = ui.interact(rect, ui.make_persistent_id(id), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("{icon:?}")));
    if active || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, if size >= 32.0 { size / 2.0 } else { 9.0 }, theme::white(0.12));
    }
    let glyph = Rect::from_center_size(rect.center(), Vec2::splat(if size <= 22.0 { size * 0.5 } else { 14.0 }));
    paint_icon(ui.painter(), glyph, icon, tint);
    response
}

/// Swift `SmallButton`: 11 medium, icon/text gap, horizontal 8 / vertical 4, hover capsule.
pub fn small_button(ui: &mut Ui, title: &str, icon: Icon) -> Response {
    let galley = ui.painter().layout_no_wrap(title.into(), theme::font(11.0, Weight::Medium), theme::MUTED);
    let size = egui::vec2(16.0 + 11.0 + 4.0 + galley.size().x, galley.size().y + 8.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title));
    let color = if response.hovered() || response.has_focus() { theme::INK } else { theme::MUTED };
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, rect.height() / 2.0, theme::FAINT);
    }
    paint_icon(ui.painter(), Rect::from_center_size(egui::pos2(rect.left() + 13.5, rect.center().y), Vec2::splat(11.0)), icon, color);
    ui.painter().galley_with_override_text_color(egui::pos2(rect.left() + 23.0, rect.center().y - galley.size().y / 2.0), galley, color);
    response
}

/// A text-only plain button, retaining native keyboard focus and accessible button semantics.
pub fn text_button(ui: &mut Ui, title: &str, size: f32, weight: Weight, color: Color32) -> Response {
    ui.add(egui::Button::new(text(title, size, weight, color)).frame(false).min_size(Vec2::ZERO))
}

/// Swift `.buttonStyle(.plain)`: no inherited egui button padding or minimum hit-box height.
pub fn plain_button(ui: &mut Ui, title: &str, size: f32, weight: Weight, color: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(title.into(), theme::font(size, weight), color);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title));
    ui.painter().galley(rect.min, galley, color);
    if response.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), 3.0, Stroke::new(1.0, theme::AMBER), egui::StrokeKind::Outside);
    }
    response
}

/// egui horizontal rows start at its minimum widget height. Reserve the measured full height
/// before placing the first child, so an early spinner is centered beside later multiline text.
pub fn centered_row<R>(ui: &mut Ui, height: f32, draw: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), height), egui::Layout::left_to_right(egui::Align::Center), draw)
}

/// WelcomeView.swift's shared button styles (also used by AccountCard and DesktopCard).
pub fn secondary_button(ui: &mut Ui, title: &str) -> Response {
    styled_button(ui, title, false)
}

pub fn amber_button(ui: &mut Ui, title: &str) -> Response {
    styled_button(ui, title, true)
}

fn styled_button(ui: &mut Ui, title: &str, amber: bool) -> Response {
    let color = if amber { theme::black(0.85) } else { theme::INK };
    let galley = ui.painter().layout_no_wrap(title.into(), theme::font(if amber { 13.0 } else { 12.0 }, Weight::Semibold), color);
    let pad = if amber { egui::vec2(16.0, 8.0) } else { egui::vec2(12.0, 6.0) };
    let (rect, response) = ui.allocate_exact_size(galley.size() + pad * 2.0, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title));
    let pressed = response.is_pointer_button_down_on();
    let fill =
        if amber { theme::AMBER.gamma_multiply(if pressed { 0.8 } else { 1.0 }) } else { theme::white(if pressed { 0.18 } else { 0.1 }) };
    let opacity = if ui.is_enabled() { 1.0 } else { 0.5 };
    ui.painter().rect_filled(rect, if amber { 9.0 } else { 8.0 }, fill.gamma_multiply(opacity));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color.gamma_multiply(opacity));
    if response.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), 10.0, Stroke::new(1.0, theme::AMBER), egui::StrokeKind::Outside);
    }
    response
}

/// Glass paint only: a dark tint plus source rim/sheen. Native live backdrop material is the
/// shell's responsibility. Insert at a reserved paint slot behind content once its height is known.
pub fn paint_glass(painter: &Painter, rect: Rect, radius: f32, tint: f32) {
    painter.add(glass_shape(rect, radius, tint));
}

/// Use the source tint except where the platform requests a denser no-blur fallback
/// (Linux without compositor blur), so sharp wallpaper does not overwhelm the content.
/// This is an opacity fallback, not an implementation of live backdrop blur.
pub fn material_tint(tint: f32, dense_glass: bool) -> f32 {
    if dense_glass { tint.max((0.85 - 0.35) / 0.65) } else { tint }
}

/// Reserved-slot version, for surfaces whose measured height is only known after layout.
pub fn glass_shape(rect: Rect, radius: f32, tint: f32) -> Shape {
    // Swift's glassSurface adds .35 black over GlassContainer's native tint.
    let alpha = 1.0 - (1.0 - tint.clamp(0.0, 1.0)) * 0.65;
    rimmed_shape(rect, radius, theme::black(alpha))
}

/// The same Swift GlassRim/sheen over a surface-specific fill. Point & Ask has its own
/// nearly opaque gray background, rather than glassSurface's extra .35 black layer.
pub fn rimmed_shape(rect: Rect, radius: f32, fill: Color32) -> Shape {
    let mut shapes = vec![Shape::rect_filled(rect, radius, fill)];
    let perimeter = rounded_points(rect.shrink(0.5), radius.min(rect.width() / 2.0).min(rect.height() / 2.0));
    let color = move |y: f32, rim: bool| {
        let t = ((y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        let a = if rim {
            if t <= 0.5 { 0.42 + (0.10 - 0.42) * t * 2.0 } else { 0.10 + (0.18 - 0.10) * (t - 0.5) * 2.0 }
        } else {
            0.10 * (1.0 - t * 2.0).max(0.0)
        };
        theme::white(a)
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), Color32::TRANSPARENT);
    // Restrict sheen triangulation to the top half. A full-perimeter fan incorrectly spreads
    // highlights diagonally below the midpoint rather than drawing Swift's vertical gradient.
    let mut top = perimeter[..13].to_vec();
    top.push(egui::pos2(rect.right() - 0.5, rect.center().y));
    top.push(egui::pos2(rect.left() + 0.5, rect.center().y));
    top.extend_from_slice(&perimeter[perimeter.len() - 13..]);
    for point in &top {
        mesh.colored_vertex(*point, color(point.y, false));
    }
    for i in 0..top.len() {
        mesh.add_triangle(0, i as u32 + 1, ((i + 1) % top.len()) as u32 + 1);
    }
    shapes.push(Shape::mesh(mesh));
    // One closed, feathered path: separate antialiased segments produce bright join seams.
    shapes.push(Shape::Path(egui::epaint::PathShape::closed_line(
        perimeter,
        egui::epaint::PathStroke::new_uv(1.0, move |_, pos| color(pos.y, true)),
    )));
    Shape::Vec(shapes)
}

fn rounded_points(rect: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius.max(0.0);
    let centers = [
        (rect.right() - r, rect.top() + r, -std::f32::consts::FRAC_PI_2),
        (rect.right() - r, rect.bottom() - r, 0.0),
        (rect.left() + r, rect.bottom() - r, std::f32::consts::FRAC_PI_2),
        (rect.left() + r, rect.top() + r, std::f32::consts::PI),
    ];
    centers
        .into_iter()
        .flat_map(|(x, y, start)| {
            (0..=12).map(move |i| {
                let a = start + i as f32 * std::f32::consts::FRAC_PI_2 / 12.0;
                egui::pos2(x + r * a.cos(), y + r * a.sin())
            })
        })
        .collect()
}

pub fn harness_logo(ui: &mut Ui, assets: &Assets, kind: &AgentKind, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if let Some(texture) = assets.harness(kind) {
        ui.put(rect, egui::Image::new(texture).fit_to_exact_size(rect.size()).corner_radius(size * 0.22));
    } else {
        ui.painter().rect_filled(rect, size * 0.22, theme::FAINT);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            kind.display_name().chars().next().unwrap_or('?'),
            theme::font(size * 0.46, Weight::Rounded),
            theme::INK,
        );
    }
    response.on_hover_text(kind.display_name())
}

pub fn agent_badge(ui: &mut Ui, assets: &Assets, agent: &Agent, size: f32) -> Response {
    let response = harness_logo(ui, assets, &agent.kind, size);
    let center = response.rect.right_bottom() + egui::vec2(2.0 - size * 0.17, 2.0 - size * 0.17);
    ui.painter().circle(center, size * 0.17, theme::state_color(agent.state), Stroke::new(1.5, theme::black(0.7)));
    response
}

pub fn state_chip(ui: &mut Ui, agent: &Agent) -> Response {
    if !agent.is_live() {
        let galley = ui.painter().layout_no_wrap("Recent".into(), theme::font(10.5, Weight::Medium), theme::MUTED);
        let (rect, response) = ui.allocate_exact_size(galley.size() + egui::vec2(14.0, 4.0), Sense::hover());
        ui.painter().rect_filled(rect, rect.height() / 2.0, theme::FAINT);
        ui.painter().galley(rect.min + egui::vec2(7.0, 2.0), galley, theme::MUTED);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), "Recent"));
        return response.on_hover_text(recent_help(agent));
    }
    let color = if agent.state.wants_you() { theme::AMBER } else { theme::MUTED };
    let galley = ui.painter().layout_no_wrap(theme::state_label(agent.state).into(), theme::font(11.0, Weight::Medium), color);
    let (rect, response) = ui.allocate_exact_size(galley.size() + egui::vec2(11.0, 0.0), Sense::hover());
    ui.painter().circle_filled(egui::pos2(rect.left() + 3.0, rect.center().y), 3.0, theme::state_color(agent.state));
    ui.painter().galley(rect.min + egui::vec2(11.0, 0.0), galley, color);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), theme::state_label(agent.state)));
    response
}

/// The "Recent" chip's help (PanelView.swift `AgentStateChip`).
pub fn recent_help(agent: &Agent) -> String {
    format!("A {} session that isn't running now", agent.source.as_deref().unwrap_or("app"))
}

pub fn state_chip_width(ui: &Ui, agent: &Agent) -> f32 {
    let (title, size, padding) = if agent.is_live() { (theme::state_label(agent.state), 11.0, 11.0) } else { ("Recent", 10.5, 14.0) };
    ui.painter().layout_no_wrap(title.into(), theme::font(size, Weight::Medium), theme::MUTED).size().x + padding
}

#[derive(Clone, Copy, Debug)]
pub struct MarkdownStyle {
    pub size: f32,
    pub color: Color32,
    pub max_rows: usize,
}
impl Default for MarkdownStyle {
    fn default() -> Self {
        Self { size: 12.5, color: theme::white(0.9), max_rows: usize::MAX }
    }
}

/// Inline-only, preserving whitespace like Swift's AttributedString. A safe link click is
/// returned for UiAction::OpenUrl; full chat block rendering uses `blocks_linked` below.
pub fn markdown_linked(ui: &mut Ui, source: &str, style: MarkdownStyle, line_spacing: f32) -> (Response, Option<String>) {
    super::markdown::render(ui, source, style, line_spacing)
}

#[cfg(test)]
fn inline_job(source: &str, style: MarkdownStyle) -> egui::text::LayoutJob {
    super::markdown::inline_job(source, style)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(String),
    Heading(String),
    Item { marker: String, text: String },
    Code(String),
}

/// Port of `MarkdownBlocks.parse`, including incomplete streamed code fences.
pub fn parse_blocks(source: &str) -> Vec<Block> {
    fn flush(paragraph: &mut Vec<&str>, out: &mut Vec<Block>) {
        if !paragraph.is_empty() {
            out.push(Block::Paragraph(paragraph.join("\n")));
            paragraph.clear();
        }
    }
    let mut out = Vec::new();
    let mut paragraph = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    for line in source.split('\n') {
        let trimmed = line.trim();
        if let Some(lines) = &mut code {
            if trimmed.starts_with("```") {
                out.push(Block::Code(lines.join("\n")));
                code = None;
            } else {
                lines.push(line);
            }
        } else if trimmed.starts_with("```") {
            flush(&mut paragraph, &mut out);
            code = Some(Vec::new());
        } else if trimmed.is_empty() {
            flush(&mut paragraph, &mut out);
        } else if trimmed.starts_with('#') {
            flush(&mut paragraph, &mut out);
            out.push(Block::Heading(trimmed.trim_start_matches('#').trim().into()));
        } else if let Some(body) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")).or_else(|| trimmed.strip_prefix("• "))
        {
            flush(&mut paragraph, &mut out);
            out.push(Block::Item { marker: "•".into(), text: body.into() });
        } else if let Some((number, body)) = trimmed.split_once(". ")
            && !number.is_empty()
            && number.chars().all(char::is_numeric)
        {
            flush(&mut paragraph, &mut out);
            out.push(Block::Item { marker: format!("{number}."), text: body.into() });
        } else {
            paragraph.push(line);
        }
    }
    if let Some(code) = code {
        out.push(Block::Code(code.join("\n")));
    }
    flush(&mut paragraph, &mut out);
    out
}

#[derive(Default)]
pub struct BlockActions {
    pub copy: Option<String>,
    pub link: Option<String>,
}

/// Full conversation Markdown, returning code/link intents rather than running side effects.
pub fn blocks_linked(ui: &mut Ui, source: &str, id: Id) -> BlockActions {
    let mut actions = BlockActions::default();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        for (i, block) in parse_blocks(source).into_iter().enumerate() {
            match block {
                Block::Paragraph(body) => {
                    if let Some(link) = markdown_linked(ui, &body, MarkdownStyle::default(), 0.0).1 {
                        actions.link = Some(link);
                    }
                }
                Block::Heading(body) => {
                    label(ui, body, 13.0, Weight::Semibold, theme::INK);
                }
                Block::Item { marker, text: body } => {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        label(ui, marker, 12.5, Weight::Regular, theme::MUTED);
                        if let Some(link) = markdown_linked(ui, &body, MarkdownStyle::default(), 0.0).1 {
                            actions.link = Some(link);
                        }
                    });
                }
                Block::Code(body) => {
                    egui::Frame::new()
                        .fill(theme::black(0.3))
                        .stroke(Stroke::new(1.0, theme::HAIRLINE))
                        .corner_radius(8)
                        .inner_margin(7)
                        .show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                let width = (ui.available_width() - 24.0).max(0.0);
                                egui::ScrollArea::horizontal().id_salt((id, i)).max_width(width).show(ui, |ui| {
                                    ui.add(Label::new(text(&body, 11.5, Weight::Mono, theme::white(0.9))).extend().selectable(true));
                                });
                                if icon_button(ui, (id, i, "copy"), Icon::Copy, 22.0, theme::MUTED, false)
                                    .on_hover_text("Copy code")
                                    .clicked()
                                {
                                    actions.copy = Some(body.clone());
                                }
                            });
                        });
                }
            }
        }
    });
    actions
}

pub fn time_ago(since_ms: Option<f64>, now_ms: f64) -> String {
    let Some(since) = since_ms else {
        return String::new();
    };
    let seconds = ((now_ms - since) / 1000.0).max(0.0) as u64;
    match seconds {
        0..60 => "now".into(),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_fonts_art_and_vector_controls_render_without_host_resources() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(700.0, 900.0))), ..Default::default() },
            |ui| {
                paint_glass(ui.painter(), Rect::from_min_size(Pos2::ZERO, egui::vec2(380.0, 600.0)), 22.0, 0.5);
                for weight in [Weight::Regular, Weight::Medium, Weight::Semibold, Weight::Bold, Weight::Rounded, Weight::Mono] {
                    label(ui, "Lantern · café Ελληνικά", 12.5, weight, theme::INK);
                }
                for kind in AgentKind::SUPPORTED {
                    let agent = Agent { kind: AgentKind::new(kind), ..Default::default() };
                    assert!(assets.harness(&agent.kind).is_some());
                    agent_badge(ui, &assets, &agent, 26.0);
                }
                use Icon::*;
                ui.horizontal_wrapped(|ui| {
                    for icon in [
                        Sparkles,
                        Layers,
                        Plus,
                        PointAsk,
                        Close,
                        Back,
                        ChevronDown,
                        ChevronUpDown,
                        Reply,
                        Open,
                        Dismiss,
                        Mic,
                        MicFill,
                        Waveform,
                        Send,
                        ArrowUp,
                        Return,
                        Cpu,
                        Folder,
                        Terminal,
                        Window,
                        Copy,
                        Check,
                        Hourglass,
                        Warning,
                        Reselect,
                        Link,
                        TextCursor,
                        Wifi,
                        Video,
                        Photo,
                        AccountCircle,
                        Mail,
                        Play,
                        Exclamation,
                    ] {
                        icon_button(ui, icon, icon, 32.0, theme::MUTED, false);
                    }
                });
                let _ = markdown_linked(ui, "**Hello**\n`let x = 1;`", MarkdownStyle::default(), 0.0);
            },
        );
        assert!(!output.textures_delta.set.is_empty());
        assert!(!ctx.tessellate(output.shapes, output.pixels_per_point).is_empty());
        // This headless test intentionally has no GPU backend to apply uploads to.
        output.textures_delta.clear();
    }
    #[test]
    fn blocks_match_swift_and_keep_streamed_code() {
        assert_eq!(
            parse_blocks("# Title\n\na\nb\n- x\n2. y\n```rust\nlet x = 1;"),
            vec![
                Block::Heading("Title".into()),
                Block::Paragraph("a\nb".into()),
                Block::Item { marker: "•".into(), text: "x".into() },
                Block::Item { marker: "2.".into(), text: "y".into() },
                Block::Code("let x = 1;".into())
            ]
        );
    }
    #[test]
    fn inline_keeps_whitespace_unicode_and_unclosed_delimiters() {
        let j = inline_job("hi **bold**\n`code` café *cut", MarkdownStyle::default());
        assert_eq!(j.text, "hi bold\ncode café *cut");
        assert!(j.sections.iter().any(|s| s.format.font_id == theme::font(12.5, Weight::Semibold)));
    }
    #[test]
    fn relative_time_is_fixed_clock_and_clamped() {
        assert_eq!(time_ago(None, 1000.0), "");
        assert_eq!(time_ago(Some(2000.0), 1000.0), "now");
        assert_eq!(time_ago(Some(0.0), 3600000.0), "1h");
    }

    #[test]
    fn measured_rows_center_an_early_spinner_with_multiline_text_and_a_later_button() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut centers = None;
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(380.0, 100.0))), ..Default::default() },
            |ui| {
                centered_row(ui, 60.0, |ui| {
                    let spinner = spinner(ui, 12.0, theme::MUTED, &super::super::Env::default());
                    let text = label(ui, "First line\nSecond line\nThird line", 12.0, Weight::Regular, theme::INK);
                    let button = plain_button(ui, "Cancel", 11.5, Weight::Semibold, theme::MUTED);
                    centers = Some((spinner.rect.center().y, text.rect.center().y, button.rect.center().y));
                });
            },
        );
        output.textures_delta.clear();
        let (spinner, text, button) = centers.unwrap();
        assert!((spinner - text).abs() < 0.5, "spinner {spinner} vs text {text}");
        assert!((spinner - button).abs() < 0.5, "spinner {spinner} vs button {button}");
    }

    #[test]
    fn material_fallback_reaches_eighty_five_percent_without_lightening_denser_glass() {
        for tint in [0.45, 0.5, 0.62, 0.9, 1.0] {
            assert_eq!(material_tint(tint, false), tint);
            let fallback = material_tint(tint, true);
            assert!(fallback >= tint);
            assert!(0.35 + 0.65 * fallback >= 0.85 - f32::EPSILON);
            let Shape::Vec(shapes) = glass_shape(Rect::from_min_size(Pos2::ZERO, egui::vec2(50.0, 100.0)), 22.0, fallback) else {
                panic!("glass layers");
            };
            let Shape::Rect(fill) = &shapes[0] else {
                panic!("glass fill");
            };
            assert!(fill.fill.a() >= 216);
        }
        assert_eq!(material_tint(0.9, true), 0.9);
    }

    #[test]
    fn glass_has_a_single_closed_rim_and_sheen_stops_at_midpoint() {
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(380.0, 500.0));
        let Shape::Vec(shapes) = glass_shape(rect, 22.0, 0.5) else {
            panic!("glass layers");
        };
        assert_eq!(shapes.len(), 3);
        let Shape::Mesh(sheen) = &shapes[1] else {
            panic!("sheen mesh");
        };
        assert!(sheen.vertices.iter().all(|vertex| vertex.pos.y <= rect.center().y));
        let Shape::Path(rim) = &shapes[2] else {
            panic!("rim path");
        };
        assert!(rim.closed);
        assert_eq!(rim.stroke.width, 1.0);
    }
}
#[test]
fn plain_return_does_not_consume_shift_or_alt_return() {
    let ctx = egui::Context::default();
    let mut consumed = false;
    let mut remaining = Vec::new();
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: [egui::Modifiers::SHIFT, egui::Modifiers::ALT, egui::Modifiers::NONE]
                .into_iter()
                .map(|modifiers| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers })
                .collect(),
            ..Default::default()
        },
        |ui| {
            consumed = consume_plain_key(ui, egui::Key::Enter);
            remaining = ui.input(|input| input.events.clone());
            assert!(!consume_plain_key(ui, egui::Key::Enter));
        },
    );
    output.textures_delta.clear();
    assert!(consumed);
    assert_eq!(remaining.len(), 2);
    assert!(remaining.iter().all(|event| matches!(event, egui::Event::Key { modifiers, .. } if *modifiers != egui::Modifiers::NONE)));
}
