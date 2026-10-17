//! Port of `WelcomeView.swift`: what Lantern is, the agents it found, and the optional setup, in
//! three steps on a fixed 540 x 640 surface. Only the platform noun changes from the Mac's copy.
use super::{
    Ctx, SettingChange, UiAction,
    components::{self as c, paint_icon},
    icons::Icon,
    sessions,
    theme::{self as t, Weight},
};
use crate::model::{AgentKind, HookState, Platform};
use eframe::egui::{self, Align, Color32, Galley, Key, Label, Layout, Modifiers, Rect, Sense, Ui, UiBuilder, Vec2, vec2};
use std::sync::Arc;

pub const SIZE: Vec2 = vec2(540.0, 640.0);
/// Swift `Color(white: 0.09)`.
const BACKGROUND: Color32 = Color32::from_gray(23);
/// SwiftUI's dark-mode `.orange`.
const ORANGE: Color32 = Color32::from_rgb(255, 159, 10);

pub fn render(ui: &mut Ui, cx: &mut Ctx) -> f32 {
    let rect = Rect::from_min_size(ui.cursor().min, SIZE);
    ui.painter().rect_filled(rect, 0.0, BACKGROUND);
    let step = cx.view.welcome_step.min(2);
    // `.keyboardShortcut(.defaultAction)` on Continue / Start using Lantern.
    if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter)) {
        advance(cx, step);
    }
    let primary = if step == 2 { "Start using Lantern" } else { "Continue" };
    let button_height = ui.painter().layout_no_wrap(primary.into(), t::font(13.0, Weight::Semibold), t::INK).size().y + 16.0;
    let footer = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - button_height - 40.0), rect.max);
    let content = Rect::from_min_max(rect.min + vec2(36.0, 32.0), egui::pos2(rect.right() - 36.0, footer.top()));
    // Swift's max-height frame constrains layout but does not clip its fixed-size children.
    // Preserve painting into the footer area in oversized error states, without painting
    // outside the viewport. Exact SwiftUI compression in those states still needs a Mac run.
    let clip = ui.clip_rect().intersect(rect);
    ui.scope_builder(
        UiBuilder::new().max_rect(content).layout(Layout::top_down(if step == 1 { Align::Min } else { Align::Center })),
        |ui| {
            ui.set_clip_rect(clip);
            match step {
                0 => intro(ui, cx),
                1 => setup(ui, cx),
                _ => done(ui, cx),
            }
        },
    );
    footer_bar(ui, cx, footer.shrink2(vec2(28.0, 20.0)), step, primary);
    ui.allocate_rect(rect, Sense::hover());
    SIZE.y
}

fn advance(cx: &mut Ctx, step: usize) {
    if step >= 2 {
        cx.emit(UiAction::FinishWelcome);
    } else {
        cx.view.welcome_step = step + 1;
    }
}

fn footer_bar(ui: &mut Ui, cx: &mut Ctx, bar: Rect, step: usize, primary: &str) {
    for i in 0..3 {
        let center = egui::pos2(bar.left() + 3.0 + i as f32 * 12.0, bar.center().y);
        ui.painter().circle_filled(center, 3.0, if i == step { t::AMBER } else { t::white(0.2) });
    }
    ui.scope_builder(UiBuilder::new().max_rect(bar).layout(Layout::right_to_left(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if c::amber_button(ui, primary).clicked() {
            advance(cx, step);
        }
        // Every step after the first can go back to the one before it.
        if step > 0 && c::plain_button(ui, "Back", 13.0, Weight::Regular, t::MUTED).clicked() {
            cx.view.welcome_step = step - 1;
        }
    });
}

fn intro(ui: &mut Ui, cx: &mut Ctx) {
    ui.spacing_mut().item_spacing.y = 18.0;
    let (logo, _) = ui.allocate_exact_size(Vec2::splat(96.0), Sense::hover());
    // `.shadow(color: Theme.amber.opacity(0.5), radius: 24)`: the logo's own silhouette, blurred.
    // A SwiftUI shadow draws past the padding, so it isn't clipped to the content area.
    let mut unclipped = ui.painter().clone();
    unclipped.set_clip_rect(ui.ctx().content_rect());
    unclipped.image(
        cx.assets.logo_shadow.id(),
        Rect::from_center_size(logo.center(), Vec2::splat(96.0 + 2.0 * 72.0)),
        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        t::opacity(t::AMBER, 0.5),
    );
    egui::Image::new(&cx.assets.logo).paint_at(ui, logo);
    centered(ui, "October Lantern", 26.0, Weight::Bold, t::INK);
    centered(
        ui,
        "A small light on the edge of your screen that watches every coding agent on this computer, and lights up when one of them needs you.",
        14.0,
        Weight::Regular,
        t::MUTED,
    );
    centered(ui, &found(cx.model.agents.len()), 13.0, Weight::Medium, t::AMBER);
    ui.add_space(6.0);
    state_rows(ui, cx);
}

fn found(count: usize) -> String {
    if count == 0 {
        "No agents running yet. Start Claude Code, Codex or another agent as usual and Lantern finds it.".into()
    } else {
        format!("Lantern already found {count} agent{} on this computer.", if count == 1 { "" } else { "s" })
    }
}

fn centered(ui: &mut Ui, text: &str, size: f32, weight: Weight, color: Color32) {
    ui.add(Label::new(c::text(text, size, weight, color)).wrap().halign(Align::Center));
}

/// The legend card: sized to its widest row and centred, like a SwiftUI VStack in a VStack.
fn state_rows(ui: &mut Ui, cx: &Ctx) {
    const ROWS: [(Option<Color32>, Option<u32>, &str, &str); 3] = [
        (None, None, "Dim", "Nothing needs you."),
        (Some(ORANGE), None, "Soft glow", "Your agents are working."),
        (Some(t::AMBER), Some(2), "Amber, with a number", "That many agents finished or are asking you something."),
    ];
    let layout = |text: &str, size: f32, weight: Weight, color: Color32| -> Arc<Galley> {
        ui.painter().layout_no_wrap(text.into(), t::font(size, weight), color)
    };
    let rows: Vec<_> = ROWS
        .iter()
        .map(|&(glow, count, title, detail)| {
            (glow, count, layout(title, 13.0, Weight::Semibold, t::INK), layout(detail, 12.0, Weight::Regular, t::MUTED))
        })
        .collect();
    let text_width = rows.iter().map(|(_, _, title, detail)| title.size().x.max(detail.size().x)).fold(0.0, f32::max);
    let row_height = |title: &Galley, detail: &Galley| (title.size().y + 2.0 + detail.size().y).max(32.0);
    let inner_height: f32 =
        rows.iter().map(|(_, _, title, detail)| row_height(title, detail)).sum::<f32>() + 12.0 * (rows.len() - 1) as f32;
    let width = (16.0 + 32.0 + 14.0 + text_width + 16.0).min(ui.available_width());
    let (card, _) = ui.allocate_exact_size(vec2(width, inner_height + 32.0), Sense::hover());
    ui.painter().rect_filled(card, 14.0, t::FAINT);
    let mut y = card.top() + 16.0;
    for (glow, count, title, detail) in rows {
        let height = row_height(&title, &detail);
        let icon = Rect::from_min_size(egui::pos2(card.left() + 16.0, y + (height - 32.0) / 2.0), Vec2::splat(32.0));
        state_icon(ui, cx, icon, glow, count);
        let text_top = y + (height - title.size().y - 2.0 - detail.size().y) / 2.0;
        let x = icon.right() + 14.0;
        let detail_top = text_top + title.size().y + 2.0;
        ui.painter().galley(egui::pos2(x, text_top), title, t::INK);
        ui.painter().galley(egui::pos2(x, detail_top), detail, t::MUTED);
        y += height + 12.0;
    }
}

/// Swift `StateRow`'s 32pt ZStack: a blurred glow, the lantern (desaturated when dim) and a count.
fn state_icon(ui: &Ui, cx: &Ctx, rect: Rect, glow: Option<Color32>, count: Option<u32>) {
    if let Some(glow) = glow {
        // `Circle().fill(glow.opacity(0.5)).blur(radius: 8)` filling the 32pt frame (radius 8 as sigma).
        blurred_disk(ui.painter(), rect.center(), 16.0, 8.0, t::opacity(glow, 0.5));
    }
    if glow.is_some() {
        egui::Image::new(&cx.assets.logo).paint_at(ui, rect);
    } else {
        egui::Image::new(&cx.assets.logo_idle).tint(t::white(0.8)).paint_at(ui, rect);
    }
    if let Some(count) = count {
        let galley = ui.painter().layout_no_wrap(count.to_string(), t::font(9.0, Weight::Rounded), t::black(0.85));
        let size = vec2(galley.size().x.max(14.0), galley.size().y.max(14.0));
        let badge = Rect::from_center_size(egui::pos2(rect.center().x, rect.bottom() - size.y / 2.0 + 3.0), size);
        ui.painter().rect_filled(badge, size.y / 2.0, t::AMBER);
        ui.painter().galley(badge.center() - galley.size() / 2.0, galley, t::black(0.85));
    }
}

const RINGS: usize = 24;

/// How much of a disk of `radius`, blurred with `sigma`, covers a point `distance` from its centre
/// (a 32 x 32 grid over the disk, so it costs the same at any size).
fn coverage(radius: f32, sigma: f32, distance: f32) -> f32 {
    let cell = radius / 16.0;
    let mut sum = 0.0;
    for gx in -16..16 {
        for gy in -16..16 {
            let (x, y) = ((gx as f32 + 0.5) * cell, (gy as f32 + 0.5) * cell);
            if x * x + y * y <= radius * radius {
                sum += (-((x - distance).powi(2) + y * y) / (2.0 * sigma * sigma)).exp();
            }
        }
    }
    sum * cell * cell / (2.0 * std::f32::consts::PI * sigma * sigma)
}

/// `coverage` at `RINGS + 1` points from the centre out to `radius + 3 sigma`.
fn blur_profile(radius: f32, sigma: f32) -> [f32; RINGS + 1] {
    let step = (radius + 3.0 * sigma) / RINGS as f32;
    std::array::from_fn(|i| coverage(radius, sigma, i as f32 * step))
}

/// A disk convolved with a Gaussian, as a radial mesh (egui has no blur filter). Stands in for
/// Approximate SwiftUI's blurred circle with a Gaussian-convolved disk on a radial mesh.
/// `sigma` is our kernel's standard deviation, not a verified SwiftUI filter equivalence.
pub fn blurred_disk(painter: &egui::Painter, center: egui::Pos2, radius: f32, sigma: f32, color: Color32) {
    const SEGMENTS: usize = 48;
    let profile = blur_profile(radius, sigma);
    let step = (radius + 3.0 * sigma) / RINGS as f32;
    let tint = |alpha: f32| color.gamma_multiply(alpha.clamp(0.0, 1.0));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, tint(profile[0]));
    for (ring, alpha) in profile.iter().enumerate().skip(1) {
        for s in 0..SEGMENTS {
            let angle = s as f32 * std::f32::consts::TAU / SEGMENTS as f32;
            mesh.colored_vertex(center + vec2(angle.cos(), angle.sin()) * ring as f32 * step, tint(*alpha));
        }
    }
    let at = |ring: usize, s: usize| (1 + (ring - 1) * SEGMENTS + s % SEGMENTS) as u32;
    for s in 0..SEGMENTS {
        mesh.add_triangle(0, at(1, s), at(1, s + 1));
        for ring in 1..RINGS {
            mesh.add_triangle(at(ring, s), at(ring + 1, s), at(ring + 1, s + 1));
            mesh.add_triangle(at(ring, s), at(ring + 1, s + 1), at(ring, s + 1));
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn setup(ui: &mut Ui, cx: &mut Ctx) {
    ui.spacing_mut().item_spacing.y = 14.0;
    c::label(ui, "A few choices", 22.0, Weight::Bold, t::INK);
    c::label(ui, "All optional. You can change them later in Settings.", 13.5, Weight::Regular, t::MUTED);
    let prefs = &cx.model.prefs;
    let mut open_at_login = prefs.open_at_login;
    setup_row(ui, Icon::Power, "Open at login", "Keep the lantern there whenever you're using your computer.", |ui| {
        let response = sessions::switch(ui, "", &mut open_at_login);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), open_at_login, "Open at login"));
        if response.changed() {
            cx.emit(UiAction::ChangeSetting(SettingChange::OpenAtLogin(open_at_login)));
        }
    });
    let mut computer_use = prefs.computer_use;
    setup_row(
        ui,
        Icon::PointAsk,
        "Let agents use your apps",
        "Agents can use your apps and browser for you, in the background, with their own cursor.",
        |ui| {
            let response = sessions::switch(ui, "", &mut computer_use);
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), computer_use, "Let agents use your apps")
            });
            if response.changed() {
                cx.emit(UiAction::ChangeSetting(SettingChange::ComputerUse(computer_use)));
            }
        },
    );
    setup_row(
        ui,
        Icon::BellBadge,
        "Notifications",
        "A notification when an agent finishes or asks you something, even when the lantern is out of sight.",
        |ui| notifications_control(ui, cx),
    );
    setup_row(ui, Icon::Bolt, "Exact status (recommended)", &hooks_detail(cx.model.caps.platform), |ui| hooks_control(ui, cx));
    // Swift shows "Share anonymous usage counts" only when analytics is built in (`Analytics.available`).
    // These builds have no analytics backend, so, like an unavailable Mac build, the row isn't shown.
    let hotkey = crate::model::hotkey_label(&cx.model.prefs.message_hotkey);
    setup_row(ui, Icon::Keyboard, "Shortcut", "Opens the message box from anywhere.", |ui| {
        let galley = ui.painter().layout_no_wrap(hotkey, t::font(12.5, Weight::Rounded), t::INK);
        let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(16.0, 6.0), Sense::hover());
        ui.painter().rect_filled(rect, 6.0, t::FAINT);
        ui.painter().galley(rect.min + vec2(8.0, 3.0), galley, t::INK);
    });
}

fn hooks_detail(platform: Platform) -> String {
    let files = match platform {
        Platform::Windows => r"the .claude\settings.json and .codex\config.toml files in your user folder",
        Platform::Linux => "~/.claude/settings.json and ~/.codex/config.toml",
    };
    format!(
        "Lets Claude Code and Codex tell Lantern the moment they finish or need permission, so you can Allow or Deny from Lantern. Adds Lantern to {files}, after backing both up. Restart running agents afterwards."
    )
}

/// Swift `SetupRow`: icon, title and wrapping detail, then the control top-aligned at the trailing edge.
fn setup_row(ui: &mut Ui, icon: Icon, title: &str, detail: &str, control: impl FnOnce(&mut Ui)) {
    egui::Frame::new().fill(t::FAINT).corner_radius(12).inner_margin(12).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            control(ui);
            ui.add_space(12.0);
            ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let title_height = ui.painter().layout_no_wrap(title.into(), t::font(13.5, Weight::Semibold), t::INK).size().y;
                let (column, _) = ui.allocate_exact_size(vec2(22.0, title_height), Sense::hover());
                paint_icon(ui.painter(), Rect::from_center_size(column.center(), Vec2::splat(15.0)), icon, t::AMBER);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    c::label(ui, title, 13.5, Weight::Semibold, t::INK);
                    c::label(ui, detail, 12.0, Weight::Regular, t::MUTED);
                });
            });
        });
    });
}

/// `Label("On", systemImage: "checkmark")` in green.
fn on_label(ui: &mut Ui) {
    let galley = ui.painter().layout_no_wrap("On".into(), t::font(12.0, Weight::Semibold), t::GREEN);
    let (rect, response) = ui.allocate_exact_size(vec2(12.0 + 4.0 + galley.size().x, galley.size().y), Sense::hover());
    paint_icon(
        ui.painter(),
        Rect::from_center_size(egui::pos2(rect.left() + 6.0, rect.center().y), Vec2::splat(11.0)),
        Icon::Check,
        t::GREEN,
    );
    ui.painter().galley(egui::pos2(rect.left() + 16.0, rect.top()), galley, t::GREEN);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "On"));
}

fn notifications_control(ui: &mut Ui, cx: &mut Ctx) {
    let allowed = cx.model.system.notifications_allowed;
    if allowed == Some(false) {
        if c::secondary_button(ui, "Open Settings…").on_hover_text("Notifications are off for Lantern in your system settings").clicked()
        {
            cx.emit(UiAction::OpenNotificationSettings);
        }
    } else if cx.model.prefs.notifications {
        on_label(ui);
    } else if c::secondary_button(ui, "Turn On").clicked() {
        cx.emit(UiAction::ChangeSetting(SettingChange::NotificationsEnabled(true)));
    }
}

fn hooks_control(ui: &mut Ui, cx: &mut Ctx) {
    let system = &cx.model.system;
    let state = |kind: &str| system.hooks.get(kind).copied().unwrap_or_default();
    let (claude, codex) = (state(AgentKind::CLAUDE), state(AgentKind::CODEX));
    if claude == HookState::On && codex == HookState::On {
        on_label(ui);
    } else if system.hooks_busy {
        c::spinner(ui, 16.0, t::MUTED, cx.env);
    } else {
        ui.with_layout(Layout::top_down(Align::Max), |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            if c::secondary_button(ui, if claude == HookState::NeedsUpdate { "Update" } else { "Turn On" }).clicked() {
                cx.emit(UiAction::SetHooks { enabled: true });
            }
            let note = if let Some(error) = &system.hooks_error {
                Some((error.as_str(), t::RED))
            } else if (claude == HookState::On) != (codex == HookState::On) {
                Some((if claude == HookState::On { "On for Claude Code only" } else { "On for Codex only" }, t::MUTED))
            } else {
                None
            };
            if let Some((note, color)) = note {
                ui.set_max_width(220.0);
                ui.add(Label::new(c::text(note, 11.0, Weight::Regular, color)).wrap().halign(Align::Max));
            }
        });
    }
}

fn done(ui: &mut Ui, cx: &mut Ctx) {
    ui.spacing_mut().item_spacing.y = 16.0;
    let (logo, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
    egui::Image::new(&cx.assets.logo).paint_at(ui, logo);
    centered(ui, "You're set", 24.0, Weight::Bold, t::INK);
    egui::Frame::new().fill(t::FAINT).corner_radius(14).inner_margin(16).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            for (icon, text) in tips(cx) {
                ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let line = ui.painter().layout_no_wrap("A".into(), t::font(13.0, Weight::Regular), t::INK).size().y;
                    let (column, _) = ui.allocate_exact_size(vec2(18.0, line), Sense::hover());
                    paint_icon(ui.painter(), Rect::from_center_size(column.center(), Vec2::splat(13.0)), icon, t::AMBER);
                    c::label(ui, text, 13.0, Weight::Regular, t::white(0.9));
                });
            }
        });
    });
}

/// Swift `done`'s tips, with this platform's shortcut, edge and tray.
fn tips(cx: &Ctx) -> Vec<(Icon, String)> {
    let prefs = &cx.model.prefs;
    let edge = match prefs.edge {
        crate::model::Edge::Left => "left",
        crate::model::Edge::Right => "right",
    };
    let point_ask = if prefs.point_ask_hotkey.is_empty() || prefs.point_ask_hotkey == "none" || !cx.model.caps.global_hotkeys {
        "the dotted-box button".to_string()
    } else {
        crate::model::hotkey_label(&prefs.point_ask_hotkey)
    };
    vec![
        (Icon::ArrowToEdge, format!("The lantern sits on the {edge} edge of your screen. Drag it to move it.")),
        (Icon::Layers, "Hover it and click the stack to see who's waiting. Click any agent to read its conversation and reply.".into()),
        (
            Icon::Sparkles,
            "Click the lantern to Ask Lantern: \"which agent needs me first?\", or tell it what to do and it starts an agent.".into(),
        ),
        (Icon::PointAsk, format!("Point & Ask: press {point_ask}, drag a box around anything on screen and ask about it.")),
        (Icon::Plus, "Start a new session with any agent from the + button.".into()),
        (
            Icon::MenuBar,
            if cx.model.caps.tray {
                "Right-click the lantern, or use the lantern in your system tray, for Settings and more.".into()
            } else {
                "Right-click the lantern for Settings and more.".into()
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Agent, Inputs, Model};
    use crate::ui::{Assets, Env, UiState, theme};

    fn frame(
        ctx: &egui::Context,
        assets: &Assets,
        model: &Model,
        view: &mut UiState,
        events: Vec<egui::Event>,
    ) -> (f32, Vec<UiAction>, Vec<(String, Rect)>) {
        fn collect(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(text) => texts.push((text.galley.job.text.clone(), text.galley.rect.translate(text.pos.to_vec2()))),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, texts)),
                _ => {}
            }
        }
        let mut height = 0.0;
        let mut actions = Vec::new();
        let mut inputs = Inputs::default();
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, SIZE)), events, ..Default::default() },
            |ui| {
                let mut cx = Ctx { model, inputs: &mut inputs, view, out: &mut actions, assets, env: &Env::default() };
                height = super::super::render_welcome(ui, &mut cx);
            },
        );
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut texts);
        }
        output.textures_delta.clear();
        (height, actions, texts)
    }

    fn setup() -> (egui::Context, Assets) {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        (ctx, assets)
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    fn click(ctx: &egui::Context, assets: &Assets, model: &Model, view: &mut UiState, target: &str) -> Vec<UiAction> {
        let (_, _, texts) = frame(ctx, assets, model, view, Vec::new());
        let pos = texts.iter().find(|(text, _)| text == target).unwrap_or_else(|| panic!("missing {target}: {texts:?}")).1.center();
        let mut out = Vec::new();
        for pressed in [true, false] {
            let (_, actions, _) = frame(
                ctx,
                assets,
                model,
                view,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE },
                ],
            );
            out.extend(actions);
        }
        out
    }

    #[test]
    fn setup_and_navigation_buttons_emit_only_the_coordinators_intents() {
        let (ctx, assets) = setup();
        let mut model = Model::default();
        let mut view = UiState { welcome_step: 1, ..Default::default() };
        model.system.notifications_allowed = Some(false);
        model.system.hooks.insert(AgentKind::CLAUDE.into(), HookState::NeedsUpdate);
        assert_eq!(click(&ctx, &assets, &model, &mut view, "Open Settings…"), vec![UiAction::OpenNotificationSettings]);
        assert_eq!(click(&ctx, &assets, &model, &mut view, "Update"), vec![UiAction::SetHooks { enabled: true }]);
        model.system.notifications_allowed = None;
        model.prefs.notifications = false;
        assert_eq!(
            click(&ctx, &assets, &model, &mut view, "Turn On"),
            vec![UiAction::ChangeSetting(SettingChange::NotificationsEnabled(true))]
        );
        assert!(click(&ctx, &assets, &model, &mut view, "Back").is_empty());
        assert_eq!(view.welcome_step, 0);
        assert!(click(&ctx, &assets, &model, &mut view, "Continue").is_empty());
        assert_eq!(view.welcome_step, 1);
        view.welcome_step = 2;
        assert!(click(&ctx, &assets, &model, &mut view, "Back").is_empty());
        assert_eq!(view.welcome_step, 1, "Back on the last step returns to setup");
        view.welcome_step = 2;
        assert_eq!(click(&ctx, &assets, &model, &mut view, "Start using Lantern"), vec![UiAction::FinishWelcome]);
    }

    #[test]
    fn fixed_size_setup_children_are_not_clipped_at_the_footer() {
        let (ctx, assets) = setup();
        let fixture: crate::app::fixture::Fixture = serde_json::from_str(include_str!("../../fixtures/welcome-setup-off.json")).unwrap();
        let mut inputs = fixture.inputs;
        let mut view = fixture.view;
        let mut actions = Vec::new();
        let mut output =
            ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, SIZE)), ..Default::default() }, |ui| {
                let mut cx = Ctx {
                    model: &fixture.model,
                    inputs: &mut inputs,
                    view: &mut view,
                    out: &mut actions,
                    assets: &assets,
                    env: &Env::default(),
                };
                super::super::render_welcome(ui, &mut cx);
            });
        let detail = output
            .shapes
            .iter()
            .find(|clipped| matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with("Lets Claude Code")))
            .expect("hooks detail");
        assert_eq!(detail.clip_rect.bottom(), SIZE.y, "layout bounds are not a paint clip in Swift");
        assert!(actions.is_empty());
        output.textures_delta.clear();
    }

    #[test]
    fn all_welcome_fixtures_use_the_real_route_and_fixed_window_dimensions() {
        let (ctx, assets) = setup();
        for raw in [
            include_str!("../../fixtures/welcome-intro.json"),
            include_str!("../../fixtures/welcome-intro-empty.json"),
            include_str!("../../fixtures/welcome-setup.json"),
            include_str!("../../fixtures/welcome-setup-off.json"),
            include_str!("../../fixtures/welcome-done.json"),
        ] {
            let mut fixture: crate::app::fixture::Fixture = serde_json::from_str(raw).unwrap();
            let (height, actions, texts) = frame(&ctx, &assets, &fixture.model, &mut fixture.view, Vec::new());
            assert_eq!(height, SIZE.y);
            assert!(actions.is_empty());
            assert!(texts.iter().all(|(_, rect)| rect.right() <= SIZE.x - 28.0 + 0.5));
        }
    }

    #[test]
    fn enter_walks_the_three_steps_then_finishes() {
        let (ctx, assets) = setup();
        let model = Model::default();
        let mut view = UiState::default();
        frame(&ctx, &assets, &model, &mut view, vec![key(Key::Enter)]);
        assert_eq!(view.welcome_step, 1);
        frame(&ctx, &assets, &model, &mut view, vec![key(Key::Enter)]);
        assert_eq!(view.welcome_step, 2);
        let (height, actions, texts) = frame(&ctx, &assets, &model, &mut view, vec![key(Key::Enter)]);
        assert_eq!(actions, vec![UiAction::FinishWelcome]);
        assert_eq!(height, 640.0);
        assert!(texts.iter().any(|(text, _)| text == "Start using Lantern"));
        assert!(texts.iter().any(|(text, _)| text == "Back"), "Back shows on the last step too");
        let (_, _, first) = frame(&ctx, &assets, &model, &mut UiState::default(), Vec::new());
        assert!(!first.iter().any(|(text, _)| text == "Back"), "the first step has nothing to go back to");
    }

    #[test]
    fn intro_counts_agents_and_never_says_mac() {
        let (ctx, assets) = setup();
        let mut model = Model::default();
        let (_, _, texts) = frame(&ctx, &assets, &model, &mut UiState::default(), Vec::new());
        assert!(texts.iter().any(|(text, _)| text.starts_with("No agents running yet.")));
        model.agents = vec![Agent::default(), Agent::default()];
        let (_, _, texts) = frame(&ctx, &assets, &model, &mut UiState::default(), Vec::new());
        assert!(texts.iter().any(|(text, _)| text == "Lantern already found 2 agents on this computer."));
        assert_eq!(found(1), "Lantern already found 1 agent on this computer.");
        for step in 0..3 {
            let (_, _, texts) = frame(&ctx, &assets, &model, &mut UiState { welcome_step: step, ..Default::default() }, Vec::new());
            assert!(texts.iter().all(|(text, _)| !text.contains("Mac")), "step {step}: {texts:?}");
            assert!(texts.iter().all(|(_, rect)| rect.right() <= 540.0 - 28.0 + 0.5 && rect.bottom() <= 640.0), "step {step} overflows");
        }
    }

    #[test]
    fn setup_rows_reflect_hook_notification_and_login_state() {
        let (ctx, assets) = setup();
        let mut model = Model::default();
        let mut view = UiState { welcome_step: 1, ..Default::default() };
        model.system.hooks = [("claude".to_string(), HookState::NeedsUpdate), ("codex".to_string(), HookState::On)].into();
        let (_, _, texts) = frame(&ctx, &assets, &model, &mut view, Vec::new());
        let has = |texts: &[(String, Rect)], wanted: &str| texts.iter().any(|(text, _)| text == wanted);
        assert!(has(&texts, "Update") && has(&texts, "On for Codex only") && has(&texts, "Back"));
        assert!(has(&texts, "Ctrl+Alt+Space"));
        model.system.hooks_busy = true;
        let (_, _, texts) = frame(&ctx, &assets, &model, &mut view, Vec::new());
        assert!(!has(&texts, "Update"), "a spinner while the hooks install");
        model.system.hooks_busy = false;
        model.system.hooks = [("claude".to_string(), HookState::On), ("codex".to_string(), HookState::On)].into();
        model.system.notifications_allowed = Some(false);
        let (_, _, texts) = frame(&ctx, &assets, &model, &mut view, Vec::new());
        assert!(has(&texts, "Open Settings…"));
        assert_eq!(texts.iter().filter(|(text, _)| text == "On").count(), 1, "hooks on; notifications blocked");
        model.system.notifications_allowed = None;
        model.prefs.notifications = false;
        let (_, _, texts) = frame(&ctx, &assets, &model, &mut view, Vec::new());
        assert!(has(&texts, "Turn On"));
        // Every row, including the long hook detail, ends above the footer (640 - 20 - 32 - 20).
        let rows = texts.iter().filter(|(text, _)| text != "Back" && text != "Continue");
        assert!(rows.clone().all(|(_, rect)| rect.bottom() <= 568.0), "{texts:?}");
        assert!(rows.clone().count() > 8);
    }

    #[test]
    fn done_tips_name_this_platforms_shortcut_edge_and_tray() {
        let (ctx, assets) = setup();
        let mut model = Model::default();
        model.caps.global_hotkeys = true;
        model.prefs.edge = crate::model::Edge::Left;
        let mut view = UiState { welcome_step: 2, ..Default::default() };
        let (_, _, texts) = frame(&ctx, &assets, &model, &mut view, Vec::new());
        let joined: String = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("on the left edge"));
        let stack = joined.find("Hover it and click the stack to see who's waiting.").expect("the stack tip");
        let ask = joined.find("Click the lantern to Ask Lantern: \"which agent needs me first?\"").expect("the Ask Lantern tip");
        assert!(stack < ask, "the stack tip comes first");
        assert!(!joined.contains("bottom of that panel"));
        assert!(joined.contains("press Ctrl+Alt+P, drag a box"));
        assert!(joined.contains("Right-click the lantern for Settings and more."));
        model.caps.tray = true;
        model.caps.global_hotkeys = false;
        let (_, _, texts) = frame(&ctx, &assets, &model, &mut view, Vec::new());
        let joined: String = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("press the dotted-box button"));
        assert!(joined.contains("your system tray"));
    }

    #[test]
    fn blurred_disks_match_a_gaussian_blur_of_a_circle() {
        // σ = 8 on a radius-16 disk: 1 - e^-2 ≈ 0.865 at the centre; a little under half at the
        // rim (a curved edge loses more than a straight one), fading out by 3σ past it.
        let profile = blur_profile(16.0, 8.0);
        assert!((profile[0] - 0.865).abs() < 0.02, "{}", profile[0]);
        assert!((coverage(16.0, 8.0, 16.0) - 0.40).abs() < 0.03, "{}", coverage(16.0, 8.0, 16.0));
        assert!(profile[RINGS] < 0.02);
        // The same shape at any size: only radius / sigma matters.
        let big = blur_profile(32.0, 16.0);
        assert!(profile.iter().zip(big).all(|(a, b)| (a - b).abs() < 1e-3));
    }
}
