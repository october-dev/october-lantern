//! A guide owns this observer. Its dedicated thread takes one display picture, observes clicks
//! without consuming them, and watches window geometry while October answers and the cue shows.
//! Only capture/run IDs and change notices leave the observer; window metadata stays local.
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::Desktop;
#[cfg(target_os = "linux")]
mod x11;
#[cfg(target_os = "linux")]
use x11::Desktop;

use super::Event;
use crate::show_overlay::{self as geometry, Display};
use anyhow::Result;
use eframe::egui::{ColorImage, Rect};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};
use std::time::{Duration, Instant};

pub struct Capture {
    pub jpeg: Vec<u8>,
    pub picture: ColorImage,
    pub size: [u32; 2],
    pub display: Display,
    pub observer: Option<Observer>,
    pub note: Option<String>,
    pub reduce_motion: bool,
    pub stable: bool,
}

pub enum Change {
    Clicked,
    Screen,
    Unavailable,
}

#[derive(Clone)]
pub struct Cue {
    pub target: Rect,
    pub expires_at: f64,
    pub title: String,
}

pub struct Observer {
    stop: Arc<AtomicBool>,
    cue: Arc<Mutex<Option<Cue>>>,
    motion: Arc<AtomicBool>,
}

impl Observer {
    pub fn point(&self, cue: Option<Cue>) {
        *self.cue.lock().unwrap_or_else(|e| e.into_inner()) = cue;
    }

    pub fn reduce_motion(&self) -> bool {
        self.motion.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn fixture() -> Self {
        Self { stop: Arc::new(AtomicBool::new(false)), cue: Arc::new(Mutex::new(None)), motion: Arc::new(AtomicBool::new(true)) }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// Called only after all of Lantern's surfaces have cleared. No engine is started here.
pub fn capture(run: String, capture: String, previous: Option<Display>, scale: f32, forced_note: Option<String>, tx: Sender<Event>) {
    std::thread::spawn(move || {
        let mut desktop = match Desktop::connect() {
            Ok(desktop) => desktop,
            Err(_) => {
                // Wayland's portal can supply a picture, but not portable global geometry.
                let result = fallback().map_err(|e| e.to_string());
                let _ = tx.send(Event::ShowCaptured { run, capture, result });
                return;
            }
        };
        let prepared = (|| -> Result<_> {
            let display = desktop.display(previous, scale)?;
            let reduce_motion = Desktop::reduce_motion();
            let baseline = desktop.screen(display)?;
            let image = desktop.capture(display)?;
            // Encoding can take time on a large display. Finish it before installing Windows'
            // timing-sensitive hook, then revalidate the picture before asking the AI.
            let mut captured = encode(image, display)?;
            let note = forced_note.or_else(|| desktop.observe().err().map(|_| "On-screen pointing isn't available on this desktop. The place is marked on the captured picture; use Look again after changing the screen.".into()));
            let (stop, cue) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(None)));
            let motion = Arc::new(AtomicBool::new(reduce_motion));
            let observer = note.is_none().then(|| Observer { stop: stop.clone(), cue: cue.clone(), motion: motion.clone() });
            if observer.is_some() {
                let stop = stop.clone();
                // Preference reads never block the click-hook/window-watching thread.
                std::thread::spawn(move || {
                    while !stop.load(Ordering::SeqCst) {
                        for _ in 0..10 {
                            if stop.load(Ordering::SeqCst) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(200));
                        }
                        motion.store(Desktop::reduce_motion(), Ordering::SeqCst);
                    }
                });
            }
            captured.observer = observer;
            captured.reduce_motion = reduce_motion;
            captured.stable = !baseline.changed(&desktop.screen(display)?);
            captured.note = note;
            Ok((desktop, baseline, stop, cue, captured))
        })();
        let (mut desktop, baseline, stop, cue, captured) = match prepared {
            Ok(v) => v,
            Err(error) => {
                // A removed monitor or failed capture must not silently switch displays.
                let _ = tx.send(Event::ShowCaptured { run, capture, result: Err(error.to_string()) });
                return;
            }
        };
        let watching = captured.observer.is_some();
        if tx.send(Event::ShowCaptured { run: run.clone(), capture: capture.clone(), result: Ok(captured) }).is_err() || !watching {
            return;
        }
        let mut checked = Instant::now();
        let send = |change| {
            let _ = tx.send(Event::ShowScreen { run: run.clone(), capture: capture.clone(), change });
        };
        while !stop.load(Ordering::SeqCst) {
            let current = cue.lock().unwrap_or_else(|e| e.into_inner()).clone();
            match desktop.clicks() {
                Ok(clicks) => {
                    if let Some(current) = &current
                        && now_ms() < current.expires_at
                        && clicks.into_iter().any(|at| geometry::hit(baseline.display, current.target, at))
                    {
                        // Hide before notifying the UI. Do not race the one-second Next timer
                        // with the window change the person's click caused.
                        hide(&current.title);
                        send(Change::Clicked);
                        return;
                    }
                }
                Err(_) => {
                    if let Some(current) = &current {
                        hide(&current.title);
                    }
                    send(Change::Unavailable);
                    return;
                }
            }
            if checked.elapsed() >= Duration::from_millis(125) {
                checked = Instant::now();
                match desktop.screen(baseline.display) {
                    Ok(now) if !baseline.changed(&now) => {}
                    result => {
                        if let Some(current) = &current {
                            hide(&current.title);
                        }
                        send(if result.is_ok() { Change::Screen } else { Change::Unavailable });
                        return;
                    }
                }
            }
            if let Some(current) = &current
                && now_ms() >= current.expires_at
            {
                hide(&current.title);
                return;
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    });
}

fn encode(image: image::RgbaImage, display: Display) -> Result<Capture> {
    anyhow::ensure!(image.width() as u64 * image.height() as u64 <= 64_000_000, "The display is too large to capture safely");
    let image = image::DynamicImage::ImageRgba8(image).resize(1600, 1600, image::imageops::FilterType::Triangle).to_rgb8();
    let (w, h) = image.dimensions();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80).encode_image(&image)?;
    anyhow::ensure!(jpeg.len() <= 2_500_000, "The screen picture is too large");
    Ok(Capture {
        jpeg,
        picture: ColorImage::from_rgb([w as usize, h as usize], image.as_raw()),
        size: [w, h],
        display,
        observer: None,
        note: None,
        reduce_motion: true,
        stable: true,
    })
}

fn fallback() -> Result<Capture> {
    let path = super::capture_screen()?;
    let image = image::open(&path);
    let _ = std::fs::remove_file(path);
    let image = image?.to_rgba8();
    let display = Display {
        bounds: Rect::from_min_size(eframe::egui::Pos2::ZERO, eframe::egui::vec2(image.width() as f32, image.height() as f32)),
        scale: 1.0,
    };
    let mut result = encode(image, display)?;
    result.note = Some("On-screen pointing isn't available on this desktop. The place is marked on the captured picture; use Look again after changing the screen.".into());
    Ok(result)
}

pub fn place(title: &str, display: Display) -> bool {
    Desktop::place(title, display)
}

pub fn hide(title: &str) {
    Desktop::hide(title);
}
