//! Settings and Welcome (Swift `SettingsView`/`WelcomeView` windows): normal titled, closable
//! windows, centred on the screen the pill is on, at the Mac's fixed sizes.
use super::Lantern;
use crate::ui::{self, Ctx, Env};
use eframe::egui::{self, Rect, ViewportBuilder, ViewportId, vec2};
use std::time::{Duration, Instant};

pub const SETTINGS_SIZE: egui::Vec2 = vec2(520.0, 480.0);
pub const WELCOME_SIZE: egui::Vec2 = vec2(540.0, 560.0);
/// On a first launch, Welcome waits this long so the engine can find agents to show.
const WELCOME_DELAY: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct State {
    pub settings: bool,
    pub welcome: bool,
    /// When Welcome opens by itself (first launch).
    pub welcome_at: Option<Instant>,
    /// Dialogs whose fixed size has been given to the window manager.
    sized: std::collections::HashSet<String>,
}

impl Lantern {
    pub(super) fn first_launch(&mut self) {
        if !self.model.prefs.welcomed {
            self.windows.dialogs.welcome_at = Some(Instant::now() + WELCOME_DELAY);
        }
    }

    /// Opens Welcome on its first step, with fresh hook states (Swift `.task { hooks.refresh() }`).
    pub(super) fn show_welcome(&mut self) {
        if !self.windows.dialogs.welcome {
            super::trace!("welcome: shown");
            self.view.welcome_step = 0;
            self.refresh_hooks();
            self.refresh_notifications();
        }
        self.windows.dialogs.welcome = true;
    }

    pub(super) fn finish_welcome(&mut self) {
        super::trace!("welcome: finished on step {}", self.view.welcome_step + 1);
        self.windows.dialogs.welcome = false;
        self.model.prefs.welcomed = true;
        self.save_prefs();
        // Expand the pill for a few seconds so people can find it.
        self.windows.flash();
    }

    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        if self.windows.dialogs.welcome_at.is_some_and(|at| Instant::now() >= at) {
            self.windows.dialogs.welcome_at = None;
            self.show_welcome();
        }
        let center = self.windows.work_areas().first().map_or(egui::pos2(960.0, 540.0), Rect::center);
        if self.windows.dialogs.settings {
            let open = self.dialog(ctx, "lantern-settings", "October Lantern Settings", SETTINGS_SIZE, center, ui::render_settings);
            self.windows.dialogs.settings = open;
            if !open {
                self.windows.dialogs.sized.remove("lantern-settings");
            }
        }
        if self.windows.dialogs.welcome {
            let open = self.dialog(ctx, "lantern-welcome", "Welcome to October Lantern", WELCOME_SIZE, center, ui::render_welcome);
            if !open {
                self.windows.dialogs.sized.remove("lantern-welcome");
                // Closing the window counts as finishing it, like the Mac.
                self.finish_welcome();
            }
        }
    }

    /// Shows one dialog window; false once the person closed it.
    fn dialog(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        title: &str,
        size: egui::Vec2,
        center: egui::Pos2,
        render: fn(&mut egui::Ui, &mut Ctx) -> f32,
    ) -> bool {
        let builder = ViewportBuilder::default()
            .with_icon(crate::ui::assets::window_icon())
            .with_title(title)
            .with_inner_size(size)
            .with_position(center - size / 2.0)
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false);
        let env = Env { now_ms: Self::now_ms(), reduce_motion: false, home: crate::settings::home().to_string_lossy().into_owned() };
        let (model, inputs, view, out) = (&self.model, &mut self.inputs, &mut self.view, &mut self.actions);
        let assets = self.assets.as_ref().expect("assets load before the first frame");
        let mut open = true;
        // A fixed-size window: winit pins min/max to a size from before the builder applied, so
        // give the window manager the real one once the window exists.
        let size_once =
            !self.windows.dialogs.sized.contains(id) && ctx.input(|input| input.raw.viewports.contains_key(&ViewportId::from_hash_of(id)));
        if size_once {
            self.windows.dialogs.sized.insert(id.to_string());
        }
        ctx.show_viewport_immediate(ViewportId::from_hash_of(id), builder, |ui, _| {
            if size_once {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
            }
            if ui.input(|input| input.viewport().close_requested()) {
                open = false;
            }
            let mut cx = Ctx { model, inputs, view, out, assets, env: &env };
            egui::CentralPanel::default().frame(egui::Frame::new().fill(ui::theme::black(1.0))).show(ui, |ui| {
                render(ui, &mut cx);
            });
        });
        open
    }
}
