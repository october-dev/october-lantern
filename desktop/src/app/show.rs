//! Show me's portable client. Uses the same engine coordinator and server contract as Mac.
//! Native Windows/X11 cues are backed by a capture-scoped window/click observer. Wayland and
//! unavailable native capabilities explicitly use the marked-picture card.
use super::Lantern;
use crate::{
    model::Fix,
    model::PanelMode,
    october_ai::{self, Failure, Piece},
    os::Event,
};
use crate::{
    os::show_overlay::{self as native, Capture, Change, Observer},
    show_overlay::Display,
};
use eframe::egui::{self, Rect, ViewportBuilder, ViewportId};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

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
    /// The round being answered, kept to ask it again with October's AI.
    query: Option<Value>,
    /// After "Ask October's AI instead", the rest of this guide uses October's AI.
    own_key_off: bool,
    /// What the last failed answer offers, and whose key answered (shown on the card).
    fix: Option<crate::model::Fix>,
    via: Option<String>,
    display: Option<Display>,
    observer: Option<Observer>,
    ring: Option<Ring>,
    overlay_note: Option<String>,
    forced_note: Option<String>,
    reduce_motion: bool,
    next_at: Option<Instant>,
    ignore_cue: bool,
    capture_step: u64,
    capture_pending: bool,
    /// Native DPI/egui scale for X11; Windows reads the captured monitor's DPI itself.
    scale: f32,
}

struct Ring {
    display: Display,
    target: Rect,
    step: u64,
    expires: f64,
    started: Instant,
    title: String,
    placed: bool,
    placement_check: Instant,
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
        let scale = self.guide.scale;
        self.stop_show();
        self.guide = Guide {
            run: uuid::Uuid::new_v4().to_string(),
            question,
            scale,
            cancelled: Arc::new(AtomicBool::new(false)),
            ..Default::default()
        };
        let _ = self.engine.send(&json!({"type":"show.start","runId":self.guide.run}));
        self.model.guide = Some(json!({"phase":"looking","question":self.guide.question,"step":1}));
        self.set_panel(Some(PanelMode::ShowMe));
    }

    pub(super) fn stop_show(&mut self) {
        self.hide_show_ring();
        if self.guide.capture_pending {
            self.windows.capture_done();
        }
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
        self.hide_show_ring();
        self.guide.next_at = None;
        self.guide.ignore_cue = true;
        // The person can move on before the engine's hide message comes back.
        self.guide.observer = None;
        let step = self.guide.capture_step.max(1);
        let _ = self.engine.send(&json!({"type":format!("show.{action}"),"runId":self.guide.run,"step":step}));
    }

    pub(super) fn show_event(&mut self, event: Value) {
        if event["runId"].as_str() != Some(&self.guide.run) || self.guide.run.is_empty() {
            return;
        }
        match event["type"].as_str() {
            Some("showCapture") => {
                self.hide_show_ring();
                self.guide.observer = None;
                self.guide.ignore_cue = false;
                self.guide.next_at = None;
                self.guide.jpeg = None;
                self.view.show_picture = None;
                self.guide.cancelled.store(true, Ordering::SeqCst);
                self.guide.cancelled = Arc::new(AtomicBool::new(false));
                self.guide.capture_id = event["captureId"].as_str().unwrap_or_default().to_string();
                self.guide.capture_step = event["step"].as_u64().unwrap_or(1);
                self.guide.capture_pending = true;
                let (run, capture) = (self.guide.run.clone(), self.guide.capture_id.clone());
                let (display, scale, note) = (self.guide.display, self.guide.scale.max(1.0), self.guide.forced_note.clone());
                self.windows.hide_for_capture(move |tx| native::capture(run, capture, display, scale, note, tx));
                self.model.guide = Some(json!({"question":self.guide.question,"phase":"looking","step":event["step"]}));
            }
            Some("showQuery") => self.ask_show(event),
            Some("showState") => {
                if event["captureId"].as_str().is_some_and(|id| !id.is_empty() && id != self.guide.capture_id) {
                    return;
                }
                self.hide_show_ring();
                let mut state = event;
                state["question"] = json!(self.guide.question);
                state["instruction"] = json!(self.guide.instruction);
                state["overlayNote"] = json!(self.guide.overlay_note);
                self.decorate(&mut state);
                if state["phase"] == "showing" {
                    let now = native::now_ms();
                    let expires = state["expiresAt"].as_f64().unwrap_or(0.0);
                    let issued = state["issuedAt"].as_f64().unwrap_or(f64::INFINITY);
                    if !self.guide.ignore_cue
                        && state["live"] == true
                        && issued <= now
                        && expires > now
                        && let Some(display) = self.guide.display
                        && let Some(target) = target_box(&state).and_then(|r| display.target(self.guide.size, r))
                        && self.guide.observer.is_some()
                    {
                        let title = format!("October Lantern · Show me {}", self.guide.capture_id);
                        self.guide.ring = Some(Ring {
                            display,
                            target,
                            expires,
                            step: self.guide.capture_step,
                            started: Instant::now(),
                            title,
                            placed: false,
                            placement_check: Instant::now(),
                        });
                    }
                    state["live"] = json!(self.guide.ring.is_some());
                }
                if matches!(state["phase"].as_str(), Some("ended" | "failed" | "finished")) {
                    if self.guide.capture_pending {
                        self.windows.capture_done();
                        self.guide.capture_pending = false;
                    }
                    self.guide.observer = None;
                    self.guide.next_at = None;
                    self.guide.cancelled.store(true, Ordering::SeqCst);
                    if state["phase"] == "ended" {
                        self.guide.jpeg = None;
                        self.view.show_picture = None;
                    }
                }
                self.model.guide = Some(state);
            }
            _ => {}
        }
    }

    pub(super) fn show_captured(&mut self, run: &str, capture: &str, result: Result<Capture, String>) {
        if run != self.guide.run || capture != self.guide.capture_id || self.guide.cancelled.load(Ordering::SeqCst) {
            return;
        }
        self.windows.capture_done();
        self.guide.capture_pending = false;
        let captured = match result {
            Ok(v) => v,
            Err(e) => {
                self.show_failure(e);
                return;
            }
        };
        self.guide.display = Some(captured.display);
        if !captured.stable {
            let _ = self.engine.send(&json!({"type":"show.again","runId":run}));
            return;
        }
        self.guide.jpeg = Some(captured.jpeg);
        self.guide.size = captured.size;
        self.view.show_picture = Some(captured.picture);
        self.guide.reduce_motion = captured.reduce_motion;
        self.guide.overlay_note = captured.note;
        self.guide.observer = captured.observer;
        let display = captured.display.bounds;
        let live = self.guide.observer.is_some();
        let _ = self.engine.send(&json!({"type":"show.captured","runId":run,"step":self.guide.capture_step,
            "display":{"x":display.min.x,"y":display.min.y,"w":display.width(),"h":display.height()},
            "imageWidth":captured.size[0],"imageHeight":captured.size[1],"captureId":capture,
            "excludePids":[std::process::id()],"markedOnly":!live,"appWatchesScreen":live}));
    }

    pub(super) fn show_screen(&mut self, run: &str, capture: &str, change: Change) {
        if run != self.guide.run || capture != self.guide.capture_id || self.guide.next_at.is_some() || self.guide.ignore_cue {
            return;
        }
        self.hide_show_ring();
        self.guide.ignore_cue = true;
        if matches!(change, Change::Clicked) {
            self.guide.next_at = Some(Instant::now() + Duration::from_secs(1));
            if let Some(state) = &mut self.model.guide {
                state["live"] = json!(false);
            }
            return;
        }
        self.guide.observer = None;
        self.guide.cancelled.store(true, Ordering::SeqCst);
        if matches!(change, Change::Unavailable) {
            self.guide.forced_note = Some("On-screen pointing stopped being available. The place is marked on the captured picture; use Look again after changing the screen.".into());
        }
        let _ = self.engine.send(&json!({"type":"show.changed","runId":run,"captureId":capture,"step":self.guide.capture_step}));
    }

    fn show_failure(&mut self, error: String) {
        self.hide_show_ring();
        self.guide.observer = None;
        self.guide.next_at = None;
        self.guide.cancelled.store(true, Ordering::SeqCst);
        self.model.guide = Some(json!({"phase":"failed","question":self.guide.question,"message":error}));
        let _ = self.engine.send(&json!({"type":"show.stop","runId":self.guide.run}));
    }

    fn hide_show_ring(&mut self) {
        if let Some(ring) = self.guide.ring.take() {
            native::hide(&ring.title);
        }
        if let Some(observer) = &self.guide.observer {
            observer.point(None);
        }
    }

    pub(super) fn show_tick(&mut self) {
        if self.guide.next_at.is_some_and(|at| Instant::now() >= at) {
            self.guide.next_at = None;
            self.show_action("next");
        }
        if self.guide.ring.as_ref().is_some_and(|r| native::now_ms() >= r.expires) {
            self.hide_show_ring();
            if let Some(state) = &mut self.model.guide {
                state["live"] = json!(false);
            }
        }
    }

    /// A separate native viewport. OS placement uses physical bounds rather than dividing a
    /// secondary monitor's origin by the pill's DPI (which breaks mixed-scale desktops).
    pub(super) fn show_overlay(&mut self, ctx: &egui::Context) {
        self.guide.scale = ctx.native_pixels_per_point().unwrap_or(1.0);
        let Some(ring) = &mut self.guide.ring else {
            return;
        };
        if native::now_ms() >= ring.expires {
            return;
        }
        let display = ring.display;
        #[allow(unused_mut)]
        let mut builder = ViewportBuilder::default()
            .with_title(&ring.title)
            .with_inner_size(display.bounds.size() / display.scale)
            .with_position((display.bounds.min.to_vec2() / display.scale).to_pos2())
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_active(false)
            .with_mouse_passthrough(true)
            .with_clamp_size_to_monitor_size(false);
        #[cfg(target_os = "linux")]
        {
            builder = builder.with_override_redirect(true).with_window_type(egui::X11WindowType::Tooltip);
        }
        let mut failed = false;
        let reduce_motion = self.guide.observer.as_ref().map_or(self.guide.reduce_motion, Observer::reduce_motion);
        let observer = self.guide.observer.as_ref();
        ctx.show_viewport_immediate(ViewportId::from_hash_of(&ring.title), builder, |ui, _| {
            if !ring.placed || ring.placement_check.elapsed() >= Duration::from_millis(250) {
                ring.placed = native::place(&ring.title, display);
                ring.placement_check = Instant::now();
                if !ring.placed {
                    failed = ring.started.elapsed() > Duration::from_millis(750);
                    ui.ctx().request_repaint_after(Duration::from_millis(16));
                    return;
                }
            }
            let pulse =
                if reduce_motion { 0.0 } else { (1.0 - (ring.started.elapsed().as_secs_f32() * std::f32::consts::PI / 0.7).cos()) / 2.0 };
            // winit's X11 scale policy can differ between monitors. Read the native scale of
            // this viewport, never the pill's monitor and never the user's egui zoom.
            #[cfg(target_os = "linux")]
            let paint_display = Display { scale: ui.ctx().native_pixels_per_point().unwrap_or(display.scale), ..display };
            #[cfg(windows)]
            let paint_display = display;
            // Keep builder geometry stable; changing its logical hints after winit reports a
            // new monitor scale would move the window a second time. Native placement also
            // corrects asynchronous DPI-resize suggestions back to the captured pixel bounds.
            crate::show_overlay::paint(ui, paint_display, ring.target, ring.step, pulse);
            if let Some(observer) = observer {
                observer.point(Some(native::Cue {
                    display: paint_display,
                    target: ring.target,
                    expires_at: ring.expires,
                    title: ring.title.clone(),
                }));
            }
            if !reduce_motion {
                ui.ctx().request_repaint_after(Duration::from_millis(16));
            }
        });
        if failed {
            self.guide.forced_note = Some(
                "Lantern couldn't draw a click-through overlay on this desktop. The place is marked on the captured picture instead."
                    .into(),
            );
            self.show_action("again");
        }
    }

    /// The card's extras: who answered, and what a failed answer offers.
    fn decorate(&self, state: &mut Value) {
        if let Some(via) = &self.guide.via {
            state["via"] = json!(via);
        }
        if let Some(fix) = &self.guide.fix {
            state["fix"] = json!(fix);
        }
    }

    /// "Ask October's AI instead" after your provider refused: the same round again, and October's
    /// AI for the rest of this guide.
    pub(super) fn show_with_october_instead(&mut self) {
        if self.guide.fix != Some(crate::model::Fix::UseOctober) {
            return;
        }
        let Some(query) = self.guide.query.clone() else { return };
        self.guide.own_key_off = true;
        self.guide.fix = None;
        self.guide.via = None;
        if let Some(state) = self.model.guide.as_mut().and_then(Value::as_object_mut) {
            state.remove("message");
            state.remove("fix");
            state.remove("via");
        }
        crate::analytics::capture("own_key_fallback", json!({}));
        self.ask_show(query);
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
        self.guide.query = Some(event.clone());
        self.guide.fix = None;
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
            use_own_key: !self.guide.own_key_off,
            ..Default::default()
        };
        let own_key = self.key_for_ask();
        self.guide.instruction.clear();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            if cancelled.load(Ordering::SeqCst) {
                return;
            }
            let mut result = json!({"guidance":null});
            let mut instruction = String::new();
            let key = own_key.key(&request);
            let response = october_ai::stream(&request, &token, key.as_ref(), &mut |piece| {
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
                    Piece::Via(provider) => result["via"] = json!(provider),
                    Piece::Note(_) | Piece::Task { .. } => {}
                }
                true
            });
            result["instruction"] = json!(instruction.chars().take(500).collect::<String>());
            let failure = response.err();
            if let Some(error) = &failure {
                result["error"] = json!(error.to_string());
            }
            if !cancelled.load(Ordering::SeqCst) {
                let _ = tx.send(Event::ShowAnswer { run, round, capture, result, failure });
            }
        });
    }

    pub(super) fn show_answer(&mut self, run: &str, round: u64, capture: &str, mut result: Value, failure: Option<Failure>) {
        if run != self.guide.run
            || round != self.guide.round
            || capture != self.guide.capture_id
            || self.guide.cancelled.load(Ordering::SeqCst)
        {
            return;
        }
        self.guide.instruction = result["instruction"].as_str().unwrap_or_default().to_string();
        self.guide.via = result.as_object_mut().and_then(|result| result.remove("via")).and_then(|via| via.as_str().map(str::to_string));
        self.guide.fix = failure.as_ref().and_then(Failure::fix);
        // Your provider refused before answering: the engine waits while you choose to ask
        // October's AI (or Stop); the round's error goes to it only from here on.
        if self.guide.fix == Some(Fix::UseOctober) && self.guide.instruction.is_empty() {
            let message = failure.map(|failure| failure.to_string()).unwrap_or_default();
            let mut state = self.model.guide.clone().unwrap_or_else(|| json!({"question": self.guide.question}));
            state["message"] = json!(message);
            self.decorate(&mut state);
            self.model.guide = Some(state);
            return;
        }
        self.guide.fix = self.guide.fix.take().filter(|fix| *fix != Fix::UseOctober);
        let _ = self.engine.send(&json!({"type":"show.result","runId":run,"round":round,"captureId":capture,"result":result}));
    }
}

fn target_box(state: &Value) -> Option<[f32; 4]> {
    let a = state["target"]["image"].as_array().filter(|a| a.len() == 4)?;
    Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32, a[3].as_f64()? as f32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::Sidecar;
    use eframe::egui::{Color32, ColorImage, pos2, vec2};

    fn client() -> Lantern {
        let (engine, _) = crate::sidecar::test_sender();
        let mut app = Lantern::new(engine, None);
        app.guide = Guide {
            run: "run".into(),
            capture_id: "capture".into(),
            capture_step: 1,
            size: [800, 500],
            observer: Some(Observer::fixture()),
            display: Some(Display { bounds: Rect::from_min_size(pos2(-1600.0, -1000.0), vec2(1600.0, 1000.0)), scale: 2.0 }),
            ..Default::default()
        };
        app
    }

    fn showing() -> Value {
        json!({"type":"showState","runId":"run","captureId":"capture","phase":"showing","step":1,
            "target":{"image":[20,30,40,20]},"live":true,"issuedAt":native::now_ms()-10.0,"expiresAt":native::now_ms()+120_000.0})
    }

    fn request(engine: &Sidecar, kind: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let value = engine.events.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap().unwrap();
            if value["type"] == kind {
                return value;
            }
        }
    }

    #[test]
    fn cues_require_a_current_capture_live_observer_and_unexpired_engine_lease() {
        let mut app = client();
        app.show_event(showing());
        assert!(app.guide.ring.is_some());
        let mut old = showing();
        old["captureId"] = json!("old");
        app.show_event(old);
        assert!(app.guide.ring.is_some(), "an old state cannot clear the current cue");
        let mut expired = showing();
        expired["expiresAt"] = json!(native::now_ms() - 1.0);
        app.show_event(expired);
        assert!(app.guide.ring.is_none());
        let mut future = showing();
        future["issuedAt"] = json!(native::now_ms() + 10_000.0);
        app.show_event(future);
        assert!(app.guide.ring.is_none());
        app.guide.observer = None;
        app.show_event(showing());
        assert!(app.guide.ring.is_none());
        assert_eq!(app.model.guide.as_ref().unwrap()["live"], false);
    }

    #[test]
    fn a_ring_click_hides_immediately_then_next_runs_once_and_is_cancelled_by_a_new_capture() {
        let mut app = client();
        app.show_event(showing());
        app.show_screen("run", "capture", Change::Clicked);
        assert!(app.guide.ring.is_none());
        assert!(app.guide.next_at.is_some());
        app.guide.next_at = Some(Instant::now() - Duration::from_millis(1));
        app.show_tick();
        assert_eq!(request(&app.engine, "show.next")["step"], 1);
        assert!(app.guide.next_at.is_none());
        app.guide.ignore_cue = false;
        app.show_screen("run", "capture", Change::Clicked);
        app.show_event(json!({"type":"showCapture","runId":"run","captureId":"new","step":2}));
        assert!(app.guide.next_at.is_none());
        app.show_screen("run", "capture", Change::Screen);
        assert!(!app.guide.cancelled.load(Ordering::SeqCst), "old window events cannot cancel the new picture");
        app.stop_show();
        assert!(app.guide.next_at.is_none() && app.guide.ring.is_none());
    }

    #[test]
    fn a_screen_change_cancels_an_answer_in_flight_and_names_the_exact_capture() {
        let mut app = client();
        app.show_screen("run", "wrong", Change::Screen);
        assert!(!app.guide.cancelled.load(Ordering::SeqCst));
        app.show_screen("run", "capture", Change::Screen);
        assert!(app.guide.cancelled.load(Ordering::SeqCst));
        assert!(app.guide.observer.is_none());
        let change = request(&app.engine, "show.changed");
        assert_eq!(change["captureId"], "capture");
        assert_eq!(change["step"], 1);
    }

    #[test]
    fn captured_messages_preserve_display_geometry_and_only_claim_live_when_observation_exists() {
        for live in [false, true] {
            let mut app = client();
            let display = app.guide.display.unwrap();
            let capture = Capture {
                jpeg: vec![1],
                picture: ColorImage::filled([8, 5], Color32::BLACK),
                size: [8, 5],
                display,
                observer: live.then(Observer::fixture),
                note: (!live).then(|| "Marked picture on this desktop".into()),
                reduce_motion: true,
                stable: true,
            };
            app.show_captured("run", "capture", Ok(capture));
            let message = request(&app.engine, "show.captured");
            assert_eq!(message["display"], json!({"x":-1600.0,"y":-1000.0,"w":1600.0,"h":1000.0}));
            assert_eq!(message["imageWidth"], 8);
            assert_eq!(message["markedOnly"], !live);
            assert_eq!(message["appWatchesScreen"], live);
            assert!(app.guide.reduce_motion);
        }
    }

    #[test]
    fn returning_to_the_real_app_keeps_the_guide_and_its_pending_next() {
        let mut app = client();
        app.model.panel = Some(PanelMode::ShowMe);
        app.show_event(showing());
        app.show_screen("run", "capture", Change::Clicked);
        app.panel_lost_focus();
        assert!(!app.actions.contains(&crate::ui::UiAction::SetPanel(None)));
        assert!(app.guide.next_at.is_some());
        app.model.panel = Some(PanelMode::Agents);
        app.panel_lost_focus();
        assert!(app.actions.contains(&crate::ui::UiAction::SetPanel(None)), "ordinary idle panels still dismiss");
    }
}
