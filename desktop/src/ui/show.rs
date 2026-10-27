use super::{Ctx, UiAction};
use eframe::egui::{self, Ui};

pub fn render(ui: &mut Ui, cx: &mut Ctx) {
    ui.heading("Show me");
    let Some(state) = &cx.model.guide else {
        ui.label("Start from Ask Lantern or Point & Ask.");
        return;
    };
    ui.label(state["question"].as_str().unwrap_or_default());
    ui.small("Each look sends a picture of the whole display to October. Lantern's windows are left out. It points on this captured picture; it never clicks or types.");
    let phase = state["phase"].as_str().unwrap_or("looking");
    ui.label(match phase {
        "asking" => "Finding the control…",
        "looking" => "Looking at the screen…",
        "notVisible" => "I couldn't locate it in this view.",
        "finished" => "The guide is finished.",
        "failed" => "Couldn't finish this look.",
        "ended" | "stopped" | "limit" | "expired" => "The guide has stopped.",
        _ => "On the captured screen:",
    });
    if let Some(error) = state["message"].as_str() {
        ui.label(error);
    }
    if let Some(instruction) = state["instruction"].as_str() {
        ui.label(instruction);
    }
    if matches!(phase, "showing" | "notVisible")
        && let Some(picture) = &cx.view.show_picture
    {
        let texture = ui.ctx().load_texture("show-me-capture", picture.clone(), egui::TextureOptions::LINEAR);
        let response =
            ui.add(egui::Image::new(&texture).fit_to_exact_size(egui::vec2(
                ui.available_width(),
                ui.available_width() * picture.size[1] as f32 / picture.size[0] as f32,
            )));
        if let Some(a) = state["target"]["image"].as_array().filter(|a| a.len() == 4) {
            let n = |i: usize| a[i].as_f64().unwrap_or(0.0) as f32;
            let scale = response.rect.width() / picture.size[0] as f32;
            let rect = egui::Rect::from_min_size(response.rect.min + egui::vec2(n(0), n(1)) * scale, egui::vec2(n(2), n(3)) * scale);
            ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 181, 55)), egui::StrokeKind::Inside);
            ui.painter().text(
                rect.left_top(),
                egui::Align2::LEFT_BOTTOM,
                format!("Step {}", state["step"].as_u64().unwrap_or(1)),
                egui::FontId::proportional(12.0),
                egui::Color32::WHITE,
            );
        }
        ui.small("Captured screen — use Look again after moving or changing it.");
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(matches!(phase, "showing" | "notVisible" | "waiting"), egui::Button::new("Next")).clicked() {
            cx.emit(UiAction::Guide("next".into()));
        }
        if ui.add_enabled(!matches!(phase, "asking" | "looking"), egui::Button::new("Look again")).clicked() {
            cx.emit(UiAction::Guide("again".into()));
        }
        if ui.button("Stop").clicked() {
            cx.emit(UiAction::Guide("stop".into()));
        }
    });
}
