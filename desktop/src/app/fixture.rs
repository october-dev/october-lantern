//! `--fixture FILE.json --out FILE.png [--scale N]`: renders one surface from a saved model, with
//! no engine, network, agents or side effects (actions are collected and dropped; nothing counts
//! down or captures). Same render paths as the app, so goldens are honest (CONTRACT §10).
//!
//! ```json
//! {"surface": "panel", "maxHeight": 1032, "nowMs": 1790000000000, "home": "/home/me",
//!  "model": {...model::Model...}, "inputs": {...model::Inputs...}}
//! ```
use crate::model::{Inputs, Model};
use crate::ui::{self, Assets, Ctx, Env, UiState};
use anyhow::{Context, Result};
use eframe::egui;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Surface {
    #[default]
    Panel,
    PointAsk,
    Welcome,
    Settings,
}

impl Surface {
    /// Width and the most height the surface may use (logical points).
    fn frame(self, max_height: f32) -> egui::Vec2 {
        match self {
            Self::Panel => egui::vec2(380.0, max_height),
            Self::PointAsk => egui::vec2(400.0, max_height),
            Self::Welcome => egui::vec2(540.0, 640.0),
            Self::Settings => egui::vec2(520.0, 480.0),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Fixture {
    pub surface: Surface,
    /// The work area's height − 16 on the screen being mimicked.
    pub max_height: Option<f32>,
    pub now_ms: f64,
    pub home: String,
    pub reduce_motion: bool,
    pub model: Model,
    pub inputs: Inputs,
    /// View-only state (welcome step, settings tab, expanded cards…), so every state renders
    /// through the real path.
    pub view: UiState,
}

impl Fixture {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

struct Render {
    fixture: Fixture,
    view: UiState,
    assets: Option<Assets>,
    out: PathBuf,
    frames: u32,
    height: f32,
    requested: bool,
    result: std::sync::Arc<std::sync::Mutex<Option<Result<()>>>>,
}

impl eframe::App for Render {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let assets = self.assets.get_or_insert_with(|| Assets::load(&ctx));
        let max_height = self.fixture.max_height.unwrap_or(1032.0);
        let env = Env { now_ms: self.fixture.now_ms, reduce_motion: self.fixture.reduce_motion, home: self.fixture.home.clone() };
        let mut actions = Vec::new();
        let mut cx = Ctx {
            model: &self.fixture.model,
            inputs: &mut self.fixture.inputs,
            view: &mut self.view,
            out: &mut actions,
            assets,
            env: &env,
        };
        let surface = self.fixture.surface;
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            self.height = match surface {
                Surface::Panel => ui::render_panel(ui, &mut cx, max_height),
                Surface::PointAsk => ui::render_point_ask(ui, &mut cx, max_height),
                Surface::Welcome => ui::render_welcome(ui, &mut cx),
                Surface::Settings => ui::render_settings(ui, &mut cx),
            };
        });
        // Let layout and textures settle, then capture.
        self.frames += 1;
        if self.frames == 4 && !self.requested {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        for event in ui.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let result = save(&image, self.height * ctx.pixels_per_point(), &self.out);
                *self.result.lock().unwrap() = Some(result);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ctx.request_repaint();
    }
}

/// Saves the top `height` physical pixels (the measured surface), keeping transparency.
fn save(image: &egui::ColorImage, height: f32, out: &Path) -> Result<()> {
    let [width, full] = image.size;
    let rows = (height.ceil() as usize).clamp(1, full);
    let mut rgba = Vec::with_capacity(width * rows * 4);
    for pixel in &image.pixels[..width * rows] {
        rgba.extend_from_slice(&pixel.to_srgba_unmultiplied());
    }
    let buffer = image::RgbaImage::from_raw(width as u32, rows as u32, rgba).context("screenshot size")?;
    buffer.save(out).with_context(|| format!("writing {}", out.display()))
}

/// Renders `fixture` to `out` at `scale` (1 or 2), then returns.
pub fn run(fixture: &Path, out: PathBuf, scale: f32) -> Result<()> {
    let mut fixture = Fixture::load(fixture)?;
    let size = fixture.surface.frame(fixture.max_height.unwrap_or(1032.0)) * scale;
    let result = std::sync::Arc::new(std::sync::Mutex::new(None));
    let view = std::mem::take(&mut fixture.view);
    let app = Render { fixture, view, assets: None, out, frames: 0, height: 0.0, requested: false, result: result.clone() };
    #[allow(unused_mut)]
    let mut viewport = egui::ViewportBuilder::default()
        .with_icon(crate::ui::assets::window_icon())
        .with_title("Lantern fixture")
        .with_inner_size(size)
        .with_position([0.0, 0.0])
        .with_resizable(false)
        .with_decorations(false)
        .with_transparent(true)
        .with_clamp_size_to_monitor_size(false);
    // Wayland compositors (and window managers for normal windows) clamp a window to the screen,
    // which would crop tall @2x surfaces. An X11 dock window may be taller than the screen.
    #[cfg(target_os = "linux")]
    {
        viewport = viewport.with_window_type(egui::X11WindowType::Dock);
    }
    let options = eframe::NativeOptions {
        viewport,
        #[cfg(target_os = "linux")]
        event_loop_builder: crate::os::shell::x11_available().then(|| -> eframe::EventLoopBuilderHook {
            Box::new(|builder| {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_x11();
            })
        }),
        ..Default::default()
    };
    eframe::run_native(
        "Lantern fixture",
        options,
        Box::new(move |cc| {
            ui::theme::install(&cc.egui_ctx);
            cc.egui_ctx.set_zoom_factor(scale);
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    let outcome = result.lock().unwrap().take();
    outcome.unwrap_or_else(|| Err(anyhow::anyhow!("the window closed before the fixture was captured")))
}
