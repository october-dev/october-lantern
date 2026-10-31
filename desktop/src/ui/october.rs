//! SessionViews.swift's optional October account/Desktop panel.
use super::{
    Ctx, cards, components as c, sessions,
    theme::{self as t, Weight},
};
use eframe::egui::{self, Margin, Ui};

pub fn render(ui: &mut Ui, cx: &mut Ctx, max_height: f32) {
    sessions::panel_title(ui, cx, "October");
    egui::ScrollArea::vertical()
        .id_salt("october-panel")
        .max_height((max_height - 60.0).max(200.0))
        .min_scrolled_height(0.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 0, bottom: 16 }).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 14.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(48.0), egui::Sense::hover());
                    egui::Image::new(&cx.assets.logo).paint_at(ui, rect);
                    let badge = egui::Rect::from_min_size(rect.right_bottom() - egui::vec2(17.0, 17.0), egui::Vec2::splat(18.0));
                    egui::Image::new(&cx.assets.october).corner_radius(4).paint_at(ui, badge);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        c::label(ui, "Keep your agents close.", 19.0, Weight::Semibold, t::INK);
                        c::label(ui, "Ask on your computer. Reply from your phone.", 11.5, Weight::Regular, t::MUTED);
                    });
                });
                cards::account::render(ui, cx);
                super::phone::render(ui, cx);
                cards::desktop::render(ui, cx, true);
            });
        });
}
