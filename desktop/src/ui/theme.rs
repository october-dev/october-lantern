//! Tokens transcribed from `Components.swift`, not the old desktop theme.
//! Typography uses bundled OFL fonts (see fonts/README.md), with locally installed Segoe UI
//! Variable preferred by the Windows app. Fixtures always use the bundled set. SF's proprietary
//! outlines/metrics are not claimed to be reproduced.
//! Inter 4.1: rsms/inter v4.1 release, SIL OFL 1.1 (`fonts/LICENSE.txt`).
//! Nunito Bold: google/fonts commit 8b0a1d0f5983c89bc2b93f1b5fb55f9e252744b5,
//! weight 700 instantiation, SIL OFL 1.1 (`fonts/Nunito-OFL.txt`).
//! JetBrains Mono 2.304: JetBrains/JetBrainsMono commit cd5227bd1f61dff3bbd6c814ceaf7ffd95e947d9,
//! SIL OFL 1.1 (`fonts/JetBrainsMono-OFL.txt`). Exact upstream URLs are in `fonts/README.md`.
use crate::model::AgentState;
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};

pub const AMBER: Color32 = Color32::from_rgb(250, 184, 69);
pub const GREEN: Color32 = Color32::from_rgb(92, 204, 130);
pub const RED: Color32 = Color32::from_rgb(237, 92, 92);
/// Native dark-mode link blue; links are not amber app controls (ref-4).
pub const LINK: Color32 = Color32::from_rgb(10, 132, 255);
pub const INK: Color32 = Color32::WHITE;
pub const MUTED: Color32 = Color32::from_rgba_premultiplied(140, 140, 140, 140);
pub const FAINT: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);
pub const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(26, 26, 26, 26);
pub const STROKE: Color32 = Color32::from_rgba_premultiplied(36, 36, 36, 36);
pub const PANEL_WIDTH: f32 = 380.0;
pub const POINT_WIDTH: f32 = 400.0;
pub const PANEL_RADIUS: u8 = 22;
pub const POINT_RADIUS: u8 = 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Weight {
    #[default]
    Regular,
    Medium,
    Semibold,
    Bold,
    Rounded,
    Mono,
    MonoBold,
}

pub fn font(size: f32, weight: Weight) -> FontId {
    let name = match weight {
        Weight::Regular => "Inter Regular",
        Weight::Medium => "Inter Medium",
        Weight::Semibold => "Inter Semibold",
        Weight::Bold => "Inter Bold",
        Weight::Rounded => "Nunito Bold",
        Weight::Mono => "JetBrains Mono",
        Weight::MonoBold => "JetBrains Mono Bold",
    };
    FontId::new(size, FontFamily::Name(name.into()))
}

pub fn white(alpha: f32) -> Color32 {
    Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

pub fn black(alpha: f32) -> Color32 {
    Color32::from_black_alpha((alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Sets alpha, rather than multiplying the existing alpha (Swift `.opacity` on an opaque token).
pub fn opacity(color: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

pub fn state_color(state: AgentState) -> Color32 {
    match state {
        AgentState::NeedsInput | AgentState::Waiting => AMBER,
        AgentState::Working => GREEN,
        AgentState::Idle | AgentState::Unknown => white(0.35),
    }
}

pub fn state_label(state: AgentState) -> &'static str {
    state.label()
}

/// Call once when creating an egui context, before measuring any surface.
/// Deterministic bundled fonts for fixtures and tests.
pub fn install(ctx: &egui::Context) {
    install_with(ctx, None);
}

/// Prefer the Windows UI face when available; no system font is copied or redistributed.
pub fn install_native(ctx: &egui::Context) {
    #[cfg(windows)]
    let system_font =
        std::env::var_os("WINDIR").and_then(|root| std::fs::read(std::path::PathBuf::from(root).join("Fonts").join("SegUIVar.ttf")).ok());
    #[cfg(not(windows))]
    let system_font = None;
    install_with(ctx, system_font);
}

fn install_with(ctx: &egui::Context, system_font: Option<Vec<u8>>) {
    let mut fonts = FontDefinitions::empty();
    for (name, bytes) in [
        ("Inter Regular", include_bytes!("fonts/Inter-Regular.ttf").as_slice()),
        ("Inter Medium", include_bytes!("fonts/Inter-Medium.ttf").as_slice()),
        ("Inter Semibold", include_bytes!("fonts/Inter-SemiBold.ttf").as_slice()),
        ("Inter Bold", include_bytes!("fonts/Inter-Bold.ttf").as_slice()),
        ("Nunito Bold", include_bytes!("fonts/Nunito-Bold.ttf").as_slice()),
        ("JetBrains Mono", include_bytes!("fonts/JetBrainsMono-Regular.ttf").as_slice()),
        ("JetBrains Mono Bold", include_bytes!("fonts/JetBrainsMono-Bold.ttf").as_slice()),
    ] {
        fonts.font_data.insert(name.into(), FontData::from_static(bytes).into());
        fonts.families.insert(FontFamily::Name(name.into()), vec![name.into(), "Inter Regular".into()]);
    }
    fonts.families.insert(FontFamily::Proportional, vec!["Inter Regular".into()]);
    fonts.families.insert(FontFamily::Monospace, vec!["JetBrains Mono".into(), "Inter Regular".into()]);
    if let Some(bytes) = system_font {
        let face = FontData::from_owned(bytes);
        // Reject invalid/static files rather than silently using Regular for every weight.
        let axes = face.variation_axes();
        if axes.iter().any(|axis| axis.tag.to_be_bytes() == *b"wght" && axis.range.min <= 400.0 && axis.range.max >= 700.0) {
            for (alias, weight) in [("Inter Regular", 400.0), ("Inter Medium", 500.0), ("Inter Semibold", 600.0), ("Inter Bold", 700.0)] {
                let name = format!("Segoe UI Variable {weight}");
                let mut data = face.clone();
                data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight), (b"opsz", 12.0)]);
                fonts.font_data.insert(name.clone(), data.into());
                fonts.families.get_mut(&FontFamily::Name(alias.into())).unwrap().insert(0, name.clone());
                if weight == 400.0 {
                    fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, name);
                }
            }
        }
    }
    ctx.set_fonts(fonts);
    let mut style = egui::Style { visuals: egui::Visuals::dark(), ..Default::default() };
    style.visuals.override_text_color = Some(INK);
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.window_fill = Color32::from_gray(28);
    style.visuals.window_stroke = Stroke::new(1.0, HAIRLINE);
    style.visuals.selection.bg_fill = opacity(AMBER, 0.30);
    style.visuals.selection.stroke = Stroke::new(1.0, INK);
    style.visuals.hyperlink_color = AMBER;
    style.visuals.widgets.inactive.bg_fill = FAINT;
    style.visuals.widgets.inactive.weak_bg_fill = FAINT;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, HAIRLINE);
    style.visuals.widgets.hovered.bg_fill = white(0.12);
    style.visuals.widgets.hovered.weak_bg_fill = white(0.12);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, STROKE);
    style.visuals.widgets.active.bg_fill = opacity(AMBER, 0.20);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, opacity(AMBER, 0.5));
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    style.visuals.text_cursor.stroke = Stroke::new(1.0, AMBER);
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    style.spacing.interact_size = egui::vec2(22.0, 22.0);
    style.spacing.window_margin = egui::Margin::ZERO;
    style.spacing.scroll.bar_width = 4.0;
    style.text_styles.insert(TextStyle::Body, font(12.5, Weight::Regular));
    style.text_styles.insert(TextStyle::Button, font(12.0, Weight::Medium));
    style.text_styles.insert(TextStyle::Small, font(11.0, Weight::Regular));
    style.text_styles.insert(TextStyle::Monospace, font(11.5, Weight::Mono));
    style.text_styles.insert(TextStyle::Heading, font(17.0, Weight::Bold));
    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Dark);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_system_fonts_keep_every_bundled_family() {
        let definitions = |bytes| {
            let ctx = egui::Context::default();
            install_with(&ctx, bytes);
            let mut fonts = None;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                fonts = Some(ui.fonts(|view| view.definitions().clone()));
            });
            output.textures_delta.clear();
            fonts.unwrap()
        };
        let bundled = definitions(None);
        for bytes in [b"invalid font".to_vec(), include_bytes!("fonts/Inter-Regular.ttf").to_vec()] {
            let fallback = definitions(Some(bytes));
            assert_eq!(fallback.families, bundled.families);
            assert_eq!(fallback.font_data, bundled.font_data);
        }
    }

    #[test]
    fn swift_colors_round_at_the_srgba_boundary() {
        assert_eq!(AMBER.to_array(), [250, 184, 69, 255]);
        assert_eq!(GREEN.to_array(), [92, 204, 130, 255]);
        assert_eq!(RED.to_array(), [237, 92, 92, 255]);
        assert_eq!(state_color(AgentState::NeedsInput), state_color(AgentState::Waiting));
        assert_eq!(state_label(AgentState::Waiting), "Your turn");
    }
}
