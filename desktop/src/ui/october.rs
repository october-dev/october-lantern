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
                    ui.add(egui::Image::new(&cx.assets.october).fit_to_exact_size(egui::Vec2::splat(40.0)).corner_radius(10));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        c::label(ui, "October", 15.0, Weight::Semibold, t::INK);
                        c::label(
                            ui,
                            "Optional. Sign in for Ask and Point & Ask answers; connect October Desktop to see its agents here.",
                            11.5,
                            Weight::Regular,
                            t::MUTED,
                        );
                    });
                });
                cards::account::render(ui, cx);
                super::phone::render(ui, cx);
                cards::desktop::render(ui, cx, true);
            });
        });
}
