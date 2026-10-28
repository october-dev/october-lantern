//! Show me: point at the control the person is asking about, on their screen.
//!
//! One coordinator (a thread) owns each guide from start to stop; the app only takes pictures,
//! talks to October and draws the card. A guide goes:
//!
//! 1. `show.start` → the coordinator asks the app for a picture of the display (`showCapture`).
//! 2. `show.captured` (the display's place on screen and the picture's size) → the coordinator reads
//!    the visible windows' controls through Cua Driver and hands the app what to send October
//!    (`showQuery`).
//! 3. `show.result` (October's answer) → the coordinator checks it against that capture and has
//!    the app ring the place on screen (`showState` "showing"); an app that can only mark its
//!    picture (`markedOnly`) marks that instead.
//! 4. While the ring shows, the coordinator watches the control's window (or, for a place found
//!    only on the picture, every window on the display). If one moves, closes or another opens, it
//!    hides the ring and looks again (another capture, the next step).
//!    `show.next` (the person says they did it) looks again too; `show.stop` ends the guide.
//!
//! Every message carries the guide's id, its step and its round; anything for an older guide, step
//! or round is ignored, so a late answer can never move the pointer after Stop or a newer step.
//! Show me never clicks or types: see `cua::ALLOWED`.

mod cua;
pub mod geom;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use geom::{Answer, Capture, Control, Element, Rect, Window};

/// At most this many steps in one guide (and twice as many rounds: one look again per step).
pub const MAX_STEPS: u32 = 8;
const MAX_ROUNDS: u32 = 16;
/// A guide ends after this long, and after this long with nothing happening.
const LIFETIME: Duration = Duration::from_secs(600);
const IDLE: Duration = Duration::from_secs(120);
/// How often the pointed-at window is checked while the pointer shows.
const WATCH_EVERY: Duration = Duration::from_millis(250);
/// Windows read for controls: the frontmost few on the display.
const WINDOWS_READ: usize = 4;
const ELEMENTS_PER_WINDOW: u32 = 400;
/// The cursor's name, shown on the badge when someone hovers over it.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Captured {
    pub run_id: String,
    pub capture_id: String,
    pub step: u32,
    pub display: Rect,
    pub image_width: u32,
    pub image_height: u32,
    /// Lantern's own processes: their windows are never read or pointed at.
    #[serde(default)]
    pub exclude_pids: Vec<i64>,
    #[serde(default)]
    pub marked_only: bool,
}

pub enum Cmd {
    Start {
        run_id: String,
    },
    Captured(Captured),
    Result {
        run_id: String,
        round: u32,
        capture_id: String,
        result: Value,
    },
    Next {
        run_id: String,
        step: u32,
    },
    Again {
        run_id: String,
    },
    Stop {
        run_id: String,
    },
    /// Computer use was turned off (or taken over): drop the driver connection now.
    DriverOff,
}

/// The ingest epoch invalidates pending reads immediately, even while the coordinator is waiting
/// for Cua. Stale queued starts cannot reopen a guide after Stop.
#[derive(Clone)]
pub struct Handle {
    tx: Sender<(u64, Cmd)>,
    current: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
impl Handle {
    pub fn send(&self, cmd: Cmd) -> Result<(), Box<mpsc::SendError<(u64, Cmd)>>> {
        use std::sync::atomic::Ordering;
        let mut current = self.current.lock().unwrap();
        let changes = match &cmd {
            Cmd::Start { run_id } => {
                *current = Some(run_id.clone());
                true
            }
            Cmd::Stop { run_id } if current.as_deref() == Some(run_id) => {
                *current = None;
                true
            }
            Cmd::DriverOff => {
                *current = None;
                true
            }
            _ => false,
        };
        let epoch = if changes { self.epoch.fetch_add(1, Ordering::SeqCst) + 1 } else { self.epoch.load(Ordering::SeqCst) };
        self.tx.send((epoch, cmd)).map_err(Box::new)
    }
}

pub fn start(emit: fn(&Value)) -> Handle {
    let (tx, rx) = mpsc::channel();
    let epoch = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let read_epoch = epoch.clone();
    std::thread::Builder::new()
        .name("show-me".into())
        .spawn(move || Coordinator { emit, run: None, cua: None, epoch: read_epoch, active_epoch: 0 }.run(rx))
        .expect("starting Show me");
    Handle { tx, epoch, current: std::sync::Arc::new(std::sync::Mutex::new(None)) }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Waiting for the app's picture of the display.
    Capturing,
    /// Waiting for October's answer to `round`.
    Asking,
    /// Pointing (live, through the driver) or marking the picture.
    Showing,
    /// October found nothing to point at, or the guide is done: waiting for Next, Again or Stop.
    Waiting,
}

struct Run {
    id: String,
    phase: Phase,
    step: u32,
    round: u32,
    started: Instant,
    touched: Instant,
    /// October's id for this guide, from its first answer: later rounds belong to it.
    goal: Option<String>,
    capture: Option<Capture>,
    capture_id: String,
    controls: Vec<Control>,
    /// What would make the pointer wrong, while it shows.
    watching: Option<Watch>,
    /// Instructions of the steps already shown, for October's next round.
    shown: Vec<String>,
    pending_instruction: Option<String>,
    excluded: Vec<i64>,
    /// The app can only mark its picture (no on-screen pointer yet: Windows, Linux).
    marked_only: bool,
}

struct Coordinator {
    epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    active_epoch: u64,
    emit: fn(&Value),
    run: Option<Run>,
    cua: Option<cua::Cua>,
}

impl Coordinator {
    fn run(mut self, rx: Receiver<(u64, Cmd)>) {
        loop {
            match rx.recv_timeout(WATCH_EVERY) {
                Ok((epoch, cmd)) => {
                    if epoch != self.epoch.load(std::sync::atomic::Ordering::SeqCst) {
                        continue;
                    }
                    self.active_epoch = epoch;
                    self.handle(cmd);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.tick();
        }
        self.end(None);
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Start { run_id } => {
                if crate::computer::held() {
                    (self.emit)(
                        &json!({"type":"showState","runId":run_id,"step":1,"phase":"failed","message":"App access is paused. Continue before starting a guide."}),
                    );
                    return;
                }
                // One guide at a time: a new question replaces the old one.
                if self.run.is_some() {
                    self.end(Some("replaced"));
                }
                let now = Instant::now();
                self.run = Some(Run {
                    id: run_id,
                    phase: Phase::Capturing,
                    step: 1,
                    round: 0,
                    started: now,
                    touched: now,
                    goal: None,
                    capture: None,
                    capture_id: String::new(),
                    controls: Vec::new(),
                    watching: None,
                    shown: Vec::new(),
                    pending_instruction: None,
                    excluded: Vec::new(),
                    marked_only: false,
                });
                self.ask_for_capture();
            }
            Cmd::Captured(c) => self.captured(c),
            Cmd::Result { run_id, round, capture_id, result } => self.answered(&run_id, round, &capture_id, &result),
            Cmd::Next { run_id, step } => {
                if self.current(&run_id)
                    && self.run.as_ref().is_some_and(|r| r.step == step && matches!(r.phase, Phase::Showing | Phase::Waiting))
                {
                    self.advance();
                }
            }
            Cmd::Again { run_id } => {
                if self.current(&run_id) && self.run.as_ref().is_some_and(|r| r.phase != Phase::Asking) {
                    self.hide();
                    self.ask_for_capture();
                }
            }
            Cmd::Stop { run_id } => {
                if self.current(&run_id) {
                    self.end(Some("stopped"));
                }
            }
            Cmd::DriverOff => {
                self.end(Some("stopped"));
                self.hide();
                self.cua = None;
                if let Some(run) = &mut self.run {
                    run.watching = None;
                }
            }
        }
    }

    fn current(&self, run_id: &str) -> bool {
        self.run.as_ref().is_some_and(|r| r.id == run_id)
    }

    fn state(&self, phase: &str, extra: Value) {
        let Some(run) = &self.run else { return };
        let mut v = json!({"type": "showState", "runId": run.id, "phase": phase, "step": run.step, "round": run.round});
        if let (Value::Object(m), Value::Object(e)) = (&mut v, extra) {
            m.extend(e);
        }
        (self.emit)(&v);
    }

    fn ask_for_capture(&mut self) {
        let Some(run) = &mut self.run else { return };
        if run.round >= MAX_ROUNDS {
            return self.end(Some("limit"));
        }
        run.capture_id = uuid::Uuid::new_v4().to_string();
        run.phase = Phase::Capturing;
        run.watching = None;
        run.touched = Instant::now();
        let v = json!({"type": "showCapture", "runId": run.id, "step": run.step, "captureId": run.capture_id});
        (self.emit)(&v);
        self.state("looking", json!({}));
    }

    /// The person did the step (or says so): the next one.
    fn advance(&mut self) {
        let Some(run) = &mut self.run else { return };
        if let Some(instruction) = run.pending_instruction.take() {
            run.shown.push(instruction);
        }
        if run.step >= MAX_STEPS {
            return self.end(Some("limit"));
        }
        run.step += 1;
        self.hide();
        self.ask_for_capture();
    }

    fn captured(&mut self, c: Captured) {
        let Some(run) = &self.run else { return };
        if run.id != c.run_id || run.step != c.step || run.phase != Phase::Capturing || run.capture_id != c.capture_id {
            return;
        }
        let capture = Capture { display: c.display, image_w: c.image_width, image_h: c.image_height };
        if !capture.valid() {
            return self.end(Some("capture_failed"));
        }
        let controls = if c.marked_only {
            self.cua = None;
            Vec::new()
        } else {
            self.read_controls(&capture, &c.exclude_pids)
        };
        if crate::computer::held() || self.active_epoch != self.epoch.load(std::sync::atomic::Ordering::SeqCst) {
            return self.end(Some("stopped"));
        }
        let Some(run) = &mut self.run else { return };
        run.round += 1;
        run.capture = Some(capture);
        run.controls = controls;
        run.excluded = c.exclude_pids;
        run.marked_only = c.marked_only;
        run.phase = Phase::Asking;
        run.touched = Instant::now();
        let elements: Vec<Value> = run
            .controls
            .iter()
            .map(|c| json!([c.id, c.kind, c.label, [round1(c.image.x), round1(c.image.y), round1(c.image.w), round1(c.image.h)]]))
            .collect();
        let v = json!({
            "type": "showQuery", "runId": run.id, "step": run.step, "round": run.round, "captureId": run.capture_id,
            "goalId": run.goal, "elements": elements, "shown": run.shown,
        });
        (self.emit)(&v);
        self.state("asking", json!({}));
    }

    /// The visible windows' controls on the captured display, through the driver; none without one.
    fn read_controls(&mut self, capture: &Capture, excluded: &[i64]) -> Vec<Control> {
        let epoch = self.epoch.clone();
        let active = self.active_epoch;
        if active != epoch.load(std::sync::atomic::Ordering::SeqCst) {
            return Vec::new();
        }
        let Some(cua) = self.driver() else { return Vec::new() };
        let Ok(list) = cua.call("list_windows", json!({}), Duration::from_secs(4)) else { return Vec::new() };
        let mut windows: Vec<Window> =
            windows_of(&list).into_iter().filter(|w| !excluded.contains(&w.pid) && overlaps(&w.bounds, &capture.display)).collect();
        windows.sort_by_key(|w| std::cmp::Reverse(w.z));
        let mut read = Vec::new();
        for window in windows.into_iter().take(WINDOWS_READ) {
            if active != epoch.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            let args = json!({"pid": window.pid, "window_id": window.window_id, "tree_format": "elements",
                "include_screenshot": false, "max_elements": ELEMENTS_PER_WINDOW});
            let elements = cua.call("get_window_state", args, Duration::from_secs(4)).map(|v| elements_of(&v)).unwrap_or_default();
            read.push((window, elements));
        }
        geom::project(capture, &read)
    }

    fn answered(&mut self, run_id: &str, round: u32, capture_id: &str, result: &Value) {
        let Some(run) = &mut self.run else { return };
        if run.id != run_id || run.round != round || run.capture_id != capture_id || run.phase != Phase::Asking {
            return;
        }
        run.touched = Instant::now();
        if let Some(goal) = result.get("goalId").and_then(Value::as_str).filter(|g| g.len() <= 64) {
            run.goal = Some(goal.to_string());
        }
        if let Some(error) = result.get("error").and_then(Value::as_str) {
            run.phase = Phase::Waiting;
            let message = error.chars().take(300).collect::<String>();
            return self.state("failed", json!({"message": message}));
        }
        let instruction =
            result.get("instruction").and_then(Value::as_str).map(|s| s.chars().take(500).collect::<String>()).unwrap_or_default();
        match parse_guidance(result.get("guidance")) {
            Some(Guidance::Target(answer)) => {
                let capture = run.capture.expect("asked with a capture");
                let Ok(screen) = geom::resolve(&capture, &run.controls, &answer) else {
                    run.phase = Phase::Waiting;
                    return self.state("notVisible", json!({"reason": "bad_answer"}));
                };
                let image = capture.to_image(screen).unwrap_or(screen);
                if !instruction.is_empty() {
                    run.pending_instruction = Some(instruction);
                }
                run.phase = Phase::Showing;
                let window = run.controls.iter().find(|c| c.screen == screen).map(|c| c.window_id);
                let original = run.controls.clone();
                let excluded = run.excluded.clone();
                // A known control is pointed at only if the window's controls still match the
                // capture. A place October found on the picture (the menu bar, something the driver
                // can't name) was just captured, so it's pointed at too.
                let live = !run.marked_only && (window.is_none() || self.read_controls(&capture, &excluded) == original);
                if live {
                    self.watch(window, capture.display, &excluded);
                }
                if let Some(run) = &mut self.run {
                    run.touched = Instant::now();
                }
                let target = json!({"image": [round1(image.x), round1(image.y), round1(image.w), round1(image.h)]});
                // The pointer stays until Next, Look again or Stop, or until the screen changes
                // under it (`tick`); the guide's idle limit bounds it.
                let now = crate::hooks::now_ms();
                self.state("showing", json!({"target": target, "live": live, "issuedAt": now, "expiresAt": now + IDLE.as_millis() as u64}));
            }
            Some(Guidance::NotVisible) => {
                run.phase = Phase::Waiting;
                self.state("notVisible", json!({}));
            }
            Some(Guidance::Finished) => {
                run.phase = Phase::Waiting;
                self.hide();
                self.state("finished", json!({}));
            }
            None => {
                run.phase = Phase::Waiting;
                self.state("notVisible", json!({"reason": "bad_answer"}));
            }
        }
    }

    /// Starts watching what the pointer depends on: the control's window, or, for a place on the
    /// picture, every window on the display. Without a driver nothing is watched; the pointer
    /// stays until Next, Look again or Stop.
    fn watch(&mut self, window: Option<i64>, display: Rect, excluded: &[i64]) {
        if self.active_epoch != self.epoch.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let watch = match window {
            Some(id) => self.window_now(id).map(|(_, bounds, count)| Watch::Window(id, bounds, count)),
            None => self.screen_now(display, excluded).map(|windows| Watch::Screen(display, windows)),
        };
        if let Some(run) = &mut self.run {
            run.watching = watch;
        }
    }

    /// The windows on `display` (but Lantern's own), with their bounds, in a fixed order.
    fn screen_now(&mut self, display: Rect, excluded: &[i64]) -> Option<Vec<(i64, Rect)>> {
        let list = self.driver()?.call("list_windows", json!({}), Duration::from_secs(2)).ok()?;
        let mut windows: Vec<(i64, Rect)> = windows_of(&list)
            .into_iter()
            .filter(|w| !excluded.contains(&w.pid) && overlaps(&w.bounds, &display))
            .map(|w| (w.window_id, w.bounds))
            .collect();
        windows.sort_by_key(|w| w.0);
        Some(windows)
    }

    /// The window's owner, bounds, and how many windows its app shows, if it's still on screen.
    fn window_now(&mut self, window_id: i64) -> Option<(i64, Rect, usize)> {
        let list = self.driver()?.call("list_windows", json!({}), Duration::from_secs(2)).ok()?;
        let windows = windows_of(&list);
        let w = windows.iter().find(|w| w.window_id == window_id)?;
        Some((w.pid, w.bounds, windows.iter().filter(|o| o.pid == w.pid).count()))
    }

    fn hide(&mut self) {
        self.state("hide", json!({}));
    }

    /// The connection to Lantern's driver, opened on first use; none while computer use is off.
    fn driver(&mut self) -> Option<&cua::Cua> {
        if self.cua.is_none() && crate::computer::ready() {
            match cua::Cua::connect() {
                Ok(c) => {
                    self.cua = Some(c);
                }
                Err(e) => eprintln!("lantern-engine: Show me couldn't reach Cua Driver: {e:#}"),
            }
        }
        self.cua.as_ref()
    }

    /// Time limits, and the pointed-at window moving or the app opening another window.
    fn tick(&mut self) {
        let Some(run) = &self.run else { return };
        if run.started.elapsed() >= LIFETIME || run.touched.elapsed() >= IDLE {
            return self.end(Some("expired"));
        }
        let Some(watching) = run.watching.clone() else { return };
        if run.phase != Phase::Showing {
            return;
        }
        let excluded = run.excluded.clone();
        let unchanged = match watching {
            Watch::Window(window_id, bounds, count) => {
                matches!(self.window_now(window_id), Some((_, now, n)) if now == bounds && n == count)
            }
            Watch::Screen(display, windows) => self.screen_now(display, &excluded).is_some_and(|now| now == windows),
        };
        match unchanged {
            true => {}
            // Moved, resized, closed, or a new window (a dialog or menu opened): the pointer may be
            // wrong now, so hide it and look again.
            false => {
                self.hide();
                if let Some(run) = &mut self.run {
                    run.watching = None;
                }
                self.state("changed", json!({}));
                self.ask_for_capture();
            }
        }
    }

    /// Ends the guide (and the driver session with it). `why` is told to the app.
    fn end(&mut self, why: Option<&str>) {
        if self.run.is_none() {
            return;
        }
        if let Some(cua) = self.cua.take() {
            let _ = cua.call("end_session", json!({}), Duration::from_secs(2));
        }
        if let Some(why) = why {
            self.state("ended", json!({"reason": why}));
        }
        self.run = None;
    }
}

/// What the pointer depends on while it shows (see `Coordinator::watch`).
#[derive(Clone)]
enum Watch {
    /// A known control's window: its id, bounds, and how many windows its app shows.
    Window(i64, Rect, usize),
    /// A place on the picture: the display, and its windows with their bounds.
    Screen(Rect, Vec<(i64, Rect)>),
}

enum Guidance {
    Target(Answer),
    NotVisible,
    Finished,
}

/// October's structured answer: exactly one of a known control or a box, or a plain outcome.
fn parse_guidance(v: Option<&Value>) -> Option<Guidance> {
    let v = v?.as_object()?;
    match v.get("kind")?.as_str()? {
        "target" => {
            let control = v.get("control").and_then(Value::as_u64);
            let rect = v.get("box").and_then(Value::as_array).and_then(|a| {
                let n: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
                (a.len() == 4 && n.len() == 4).then(|| Rect::new(n[0], n[1], n[2], n[3]))
            });
            match (control, rect) {
                (Some(id), None) => u32::try_from(id).ok().map(|id| Guidance::Target(Answer::Control(id))),
                (None, Some(r)) => Some(Guidance::Target(Answer::Box(r))),
                _ => None,
            }
        }
        "not_visible" | "need_context" => Some(Guidance::NotVisible),
        "finished" => Some(Guidance::Finished),
        _ => None,
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn rect_of(v: &Value) -> Option<Rect> {
    let n = |k: &str, alt: &str| v.get(k).or_else(|| v.get(alt)).and_then(Value::as_f64);
    Some(Rect::new(n("x", "x")?, n("y", "y")?, n("width", "w")?, n("height", "h")?))
}

/// On-screen, ordinary windows from `list_windows`.
fn windows_of(v: &Value) -> Vec<Window> {
    v.get("windows")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|w| w.get("is_on_screen").and_then(Value::as_bool) != Some(false))
                .filter(|w| w.get("layer").and_then(Value::as_i64).unwrap_or(0) == 0)
                .filter_map(|w| {
                    Some(Window {
                        pid: w.get("pid")?.as_i64()?,
                        window_id: w.get("window_id")?.as_i64()?,
                        app: w.get("app_name").and_then(Value::as_str).unwrap_or_default().to_string(),
                        title: w.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
                        bounds: rect_of(w.get("bounds")?)?,
                        z: w.get("z_index").and_then(Value::as_i64).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The elements of a `get_window_state` read (`tree_format: "elements"`).
fn elements_of(v: &Value) -> Vec<Element> {
    v.get("elements")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|e| Element {
                    role: e.get("role").and_then(Value::as_str).unwrap_or_default().to_string(),
                    label: e.get("label").and_then(Value::as_str).map(str::to_string),
                    value: e.get("value").and_then(Value::as_str).map(str::to_string),
                    frame: e.get("frame").and_then(rect_of),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static EMITTED: Mutex<Vec<Value>> = Mutex::new(Vec::new());

    static LAST_CAPTURE: Mutex<Option<String>> = Mutex::new(None);
    fn record(v: &Value) {
        if v["type"] == "showCapture" {
            *LAST_CAPTURE.lock().unwrap() = v["captureId"].as_str().map(str::to_string);
        }
        EMITTED.lock().unwrap().push(v.clone());
    }

    fn take() -> Vec<Value> {
        std::mem::take(&mut *EMITTED.lock().unwrap())
    }

    fn coordinator() -> Coordinator {
        Coordinator {
            emit: record,
            run: None,
            cua: None,
            epoch: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            active_epoch: 0,
        }
    }

    #[test]
    fn an_old_stop_does_not_cancel_the_new_guide() {
        let (tx, rx) = mpsc::channel();
        let epoch = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let handle = Handle { tx, epoch: epoch.clone(), current: std::sync::Arc::new(Mutex::new(None)) };
        handle.send(Cmd::Start { run_id: "new".into() }).unwrap();
        let start_epoch = rx.recv().unwrap().0;
        handle.send(Cmd::Stop { run_id: "old".into() }).unwrap();
        assert_eq!(epoch.load(std::sync::atomic::Ordering::SeqCst), start_epoch);
        handle.send(Cmd::Stop { run_id: "new".into() }).unwrap();
        assert!(epoch.load(std::sync::atomic::Ordering::SeqCst) > start_epoch);
    }

    #[test]
    fn replacement_capture_and_driver_off_fence_late_work() {
        let _guard = SERIAL.lock().unwrap();
        take();
        let mut c = coordinator();
        c.handle(Cmd::Start { run_id: "r".into() });
        let old = captured("r", 1);
        c.handle(Cmd::Again { run_id: "r".into() });
        take();
        c.handle(old);
        assert!(!take().iter().any(|v| v["type"] == "showQuery"));
        c.handle(captured("r", 1));
        let query = take().into_iter().find(|v| v["type"] == "showQuery").unwrap();
        c.handle(Cmd::Next { run_id: "r".into(), step: 1 });
        assert_eq!(c.run.as_ref().unwrap().step, 1, "Next cannot complete an unanswered round");
        c.handle(Cmd::DriverOff);
        take();
        c.handle(Cmd::Result {
            run_id: "r".into(),
            round: 1,
            capture_id: query["captureId"].as_str().unwrap().into(),
            result: json!({"guidance":{"kind":"target","box":[10,10,20,20]}}),
        });
        assert!(c.run.is_none());
        assert!(!take().iter().any(|v| v["phase"] == "showing"));
    }

    fn captured(run: &str, step: u32) -> Cmd {
        Cmd::Captured(Captured {
            run_id: run.into(),
            capture_id: LAST_CAPTURE.lock().unwrap().clone().unwrap_or_default(),
            step,
            display: Rect::new(0.0, 0.0, 1600.0, 1000.0),
            image_width: 800,
            image_height: 500,
            exclude_pids: vec![],
            marked_only: false,
        })
    }

    #[test]
    fn a_guide_asks_for_a_picture_then_october_then_points() {
        let _guard = SERIAL.lock().unwrap();
        take();
        let mut c = coordinator();
        c.handle(Cmd::Start { run_id: "r1".into() });
        assert_eq!(take().iter().filter(|v| v["type"] == "showCapture").count(), 1);
        c.handle(captured("r1", 1));
        let query = take().into_iter().find(|v| v["type"] == "showQuery").unwrap();
        assert_eq!(query["round"], 1);
        let capture_id = query["captureId"].as_str().unwrap().to_string();
        // A place on the picture is pointed at on screen too (nothing to watch without a driver).
        c.handle(Cmd::Result {
            run_id: "r1".into(),
            round: 1,
            capture_id,
            result: json!({"goalId": "g1", "guidance": {"kind": "target", "box": [250, 150, 20, 10]}, "instruction": "Click the gear."}),
        });
        let showing = take().into_iter().find(|v| v["phase"] == "showing").unwrap();
        assert_eq!(showing["live"], true);
        assert!(showing["expiresAt"].as_u64().unwrap() - showing["issuedAt"].as_u64().unwrap() >= IDLE.as_millis() as u64);
        assert_eq!(showing["target"]["image"], json!([250.0, 150.0, 20.0, 10.0]));
        assert_eq!(c.run.as_ref().unwrap().goal.as_deref(), Some("g1"));
        // Next: step 2, and the next query names the goal and what was shown.
        c.handle(Cmd::Next { run_id: "r1".into(), step: 1 });
        c.handle(captured("r1", 2));
        let next = take().into_iter().find(|v| v["type"] == "showQuery").unwrap();
        assert_eq!((next["step"].as_u64(), next["round"].as_u64()), (Some(2), Some(2)));
        assert_eq!(next["goalId"], "g1");
        assert_eq!(next["shown"], json!(["Click the gear."]));
    }

    #[test]
    fn an_app_that_can_only_mark_its_picture_is_never_told_to_point() {
        let _guard = SERIAL.lock().unwrap();
        take();
        let mut c = coordinator();
        c.handle(Cmd::Start { run_id: "r1".into() });
        let Cmd::Captured(mut capture) = captured("r1", 1) else { unreachable!() };
        capture.marked_only = true;
        c.handle(Cmd::Captured(capture));
        let id = take().into_iter().find(|v| v["type"] == "showQuery").unwrap()["captureId"].as_str().unwrap().to_string();
        c.handle(Cmd::Result {
            run_id: "r1".into(),
            round: 1,
            capture_id: id,
            result: json!({"guidance": {"kind": "target", "box": [250, 150, 20, 10]}}),
        });
        assert_eq!(take().into_iter().find(|v| v["phase"] == "showing").unwrap()["live"], false);
    }

    #[test]
    fn late_or_foreign_answers_change_nothing() {
        let _guard = SERIAL.lock().unwrap();
        take();
        let mut c = coordinator();
        c.handle(Cmd::Start { run_id: "r1".into() });
        c.handle(captured("r1", 1));
        let id = take().into_iter().find(|v| v["type"] == "showQuery").unwrap()["captureId"].as_str().unwrap().to_string();
        let answer = json!({"guidance": {"kind": "target", "box": [250, 150, 20, 10]}});
        // Wrong capture, wrong round, another guide: ignored.
        c.handle(Cmd::Result { run_id: "r1".into(), round: 1, capture_id: "other".into(), result: answer.clone() });
        c.handle(Cmd::Result { run_id: "r1".into(), round: 2, capture_id: id.clone(), result: answer.clone() });
        c.handle(Cmd::Result { run_id: "r0".into(), round: 1, capture_id: id.clone(), result: answer.clone() });
        assert!(take().is_empty());
        // Stopped, then the real answer arrives: still nothing.
        c.handle(Cmd::Stop { run_id: "r1".into() });
        assert_eq!(take().into_iter().find(|v| v["phase"] == "ended").unwrap()["reason"], "stopped");
        c.handle(Cmd::Result { run_id: "r1".into(), round: 1, capture_id: id, result: answer });
        assert!(take().is_empty());
        assert!(c.run.is_none());
    }

    #[test]
    fn answers_must_name_one_known_control_or_one_box() {
        let _guard = SERIAL.lock().unwrap();
        assert!(parse_guidance(Some(&json!({"kind": "target", "control": 3, "box": [1, 2, 3, 4]}))).is_none());
        assert!(parse_guidance(Some(&json!({"kind": "target"}))).is_none());
        assert!(parse_guidance(Some(&json!({"kind": "target", "box": [1, 2, 3]}))).is_none());
        assert!(parse_guidance(Some(&json!({"kind": "click", "control": 1}))).is_none());
        assert!(matches!(parse_guidance(Some(&json!({"kind": "target", "control": 3}))), Some(Guidance::Target(Answer::Control(3)))));
        assert!(matches!(parse_guidance(Some(&json!({"kind": "finished"}))), Some(Guidance::Finished)));

        take();
        let mut c = coordinator();
        c.handle(Cmd::Start { run_id: "r1".into() });
        c.handle(captured("r1", 1));
        let id = take().into_iter().find(|v| v["type"] == "showQuery").unwrap()["captureId"].as_str().unwrap().to_string();
        // A box outside the picture isn't pointed at.
        c.handle(Cmd::Result {
            run_id: "r1".into(),
            round: 1,
            capture_id: id,
            result: json!({"guidance": {"kind": "target", "box": [900, 10, 20, 10]}}),
        });
        assert_eq!(take().into_iter().find(|v| v["type"] == "showState").unwrap()["phase"], "notVisible");
    }

    #[test]
    fn a_guide_stops_after_its_step_limit() {
        let _guard = SERIAL.lock().unwrap();
        take();
        let mut c = coordinator();
        c.handle(Cmd::Start { run_id: "r1".into() });
        for step in 1..=MAX_STEPS {
            c.handle(captured("r1", step));
            let capture_id = c.run.as_ref().unwrap().capture_id.clone();
            let round = c.run.as_ref().unwrap().round;
            c.handle(Cmd::Result { run_id: "r1".into(), round, capture_id, result: json!({"guidance":{"kind":"not_visible"}}) });
            c.handle(Cmd::Next { run_id: "r1".into(), step });
        }
        assert_eq!(take().into_iter().rfind(|v| v["phase"] == "ended").unwrap()["reason"], "limit");
        assert!(c.run.is_none());
    }

    #[test]
    fn the_drivers_window_list_and_elements_are_read() {
        let list = json!({"windows": [
            {"pid": 7, "window_id": 3, "app_name": "Finder", "title": "Downloads", "bounds": {"x": 325, "y": 191, "width": 920, "height": 436}, "z_index": 5, "is_on_screen": true, "layer": 0},
            {"pid": 8, "window_id": 4, "bounds": {"x": 0, "y": 0, "width": 10, "height": 10}, "is_on_screen": false},
            {"pid": 9, "window_id": 5, "bounds": {"x": 0, "y": 0, "width": 10, "height": 10}, "layer": 25}
        ]});
        let windows = windows_of(&list);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].bounds, Rect::new(325.0, 191.0, 920.0, 436.0));
        let state = json!({"elements": [{"element_index": 2, "role": "AXButton", "label": "Deny", "frame": {"h": 26, "w": 57, "x": 874, "y": -696}}]});
        let elements = elements_of(&state);
        assert_eq!(elements[0].frame, Some(Rect::new(874.0, -696.0, 57.0, 26.0)));
    }

    static SERIAL: Mutex<()> = Mutex::new(());
}
