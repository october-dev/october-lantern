//! Artwork, decoded and uploaded once at startup; read-only afterwards (CONTRACT §11.8).
//! Artwork stays sourced from macos/Resources; there are no OS-specific copies.
use crate::model::AgentKind;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

pub struct Assets {
    /// The lantern mascot (repo root logo.png).
    pub logo: TextureHandle,
    /// Swift's idle `.saturation(0.55)`; apply `.opacity(0.8)` when drawing.
    pub logo_idle: TextureHandle,
    /// The lantern's silhouette, blurred: Welcome's `.shadow(color: amber.opacity(0.5), radius: 24)`.
    /// The 96pt logo with 72pt (3 sigma) around it, 240 x 240; tint it with the shadow colour.
    pub logo_shadow: TextureHandle,
    pub october: TextureHandle,
    pub google: TextureHandle,
    pub github: TextureHandle,
    /// Authored outline, not an extracted SF Symbol or a redistributed Apple font.
    pub apple: TextureHandle,
    harnesses: HashMap<&'static str, TextureHandle>,
}

impl Assets {
    pub fn load(ctx: &egui::Context) -> Self {
        let mut harnesses = HashMap::new();
        macro_rules! harness {
            ($($kind:literal),+ $(,)?) => {$({
                harnesses.insert($kind, texture(ctx, concat!("harness/", $kind),
                    include_bytes!(concat!("../../../macos/Resources/harness/", $kind, ".png"))));
            })+};
        }
        harness!(
            "claude", "codex", "gemini", "opencode", "pi", "october", "grok", "cursor", "copilot", "qwen", "goose", "aider", "amp", "kimi",
            "droid", "crush", "auggie", "muse"
        );
        let logo_image = decode("logo", include_bytes!("../../../logo.png"));
        let logo = upload(ctx, "logo", &logo_image);
        let logo_shadow = upload(ctx, "logo/shadow", &shadow(&logo_image));
        let mut idle = logo_image;
        for pixel in idle.pixels_mut() {
            let [r, g, b, _] = pixel.0;
            let gray = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
            for channel in &mut pixel.0[..3] {
                *channel = (gray + (*channel as f32 - gray) * 0.55).round() as u8;
            }
        }
        Self {
            logo,
            logo_idle: upload(ctx, "logo/idle", &idle),
            logo_shadow,
            october: texture(ctx, "october", include_bytes!("../../../macos/Resources/october.png")),
            google: svg_texture(ctx, "brands/google", include_bytes!("../../../macos/Resources/brands/google.svg")),
            github: svg_texture(ctx, "brands/github", include_bytes!("../../../macos/Resources/brands/github.svg")),
            apple: svg_texture(ctx, "brands/apple-authored", br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path fill="#fff" d="M15.7 1.5c.3 2.1-1.2 4.2-3.4 4.5-.3-2 1.3-4 3.4-4.5ZM18.4 12.5c0-2 1.1-3.2 2.1-3.8-1.1-1.6-2.6-2.1-3.8-2.1-1.6-.1-2.9 1-3.9 1-1.1 0-2.4-1-3.9-1-2.9.1-4.8 2.5-4.8 5.8 0 4 2.8 9.3 5 9.3 1.2 0 1.6-.8 3.4-.8 1.7 0 2.1.8 3.4.8 1.8 0 3.7-3.1 4.5-5.1-1.2-.6-2-2.1-2-4.1Z"/></svg>"##),
            harnesses,
        }
    }

    pub fn harness(&self, kind: &AgentKind) -> Option<&TextureHandle> {
        self.harnesses.get(kind.as_str())
    }
}

/// The lantern as every window's icon: the title bar, taskbar and Alt-Tab (Windows), and the
/// window list (Linux). Decoded once; 64 px reads well from 16 px up to high-DPI taskbars.
pub fn window_icon() -> Arc<egui::IconData> {
    static ICON: OnceLock<Arc<egui::IconData>> = OnceLock::new();
    ICON.get_or_init(|| {
        let logo = decode("logo", include_bytes!("../../../logo.png"));
        let icon = image::imageops::resize(&logo, WINDOW_ICON, WINDOW_ICON, image::imageops::FilterType::Lanczos3);
        Arc::new(egui::IconData { rgba: icon.into_raw(), width: WINDOW_ICON, height: WINDOW_ICON })
    })
    .clone()
}

const WINDOW_ICON: u32 = 64;

/// Gaussian approximation of Welcome's `shadow(radius: 24)`, using sigma 24 at 1x.
/// A 96pt logo is 96 px, padded 72 px (3 sigma) so its visible falloff has room to fade.
/// The kernel is deterministic across our platforms; exact SwiftUI filter equivalence is unproven.
fn shadow(logo: &image::RgbaImage) -> image::RgbaImage {
    const LOGO: u32 = 96;
    const PAD: u32 = 72;
    let small = image::imageops::resize(logo, LOGO, LOGO, image::imageops::FilterType::Triangle);
    // White everywhere, so blurring the colour channels never mixes in black at the edges.
    let mut silhouette = image::RgbaImage::from_pixel(LOGO + 2 * PAD, LOGO + 2 * PAD, image::Rgba([255, 255, 255, 0]));
    for (x, y, pixel) in small.enumerate_pixels() {
        silhouette.put_pixel(x + PAD, y + PAD, image::Rgba([255, 255, 255, pixel.0[3]]));
    }
    image::imageops::blur(&silhouette, 24.0)
}

/// Canonical SVG brand marks rasterized once at 4x their 13pt display size. SVG parsing never
/// happens in a render pass; the same pixels are used on both platforms and by fixtures.
fn svg_texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> TextureHandle {
    let tree =
        resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).unwrap_or_else(|error| panic!("bundled SVG {name}: {error}"));
    let mut pixmap = resvg::tiny_skia::Pixmap::new(52, 52).expect("brand pixmap");
    let scale = (52.0 / tree.size().width()).min(52.0 / tree.size().height());
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale)
        .post_translate((52.0 - tree.size().width() * scale) / 2.0, (52.0 - tree.size().height() * scale) / 2.0);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia's RGBA is premultiplied. Undo that before egui performs its own conversion.
    let mut rgba = pixmap.take();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = pixel[3] as u32;
        for channel in &mut pixel[..3] {
            *channel = (*channel as u32 * 255 + alpha / 2).checked_div(alpha).unwrap_or(0).min(255) as u8;
        }
    }
    ctx.load_texture(name, ColorImage::from_rgba_unmultiplied([52, 52], &rgba), TextureOptions::LINEAR)
}

/// Decodes bundled PNG/JPEG bytes into a texture. Bundled art always decodes; a failure is a build mistake.
pub fn texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> TextureHandle {
    upload(ctx, name, &decode(name, bytes))
}

fn decode(name: &str, bytes: &[u8]) -> image::RgbaImage {
    image::load_from_memory(bytes).unwrap_or_else(|error| panic!("bundled image {name}: {error}")).to_rgba8()
}

fn upload(ctx: &egui::Context, name: &str, image: &image::RgbaImage) -> TextureHandle {
    let size = [image.width() as usize, image.height() as usize];
    ctx.load_texture(name, ColorImage::from_rgba_unmultiplied(size, image.as_raw()), TextureOptions::LINEAR)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_window_icon_is_the_square_lantern_with_transparency() {
        let icon = super::window_icon();
        assert_eq!((icon.width, icon.height), (64, 64));
        assert_eq!(icon.rgba.len(), 64 * 64 * 4);
        assert!(icon.rgba.chunks(4).any(|pixel| pixel[3] == 0) && icon.rgba.chunks(4).any(|pixel| pixel[3] == 255));
        assert!(std::sync::Arc::ptr_eq(&icon, &super::window_icon()), "decoded once");
    }

    #[test]
    fn the_logo_shadow_is_a_soft_white_silhouette_with_room_to_fade() {
        let logo = super::decode("logo", include_bytes!("../../../logo.png"));
        let shadow = super::shadow(&logo);
        assert_eq!(shadow.dimensions(), (240, 240));
        assert!(shadow.pixels().all(|pixel| pixel.0[..3] == [255, 255, 255]));
        let alpha = |x: u32, y: u32| shadow.get_pixel(x, y).0[3];
        assert!(alpha(120, 120) > 64, "centre {}", alpha(120, 120));
        assert!(alpha(0, 0) < 3 && alpha(239, 120) < 8, "edges {} {}", alpha(0, 0), alpha(239, 120));
    }
}
