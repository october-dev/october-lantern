//! Settings and Welcome (Swift `SettingsView`/`WelcomeView` windows): normal titled, closable
//! windows, centred on the screen the pill is on, at the Mac's fixed sizes.
use super::Lantern;
use crate::ui::{self, Ctx, Env};
use eframe::egui::{self, Rect, ViewportBuilder, ViewportId, vec2};
use std::time::{Duration, Instant};

pub const SETTINGS_SIZE: egui::Vec2 = vec2(520.0, 480.0);
pub const WELCOME_SIZE: egui::Vec2 = vec2(540.0, 640.0);
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
        self.show_welcome_at(0);
    }

    /// Opens Welcome on `step` (a first launch: where an unfinished welcome was left).
    fn show_welcome_at(&mut self, step: usize) {
        if !self.windows.dialogs.welcome {
            super::trace!("welcome: shown at step {}", step + 1);
            self.view.welcome_step = step.min(ui::welcome::LAST_STEP);
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
            self.show_welcome_at(self.model.prefs.welcome_step);
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
            self.note_welcome_step();
            if !open {
                self.windows.dialogs.sized.remove("lantern-welcome");
                // Closing the window counts as finishing it, like the Mac.
                self.finish_welcome();
            }
        }
    }

    /// Saves an unfinished welcome's page as it changes (Swift's `welcomeStep`), so quitting
    /// Lantern partway through picks up there next time.
    fn note_welcome_step(&mut self) {
        if !self.model.prefs.welcomed && self.model.prefs.welcome_step != self.view.welcome_step {
            self.model.prefs.welcome_step = self.view.welcome_step;
            self.save_prefs();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unfinished_welcome_picks_up_where_it_was_left() {
        let (engine, _) = crate::sidecar::test_sender();
        let mut lantern = Lantern::new(engine, None);
        lantern.model.prefs.welcome_step = 2;
        lantern.first_launch();
        lantern.windows.dialogs.welcome_at = Some(Instant::now());
        lantern.show_welcome_at(lantern.model.prefs.welcome_step);
        assert_eq!(lantern.view.welcome_step, 2);
        lantern.view.welcome_step = 3;
        lantern.note_welcome_step();
        assert_eq!(lantern.model.prefs.welcome_step, 3);
        // The Welcome Guide (from the menu) always starts at the beginning.
        lantern.windows.dialogs.welcome = false;
        lantern.show_welcome();
        assert_eq!(lantern.view.welcome_step, 0);
        lantern.finish_welcome();
        lantern.view.welcome_step = 1;
        lantern.note_welcome_step();
        assert_eq!(lantern.model.prefs.welcome_step, 3, "a finished welcome isn't resumed");
    }
}
