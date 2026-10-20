//! Point & Ask (Swift `PointAsk`): select part of the screen, then Ask October's AI about it in a
//! card beside the selection, or Send it to a coding agent like a reply.
//!
//! Every asynchronous step carries the selection or conversation it belongs to; a newer one makes
//! older results stale, so a late capture, answer or save never lands on the wrong card.
use super::{Lantern, Pending};
use crate::model::{AppContext, AskTurn, Capture, PanelMode, PointAskModel, PointAskPhase};
use crate::october_ai::{self, Failure, Piece};
use crate::os::Event;
use crate::ui::DictationOwner;
use eframe::egui::Rect;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Shown when Send's pinned recipient has exited (CONTRACT §9.4).
const RECIPIENT_EXITED: &str = "That agent has exited. Choose another.";

/// From the pill: Lantern's panel fades before the screen dims.
const PANEL_FADE: Duration = Duration::from_millis(150);

#[derive(Default)]
pub(super) struct Pointing {
    /// Selecting starts once the panel has faded.
    start_at: Option<Instant>,
    /// The current selection; captures, saves and sends for older ones are stale.
    selection: u64,
    /// The current conversation (questions on this card); older answers are stale.
    conversation: Arc<AtomicU64>,
    /// The card has been shown (Esc while reselecting goes back to it).
    card_shown: bool,
    /// Where the selection is, in screen pixels (for the card's placement and the crop).
    pub rect: Option<Rect>,
    /// The selected area as PNG, once captured.
    shot: Option<Vec<u8>>,
    /// When the shortcut was pressed, while it's held (letting go after a hold stops listening).
    pressed_at: Option<Instant>,
    /// The answer said the question is work: started through Ask Lantern once it's complete.
    work: Option<(String, Option<String>, Option<String>)>,
    /// The next selection starts with dictation on (the shortcut, not the pill's button).
    listen: bool,
}

/// Holding the shortcut longer than this, then letting go, stops listening; a tap keeps it on.
const HOLD: Duration = Duration::from_millis(350);

impl Lantern {
    /// The pill's button (and the tray menu): close the panel, then select. No listening.
    pub(super) fn start_point_ask(&mut self) {
        if self.model.point_ask.phase == PointAskPhase::Selecting {
            return;
        }
        self.pointing.pressed_at = None;
        self.pointing.listen = false;
        self.set_panel(None);
        self.pointing.start_at = Some(Instant::now() + PANEL_FADE);
    }

    /// The shortcut went down (Swift `PointAsk.pressed`): select while listening, so you can hold
    /// it, point and speak. Pressed again while the card is up, it stops or resumes listening.
    pub(super) fn point_ask_pressed(&mut self) {
        match self.model.point_ask.phase {
            PointAskPhase::Selecting => {}
            PointAskPhase::Card => self.toggle_dictation(DictationOwner::PointAsk),
            PointAskPhase::Closed => {
                self.pointing.pressed_at = Some(Instant::now());
                self.pointing.listen = true;
                self.set_panel(None);
                self.pointing.start_at = Some(Instant::now());
            }
        }
    }

    /// The shortcut came up (Swift `PointAsk.released`): after a hold, stop listening and wait for
    /// Enter; a quick tap keeps listening.
    pub(super) fn point_ask_released(&mut self) {
        let held = self.pointing.pressed_at.take().is_some_and(|at| at.elapsed() > HOLD);
        if held && self.model.point_ask.listening {
            self.toggle_dictation(DictationOwner::PointAsk);
        }
    }

    pub(super) fn point_ask_tick(&mut self) {
        if self.pointing.start_at.is_some_and(|at| Instant::now() >= at) {
            self.pointing.start_at = None;
            self.begin_point_ask();
        }
    }

    /// A new conversation: read the app you're in, then show the selection overlay.
    fn begin_point_ask(&mut self) {
        self.invalidate_selection();
        self.pointing.conversation.fetch_add(1, Ordering::SeqCst);
        self.pointing.card_shown = false;
        self.inputs.point_ask.clear();
        let context = crate::os::foreground_app();
        let recipient = default_recipient(&self.model.agents, context.as_ref(), self.inputs.drafts.recipient());
        self.model.point_ask = PointAskModel { phase: PointAskPhase::Selecting, context, recipient, ..Default::default() };
        if std::mem::take(&mut self.pointing.listen) {
            // Another draft's dictation ends: this one is about what you point at.
            if self.model.dictating {
                self.cancel_dictation();
            }
            self.toggle_dictation(DictationOwner::PointAsk);
        }
        self.freeze_screen();
    }

    /// Windows: before the overlay shows, capture the display under the pointer (Lantern's windows
    /// drawing nothing) for the overlay to show frozen, Snipping Tool style. Windows draws a
    /// transparent window over a whole monitor black, and the selection is cropped from this same
    /// still. Until it arrives Lantern's windows stay clear; without it, the overlay is transparent.
    fn freeze_screen(&mut self) {
        #[cfg(windows)]
        {
            self.windows.select.unfreeze();
            let generation = self.pointing.selection;
            self.windows.hide_for_capture(move |tx| {
                crate::os::task(tx, crate::os::freeze_screen, move |result| Event::Frozen { generation, result });
            });
        }
    }

    /// Windows: the display under the pointer came back (or couldn't be captured).
    #[cfg(windows)]
    pub(super) fn point_ask_frozen(&mut self, generation: u64, result: Result<crate::os::Frozen, String>) {
        if generation != self.pointing.selection || self.model.point_ask.phase != PointAskPhase::Selecting {
            return;
        }
        match result {
            Ok(frozen) => self.windows.select.freeze(frozen),
            Err(error) => super::trace!("point & ask: no frozen screen ({error}); the overlay is transparent"),
        }
    }

    /// The overlay finished: an area in screen pixels, or `None` (Esc).
    pub(super) fn point_ask_selected(&mut self, chosen: Option<Rect>) {
        super::trace!("point & ask: selected {chosen:?}");
        let Some(rect) = chosen else {
            // Esc while reselecting goes back to the card as it was; before any card, it cancels.
            if self.pointing.card_shown {
                self.model.point_ask.phase = PointAskPhase::Card;
                // The card's question box takes focus each time it appears (Swift `.onAppear { focused = true }`).
                self.model.compose_focus += 1;
            } else {
                self.cancel_point_ask();
            }
            return;
        };
        self.invalidate_selection();
        self.pointing.card_shown = true;
        self.pointing.rect = Some(rect);
        self.model.point_ask.phase = PointAskPhase::Card;
        self.model.compose_focus += 1;
        self.model.point_ask.screenshot = Capture::Capturing;
        self.model.point_ask.problem = None;
        let generation = self.pointing.selection;
        // Windows: the selection from the display as it was frozen under the overlay.
        #[cfg(windows)]
        if let Some((image, [x, y, width, height])) = self.windows.select.pick.take() {
            crate::os::task(
                self.tx.clone(),
                move || super::shots::crop_image(&image, x, y, width, height),
                move |result| Event::PointAskShot { generation, result },
            );
            return;
        }
        self.windows.hide_for_capture(move |tx| {
            crate::os::task(tx, crate::os::capture_screen, move |result| Event::Screenshot { owner: "__point".into(), generation, result });
        });
    }

    /// The whole screen came back: crop it to the selection.
    pub(super) fn point_ask_captured(&mut self, generation: u64, result: Result<std::path::PathBuf, String>) {
        if generation != self.pointing.selection || self.model.point_ask.phase == PointAskPhase::Closed {
            return;
        }
        super::trace!("point & ask: capture {generation}: {result:?}");
        let cropped = result.and_then(|full| {
            let rect = self.pointing.rect.ok_or("nothing selected")?;
            let bytes = std::fs::read(&full).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&full);
            super::shots::crop(&bytes, rect.min.x as u32, rect.min.y as u32, rect.width() as u32, rect.height() as u32)
                .map_err(|e| e.to_string())
        });
        self.point_ask_shot(generation, cropped);
    }

    /// The selection as PNG (or why there's none): kept for Ask and Send.
    pub(super) fn point_ask_shot(&mut self, generation: u64, cropped: Result<Vec<u8>, String>) {
        if generation != self.pointing.selection || self.model.point_ask.phase == PointAskPhase::Closed {
            return;
        }
        let saved = cropped.and_then(|png| {
            let path = crate::settings::data_dir().join("screenshots").join(format!("point-{}.png", uuid::Uuid::new_v4().simple()));
            crate::settings::private_write(&path, &png).map_err(|e| e.to_string())?;
            Ok((png, path))
        });
        match saved {
            Ok((png, path)) => {
                self.pointing.shot = Some(png);
                self.model.point_ask.screenshot = Capture::Ready { path };
            }
            Err(error) if error.contains("denied") || error.contains("cancelled") => {
                self.model.point_ask.screenshot = Capture::PermissionNeeded;
            }
            Err(error) => {
                self.model.point_ask.screenshot = Capture::Failed { message: error.clone() };
                self.model.point_ask.problem = Some(format!("Couldn't take the screenshot: {error}"));
            }
        }
    }

    /// New async work: older captures, answers and sends no longer count.
    fn invalidate_selection(&mut self) {
        self.pointing.selection += 1;
        self.pointing.shot = None;
        self.model.point_ask.screenshot = Capture::None;
        self.model.point_ask.asking = false;
        self.model.point_ask.sending = false;
    }

    pub(super) fn reselect_point_ask(&mut self) {
        self.model.point_ask.phase = PointAskPhase::Selecting;
        self.freeze_screen();
    }

    pub(super) fn cancel_point_ask(&mut self) {
        self.invalidate_selection();
        self.pointing.conversation.fetch_add(1, Ordering::SeqCst);
        self.pointing.start_at = None;
        self.pointing.rect = None;
        self.model.point_ask.phase = PointAskPhase::Closed;
        self.withdraw_dictation_offer(true);
    }

    /// Nothing is in flight; the screenshot is optional (it may have failed or been denied).
    fn can_send_selection(&self) -> bool {
        self.model.point_ask.screenshot != Capture::Capturing && !self.model.point_ask.sending
    }

    /// Enter: Ask when signed in to October (or with nobody to send to), otherwise Send.
    pub(super) fn submit_point_ask(&mut self) {
        if self.session.is_some() || self.model.point_ask.recipient.is_none() {
            self.ask_point();
        } else {
            self.send_point();
        }
    }

    fn ask_point(&mut self) {
        let Some(token) = self.session.as_ref().map(|session| session.access.clone()) else {
            // Ask needs October: open its sign-in.
            self.cancel_point_ask();
            self.set_panel(Some(PanelMode::October));
            return;
        };
        if !self.can_send_selection() || self.model.point_ask.asking {
            return;
        }
        let words = self.inputs.point_ask.trim().to_string();
        if words.is_empty() && !self.model.point_ask.turns.is_empty() {
            return; // a blank follow-up is ignored
        }
        let question = if words.is_empty() { "What is this?".to_string() } else { words };
        self.inputs.point_ask.clear();
        let earlier: Vec<(String, String)> = self.model.point_ask.turns.iter().map(|t| (t.question.clone(), t.answer.clone())).collect();
        self.model.point_ask.turns.push(AskTurn { question: question.clone(), ..Default::default() });
        self.model.point_ask.asking = true;
        let turn = self.model.point_ask.turns.len() - 1;
        let request = october_ai::Request {
            turns: earlier,
            question,
            image: self.pointing.shot.as_deref().and_then(super::shots::jpeg),
            context: context_lines(self.model.point_ask.context.as_ref()).join("\n"),
            mode: Some("point_task".into()),
            platform: self.model.caps.platform,
        };
        self.pointing.work = None;
        let generation = self.pointing.conversation.fetch_add(1, Ordering::SeqCst) + 1;
        let current = self.pointing.conversation.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let pieces = tx.clone();
            let result = october_ai::stream(&request, &token, &mut |piece| {
                current.load(Ordering::SeqCst) == generation && pieces.send(Event::PointAsk { generation, turn, piece }).is_ok()
            });
            let error = result.err().filter(|failure| *failure != Failure::Cancelled).map(|failure| failure.to_string());
            let _ = tx.send(Event::PointAskDone { generation, turn, error });
        });
    }

    pub(super) fn point_ask_piece(&mut self, generation: u64, turn: usize, piece: Piece) {
        if generation != self.pointing.conversation.load(Ordering::SeqCst) {
            return;
        }
        let Some(t) = self.model.point_ask.turns.get_mut(turn) else { return };
        match piece {
            Piece::Text(text) => t.answer.push_str(&text),
            Piece::Note(note) => t.note = Some(note),
            Piece::Task { prompt, agent, project } => self.pointing.work = Some((prompt, agent, project)),
        }
    }

    pub(super) fn point_ask_done(&mut self, generation: u64, turn: usize, error: Option<String>) {
        if generation != self.pointing.conversation.load(Ordering::SeqCst) {
            return;
        }
        if let Some(t) = self.model.point_ask.turns.get_mut(turn) {
            match error {
                Some(error) => {
                    t.answer = error;
                    t.failed = true;
                }
                None if t.answer.is_empty() => {
                    t.answer = "No answer came back. Try again.".into();
                    t.failed = true;
                }
                None => {
                    if let Some(work) = self.pointing.work.take() {
                        let (question, answer) = (t.question.clone(), t.answer.clone());
                        return self.start_point_task(question, answer, work);
                    }
                }
            }
        }
        self.model.point_ask.asking = false;
    }

    /// The answer is work ("make a folder and move these into it"): Ask Lantern starts an agent on
    /// it with this selection's screenshot, where the agent's progress is (Swift
    /// `PointAsk.startTask`). Point & Ask closes.
    fn start_point_task(&mut self, question: String, answer: String, work: (String, Option<String>, Option<String>)) {
        let screenshot = match &self.model.point_ask.screenshot {
            Capture::Ready { path } => Some(path.clone()),
            _ => None,
        };
        let context = self.model.point_ask.context.clone();
        self.adopt_point_task(question, answer, work, screenshot, context);
        self.cancel_point_ask();
        self.set_panel(Some(PanelMode::Assistant));
    }

    pub(super) fn copy_point_ask_answer(&mut self) {
        if let Some(answer) = self.model.point_ask.turns.iter().rev().find(|t| !t.failed && !t.answer.is_empty()) {
            self.windows.copy(answer.answer.clone());
        }
    }

    /// Hands the selection to the agent: the screenshot saved in its project (ignored by Git),
    /// your words, where it is, and October's answer if there was one.
    /// Choosing who Send goes to. A recipient that resolves clears only the "exited" problem;
    /// capture, permission and October errors stay.
    pub(super) fn choose_point_recipient(&mut self, agent_id: String) {
        if self.model.agent(&agent_id).is_some() && self.model.point_ask.problem.as_deref() == Some(RECIPIENT_EXITED) {
            self.model.point_ask.problem = None;
        }
        self.model.point_ask.recipient = Some(agent_id);
    }

    pub(super) fn send_point(&mut self) {
        let Some(pinned) = self.model.point_ask.recipient.clone() else { return };
        // The pinned agent stays pinned when it exits (CONTRACT §9.4): say so, never retarget.
        let Some(agent) = self.model.agent(&pinned).cloned() else {
            self.model.point_ask.problem = Some(RECIPIENT_EXITED.into());
            return;
        };
        if !self.can_send_selection() {
            return;
        }
        let mut words = self.inputs.point_ask.trim().to_string();
        if words.is_empty()
            && let Some(last) = self.model.point_ask.turns.last()
        {
            words = last.question.clone();
        }
        let shot = self.pointing.shot.clone();
        if words.is_empty() && shot.is_none() {
            return;
        }
        let answer = self.model.point_ask.turns.iter().rev().find(|t| !t.failed && !t.answer.is_empty()).map(|t| t.answer.clone());
        let mut lines = vec![if words.is_empty() { "Take a look at what I've selected on my screen.".to_string() } else { words }];
        let context = context_lines(self.model.point_ask.context.as_ref());
        self.model.point_ask.sending = true;
        let generation = self.pointing.selection;
        let tx = self.tx.clone();
        let cwd = agent.cwd.clone().map(std::path::PathBuf::from);
        std::thread::spawn(move || {
            let saved = match shot {
                Some(png) => {
                    let fallback = crate::settings::data_dir().join("screenshots");
                    match super::shots::save(&png, cwd.as_deref(), &fallback) {
                        Ok(path) => Some(path),
                        Err(_) => {
                            let _ = tx.send(Event::PointAskSent {
                                generation,
                                agent_id: agent.id,
                                text: Err("Couldn't save the screenshot. Nothing was sent.".into()),
                            });
                            return;
                        }
                    }
                }
                None => None,
            };
            if let Some(path) = saved {
                lines.push(format!("\nScreenshot of the part of my screen I selected: {}", path.display()));
            }
            if !context.is_empty() {
                lines.push(context.join("\n"));
            }
            if let Some(answer) = answer {
                lines.push(format!("October's quick answer, for reference:\n{}", answer.chars().take(3000).collect::<String>()));
            }
            let _ = tx.send(Event::PointAskSent { generation, agent_id: agent.id, text: Ok(lines.join("\n")) });
        });
    }

    pub(super) fn point_ask_sent(&mut self, generation: u64, agent_id: String, text: Result<String, String>) {
        if generation != self.pointing.selection {
            return;
        }
        self.model.point_ask.sending = false;
        match text {
            Ok(text) => {
                self.send_direct(&agent_id, text);
                self.cancel_point_ask();
            }
            Err(problem) => self.model.point_ask.problem = Some(problem),
        }
    }

    /// A message to an agent without the composer (Swift `sendDirect`): typed in where Lantern can,
    /// otherwise copied, with the agent's terminal brought forward.
    pub(super) fn send_direct(&mut self, agent_id: &str, text: String) {
        let Some(agent) = self.model.agent(agent_id).cloned() else { return };
        if agent.can_reply {
            if let Some(id) = self.request(serde_json::json!({"type": "reply", "agentId": agent.id, "text": text})) {
                self.pending.insert(id, Pending::Direct { agent_id: agent.id });
            }
            return;
        }
        self.copy_to_terminal(&agent, text);
    }

    /// A terminal Lantern can't type into (on Windows, any agent Lantern didn't start): copy the
    /// text and bring that terminal forward, with the paste shortcut (Swift `AppModel.send`).
    pub(super) fn copy_to_terminal(&mut self, agent: &crate::model::Agent, text: String) {
        self.windows.copy(text);
        crate::os::allow_focus(self.engine.pid());
        if let Some(id) = self.request(serde_json::json!({"type": "focus", "agentId": agent.id})) {
            self.pending.insert(id, Pending::Focus { agent_id: agent.id.clone() });
        }
        let place = agent.host.as_ref().map_or("its terminal".to_string(), |host| host.app.clone());
        self.show(format!("Copied. Paste into @{} in {place} with {}", agent.handle, paste_keys(self.model.caps.platform)));
    }
}

/// The paste shortcut people use in terminals on this platform.
fn paste_keys(platform: crate::model::Platform) -> &'static str {
    match platform {
        crate::model::Platform::Windows => "Ctrl+V",
        crate::model::Platform::Linux => "Ctrl+Shift+V",
    }
}

/// The context as lines of text, for the model or an agent (Swift `PointAsk.Context.lines`).
pub fn context_lines(context: Option<&AppContext>) -> Vec<String> {
    let Some(context) = context else { return Vec::new() };
    let mut lines = Vec::new();
    if !context.name.is_empty() {
        let window = context.window.as_ref().map_or(String::new(), |w| format!(", window \"{w}\""));
        lines.push(format!("App: {}{window}", context.name));
    }
    if let Some(document) = &context.document {
        lines.push(format!("Document: {}", document.display()));
    }
    if let Some(url) = &context.url {
        lines.push(format!("Page: {url}"));
    }
    if let Some(selected) = context.selected_text.as_ref().filter(|s| !s.is_empty()) {
        lines.push(format!("Selected text:\n{}", selected.chars().take(2000).collect::<String>()));
    }
    lines
}

/// Who "Send to" goes to (Swift `defaultRecipient`): the agent working on the open document's
/// project (deepest folder), one whose project the window title names, the agent in the terminal
/// in front, else the composer's agent, else the first live one.
pub fn default_recipient(agents: &[crate::model::Agent], context: Option<&AppContext>, composer: Option<&str>) -> Option<String> {
    let live: Vec<_> = agents.iter().filter(|agent| agent.is_live()).collect();
    if let Some(document) = context.and_then(|c| c.document.as_ref()).map(|d| d.to_string_lossy().into_owned()) {
        let within = |cwd: &String| document.starts_with(&format!("{cwd}/")) || document.starts_with(&format!("{cwd}\\"));
        if let Some(agent) =
            live.iter().filter(|a| a.cwd.as_ref().is_some_and(within)).max_by_key(|a| a.cwd.as_ref().map_or(0, String::len))
        {
            return Some(agent.id.clone());
        }
    }
    if let Some(title) = context.and_then(|c| c.window.as_ref()).map(|t| t.to_lowercase())
        && let Some(agent) = live.iter().find(|a| a.project.as_ref().is_some_and(|p| !p.is_empty() && title.contains(&p.to_lowercase())))
    {
        return Some(agent.id.clone());
    }
    if let Some(pid) = context.and_then(|c| c.pid)
        && let Some(agent) = live.iter().find(|a| a.host.as_ref().is_some_and(|host| host.pid == pid))
    {
        return Some(agent.id.clone());
    }
    composer.filter(|id| live.iter().any(|a| a.id == *id)).map(str::to_string).or_else(|| live.first().map(|a| a.id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Agent, AgentKind, HostApp};

    fn agent(id: &str, cwd: &str, project: &str) -> Agent {
        Agent { id: id.into(), cwd: Some(cwd.into()), project: Some(project.into()), ..Default::default() }
    }

    #[test]
    fn recipient_follows_document_then_title_then_terminal_then_composer() {
        let agents = vec![agent("a", "/w/site", "site"), agent("b", "/w/site/docs", "docs"), agent("c", "/w/api", "api")];
        let doc = AppContext { document: Some("/w/site/docs/intro.md".into()), ..Default::default() };
        assert_eq!(default_recipient(&agents, Some(&doc), None).as_deref(), Some("b"), "the deepest folder wins");
        let title = AppContext { window: Some("API — Firefox".into()), ..Default::default() };
        assert_eq!(default_recipient(&agents, Some(&title), None).as_deref(), Some("c"));
        let mut hosted = agent("d", "/x", "x");
        hosted.host = Some(HostApp { pid: 77, ..Default::default() });
        let terminal = AppContext { pid: Some(77), ..Default::default() };
        assert_eq!(default_recipient(&[agents[0].clone(), hosted], Some(&terminal), None).as_deref(), Some("d"));
        assert_eq!(default_recipient(&agents, None, Some("c")).as_deref(), Some("c"));
        assert_eq!(default_recipient(&agents, None, Some("gone")).as_deref(), Some("a"));
    }

    #[test]
    fn context_lines_match_the_mac() {
        let context = AppContext {
            name: "Firefox".into(),
            window: Some("Checkout".into()),
            url: Some("https://x.test".into()),
            selected_text: Some("y".repeat(2500)),
            ..Default::default()
        };
        let lines = context_lines(Some(&context));
        assert_eq!(lines[0], "App: Firefox, window \"Checkout\"");
        assert_eq!(lines[1], "Page: https://x.test");
        assert_eq!(lines[2].len(), "Selected text:\n".len() + 2000);
    }

    fn lantern() -> Lantern {
        let (engine, _) = crate::sidecar::test_sender();
        let mut lantern = Lantern::new(engine, None);
        lantern.connected = true;
        lantern
    }

    #[test]
    fn esc_while_reselecting_returns_to_the_card_but_cancels_before_one() {
        let mut lantern = lantern();
        lantern.begin_point_ask();
        lantern.point_ask_selected(None);
        assert_eq!(lantern.model.point_ask.phase, PointAskPhase::Closed);
        lantern.begin_point_ask();
        lantern.pointing.card_shown = true;
        lantern.reselect_point_ask();
        lantern.point_ask_selected(None);
        assert_eq!(lantern.model.point_ask.phase, PointAskPhase::Card);
    }

    #[test]
    fn a_stale_capture_or_answer_never_lands_on_a_newer_card() {
        let mut lantern = lantern();
        lantern.begin_point_ask();
        lantern.point_ask_selected(Some(Rect::from_min_size(Default::default(), eframe::egui::vec2(10.0, 10.0))));
        let old = lantern.pointing.selection;
        lantern.point_ask_selected(Some(Rect::from_min_size(Default::default(), eframe::egui::vec2(20.0, 20.0))));
        lantern.point_ask_captured(old, Err("late".into()));
        assert_eq!(lantern.model.point_ask.screenshot, Capture::Capturing, "the newer capture is still the one shown");
        lantern.model.point_ask.turns.push(AskTurn { question: "q".into(), ..Default::default() });
        let generation = lantern.pointing.conversation.load(Ordering::SeqCst);
        lantern.point_ask_piece(generation - 1, 0, Piece::Text("stale".into()));
        lantern.point_ask_piece(generation, 0, Piece::Text("fresh".into()));
        assert_eq!(lantern.model.point_ask.turns[0].answer, "fresh");
    }

    #[test]
    fn the_shortcut_listens_but_the_pills_button_does_not() {
        let mut lantern = lantern();
        lantern.point_ask_pressed();
        assert!(lantern.pointing.listen && lantern.pointing.pressed_at.is_some());
        lantern.start_point_ask();
        assert!(!lantern.pointing.listen && lantern.pointing.pressed_at.is_none(), "the pill's button never listens");
        // Pressed while selecting: nothing changes. A quick tap's release keeps listening.
        lantern.point_ask_pressed();
        lantern.begin_point_ask();
        assert_eq!(lantern.model.point_ask.phase, PointAskPhase::Selecting);
        lantern.model.point_ask.listening = true;
        lantern.point_ask_pressed();
        assert!(lantern.model.point_ask.listening);
        lantern.pointing.pressed_at = Some(Instant::now());
        lantern.point_ask_released();
        assert!(lantern.model.point_ask.listening, "a tap keeps listening");
    }

    #[test]
    fn the_card_takes_focus_each_time_it_appears() {
        let mut lantern = lantern();
        lantern.begin_point_ask();
        let before = lantern.model.compose_focus;
        lantern.point_ask_selected(Some(Rect::from_min_size(eframe::egui::pos2(10.0, 10.0), eframe::egui::vec2(100.0, 80.0))));
        assert_eq!(lantern.model.compose_focus, before + 1);
        lantern.reselect_point_ask();
        lantern.point_ask_selected(None);
        assert_eq!(lantern.model.point_ask.phase, PointAskPhase::Card);
        assert_eq!(lantern.model.compose_focus, before + 2, "back on the card after Esc while reselecting");
    }

    #[test]
    fn sending_to_an_agent_that_exited_says_so_and_keeps_it_pinned() {
        let mut lantern = lantern();
        lantern.begin_point_ask();
        lantern.model.point_ask.phase = PointAskPhase::Card;
        lantern.model.point_ask.recipient = Some("gone".into());
        lantern.inputs.point_ask = "Fix this".into();
        lantern.send_point();
        assert_eq!(lantern.model.point_ask.problem.as_deref(), Some(RECIPIENT_EXITED));
        assert_eq!(lantern.model.point_ask.recipient.as_deref(), Some("gone"));
        assert_eq!(lantern.inputs.point_ask, "Fix this");
        assert!(!lantern.model.point_ask.sending);
        // Choosing an agent that's gone too changes nothing visible; a live one clears the note.
        lantern.choose_point_recipient("also-gone".into());
        assert_eq!(lantern.model.point_ask.problem.as_deref(), Some(RECIPIENT_EXITED));
        lantern.model.agents.push(Agent { id: "live".into(), ..Default::default() });
        lantern.choose_point_recipient("live".into());
        assert_eq!(lantern.model.point_ask.problem, None);
        assert_eq!(lantern.model.point_ask.recipient.as_deref(), Some("live"));
        // Other problems are not the chooser's to clear.
        lantern.model.point_ask.problem = Some("Couldn't take the screenshot: denied".into());
        lantern.choose_point_recipient("live".into());
        assert_eq!(lantern.model.point_ask.problem.as_deref(), Some("Couldn't take the screenshot: denied"));
    }

    #[test]
    fn work_from_point_ask_starts_in_ask_lantern_with_the_screenshot() {
        let mut lantern = lantern();
        lantern.model.new_session.installed = Some(vec![AgentKind::new("claude")]);
        lantern.begin_point_ask();
        lantern.model.point_ask.phase = PointAskPhase::Card;
        lantern.model.point_ask.screenshot = Capture::Ready { path: "/shots/point-1.png".into() };
        lantern.model.point_ask.turns.push(AskTurn { question: "move these into a folder".into(), ..Default::default() });
        let generation = lantern.pointing.conversation.load(Ordering::SeqCst);
        lantern.point_ask_piece(generation, 0, Piece::Text("Starting an agent to make the folder.".into()));
        let prompt = "Make a folder on the Desktop and move the images in the attached screenshot into it.";
        lantern.point_ask_piece(generation, 0, Piece::Task { prompt: prompt.into(), agent: None, project: None });
        assert_eq!(lantern.model.point_ask.phase, PointAskPhase::Card, "nothing starts before the answer is complete");
        lantern.point_ask_done(generation, 0, None);
        assert_eq!(lantern.model.point_ask.phase, PointAskPhase::Closed);
        assert_eq!(lantern.model.panel, Some(PanelMode::Assistant));
        let turn = &lantern.model.assistant.turns[0];
        assert_eq!((turn.question.as_str(), turn.answer.as_str()), ("move these into a folder", "Starting an agent to make the folder."));
        let pending = lantern.model.assistant.pending_task.clone().expect("the task, started on the next tick");
        assert_eq!(pending.prompt, prompt, "October's prompt, which says what the screenshot shows");
        assert_eq!(pending.screenshot, Some("/shots/point-1.png".into()));
    }

    #[test]
    fn asking_signed_out_opens_october_sign_in() {
        let mut lantern = lantern();
        lantern.begin_point_ask();
        lantern.submit_point_ask();
        assert_eq!(lantern.model.point_ask.phase, PointAskPhase::Closed);
        assert_eq!(lantern.model.panel, Some(PanelMode::October));
    }

    #[test]
    fn sending_to_an_agent_that_takes_replies_types_it_in_and_closes_the_card() {
        let mut lantern = lantern();
        lantern.model.agents = vec![Agent { id: "a".into(), can_reply: true, ..agent("a", "/nowhere", "p") }];
        lantern.begin_point_ask();
        lantern.model.point_ask.recipient = Some("a".into());
        lantern.model.point_ask.phase = PointAskPhase::Card;
        lantern.inputs.point_ask = "why is this red?".into();
        let generation = lantern.pointing.selection;
        lantern.point_ask_sent(generation, "a".into(), Ok("why is this red?".into()));
        assert!(lantern.pending.values().any(|p| matches!(p, Pending::Direct { agent_id } if agent_id == "a")));
        assert_eq!(lantern.model.point_ask.phase, PointAskPhase::Closed);
    }
}
