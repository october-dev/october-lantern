//! Show me's portable client. Uses the same engine coordinator and server contract as Mac.
//! Windows/Linux show a dated captured picture until live-overlay geometry is qualified.
use super::Lantern;
use crate::{
    model::PanelMode,
    october_ai::{self, Piece},
    os::Event,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
pub(super) struct Guide {
    run: String,
    round: u64,
    capture_id: String,
    question: String,
    jpeg: Option<Vec<u8>>,
    size: [u32; 2],
    instruction: String,
    cancelled: Arc<AtomicBool>,
}

impl Lantern {
    pub(super) fn start_show(&mut self, question: String) {
        if !self.model.prefs.show_me {
            self.show("Show me is turned off in Settings");
            return;
        }
        if self.session.is_none() {
            self.show("Sign in to October to use Show me");
            self.set_panel(Some(PanelMode::October));
            return;
        }
        let question = question.trim().chars().take(4000).collect::<String>();
        if question.is_empty() {
            self.show("Type what you want Lantern to show you");
            return;
        }
        self.stop_show();
        self.guide =
            Guide { run: uuid::Uuid::new_v4().to_string(), question, cancelled: Arc::new(AtomicBool::new(false)), ..Default::default() };
        let _ = self.engine.send(&json!({"type":"show.start","runId":self.guide.run}));
        self.model.guide = Some(json!({"phase":"looking","question":self.guide.question,"step":1}));
        self.set_panel(Some(PanelMode::ShowMe));
    }

    pub(super) fn stop_show(&mut self) {
        self.guide.cancelled.store(true, Ordering::SeqCst);
        if !self.guide.run.is_empty() {
            let _ = self.engine.send(&json!({"type":"show.stop","runId":self.guide.run}));
        }
        self.guide = Guide::default();
        self.model.guide = None;
        self.view.show_picture = None;
    }

    pub(super) fn show_action(&mut self, action: &str) {
        if action == "stop" {
            self.stop_show();
            self.set_panel(None);
            return;
        }
        if self.guide.run.is_empty() {
            return;
        }
        let step = self.model.guide.as_ref().and_then(|v| v["step"].as_u64()).unwrap_or(1);
        let _ = self.engine.send(&json!({"type":format!("show.{action}"),"runId":self.guide.run,"step":step}));
    }

    pub(super) fn show_event(&mut self, event: Value) {
        if event["runId"].as_str() != Some(&self.guide.run) || self.guide.run.is_empty() {
            return;
        }
        match event["type"].as_str() {
            Some("showCapture") => {
                self.guide.jpeg = None;
                self.view.show_picture = None;
                self.guide.cancelled.store(true, Ordering::SeqCst);
                self.guide.cancelled = Arc::new(AtomicBool::new(false));
                self.guide.capture_id = event["captureId"].as_str().unwrap_or_default().to_string();
                let run = format!("{}:{}", self.guide.run, self.guide.capture_id);
                self.windows.hide_for_capture(move |tx| {
                    crate::os::task(tx, crate::os::capture_screen, move |result| Event::Screenshot {
                        owner: format!("__show:{run}"),
                        generation: 0,
                        result,
                    });
                });
                self.model.guide = Some(json!({"question":self.guide.question,"phase":"looking","step":event["step"]}));
            }
            Some("showQuery") => self.ask_show(event),
            Some("showState") => {
                let mut state = event;
                state["question"] = json!(self.guide.question);
                state["instruction"] = json!(self.guide.instruction);
                if matches!(state["phase"].as_str(), Some("ended")) {
                    self.guide.cancelled.store(true, Ordering::SeqCst);
                    self.guide.jpeg = None;
                    self.view.show_picture = None;
                }
                self.model.guide = Some(state);
            }
            _ => {}
        }
    }

    pub(super) fn show_captured(&mut self, capture: &str, result: Result<PathBuf, String>) {
        self.windows.capture_done();
        let (run, capture_id) = capture.split_once(':').unwrap_or((capture, ""));
        let current = run == self.guide.run && capture_id == self.guide.capture_id && !self.guide.cancelled.load(Ordering::SeqCst);
        let path = match result {
            Ok(p) => p,
            Err(e) => {
                if current {
                    self.show_failure(e);
                }
                return;
            }
        };
        if !current {
            let _ = std::fs::remove_file(path);
            return;
        }
        let encoded = (|| -> anyhow::Result<_> {
            let size = image::ImageReader::open(&path)?.into_dimensions()?;
            anyhow::ensure!(size.0 as u64 * size.1 as u64 <= 64_000_000, "The display is too large to capture safely");
            let image = image::open(&path)?.resize(1600, 1600, image::imageops::FilterType::Triangle).to_rgb8();
            let (w, h) = image.dimensions();
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80).encode_image(&image)?;
            anyhow::ensure!(jpeg.len() <= 2_500_000, "The screen picture is too large");
            let picture = eframe::egui::ColorImage::from_rgb([w as usize, h as usize], image.as_raw());
            Ok((jpeg, picture, [w, h]))
        })();
        let _ = std::fs::remove_file(path);
        match encoded {
            Ok((jpeg, picture, size)) => {
                self.guide.jpeg = Some(jpeg);
                self.guide.size = size;
                self.view.show_picture = Some(picture);
                let step = self.model.guide.as_ref().and_then(|v| v["step"].as_u64()).unwrap_or(1);
                let _=self.engine.send(&json!({"type":"show.captured","runId":run,"step":step,"display":{"x":0,"y":0,"w":size[0],"h":size[1]},"imageWidth":size[0],"imageHeight":size[1],"captureId":capture_id,"excludePids":[std::process::id()],"markedOnly":true}));
            }
            Err(e) => self.show_failure(e.to_string()),
        }
    }

    fn show_failure(&mut self, error: String) {
        self.guide.cancelled.store(true, Ordering::SeqCst);
        self.model.guide = Some(json!({"phase":"failed","question":self.guide.question,"message":error}));
        let _ = self.engine.send(&json!({"type":"show.stop","runId":self.guide.run}));
    }

    fn ask_show(&mut self, event: Value) {
        let (Some(jpeg), Some(session)) = (self.guide.jpeg.clone(), self.session.as_ref()) else {
            return;
        };
        let token = session.access.clone();
        let cancelled = self.guide.cancelled.clone();
        if cancelled.load(Ordering::SeqCst) {
            return;
        }
        let run = self.guide.run.clone();
        let round = event["round"].as_u64().unwrap_or(0);
        self.guide.round = round;
        let capture = event["captureId"].as_str().unwrap_or_default().to_string();
        let mut guidance = json!({"roundId":uuid::Uuid::new_v4().to_string(),"step":event["step"],"captureId":capture,"imageSize":self.guide.size,"elements":event["elements"],"shown":event["shown"]});
        if event["goalId"].is_string() {
            guidance["goalId"] = event["goalId"].clone();
        }
        let request = october_ai::Request {
            question: self.guide.question.clone(),
            image: Some(jpeg),
            mode: Some("show".into()),
            guidance: Some(guidance),

            ..Default::default()
        };
        self.guide.instruction.clear();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            if cancelled.load(Ordering::SeqCst) {
                return;
            }
            let mut result = json!({"guidance":null});
            let mut instruction = String::new();
            let response = october_ai::stream(&request, &token, &mut |piece| {
                if cancelled.load(Ordering::SeqCst) {
                    return false;
                }
                match piece {
                    Piece::Text(t) => instruction.push_str(&t),
                    Piece::Guide { guidance, goal_id } => {
                        result["guidance"] = guidance;
                        if let Some(goal) = goal_id {
                            result["goalId"] = json!(goal);
                        }
                    }
                    Piece::Note(_) | Piece::Task { .. } => {}
                }
                true
            });
            result["instruction"] = json!(instruction.chars().take(500).collect::<String>());
            if let Err(error) = response {
                result["error"] = json!(error.to_string());
            }
            if !cancelled.load(Ordering::SeqCst) {
                let _ = tx.send(Event::ShowAnswer { run, round, capture, result });
            }
        });
    }

    pub(super) fn show_answer(&mut self, run: &str, round: u64, capture: &str, result: Value) {
        if run != self.guide.run
            || round != self.guide.round
            || capture != self.guide.capture_id
            || self.guide.cancelled.load(Ordering::SeqCst)
        {
            return;
        }
        self.guide.instruction = result["instruction"].as_str().unwrap_or_default().to_string();
        let _ = self.engine.send(&json!({"type":"show.result","runId":run,"round":round,"captureId":capture,"result":result}));
    }
}
