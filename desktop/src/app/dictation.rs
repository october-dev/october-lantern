//! Dictation (Swift `Dictation`): speak into the draft you started it in. The Mac uses Apple's
//! on-device speech; here the audio is recorded and transcribed on this computer by whisper.cpp
//! (`crate::speech`: installed with Lantern on Windows, or downloaded with its model on first use,
//! after the person agrees to the download; a "Not now" isn't remembered). Each
//! dictation belongs to one draft: moving to another conversation ends it, and a late transcript
//! only ever lands in the draft it was started for.
use super::Lantern;
use crate::os::Event;
use crate::speech::{Readiness, Setup};
use crate::ui::DictationOwner;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Dictating {
    recording: Option<(crate::os::Recording, DictationOwner)>,
    /// The dictation whose transcript counts; cancelling makes older ones stale.
    generation: u64,
    /// The first-use setup under way (one at a time), and the draft to listen for when it's done.
    setup: Option<SettingUp>,
    /// Waiting for Download or Not now (`model.dictation_offer`).
    offer: Option<Offer>,
}

/// What a Download would set up, and for which draft.
struct Offer {
    owner: DictationOwner,
    /// The runtime found (`None`: download it too).
    whisper: Option<std::path::PathBuf>,
    /// The model needs downloading.
    model: bool,
}

struct SettingUp {
    owner: Option<DictationOwner>,
    percent: u8,
    /// What it downloads, for the toast.
    megabytes: u32,
}

fn encode(owner: &DictationOwner) -> String {
    match owner {
        DictationOwner::Reply(agent) => format!("reply:{agent}"),
        DictationOwner::Assistant => "assistant".into(),
        DictationOwner::PointAsk => "point".into(),
    }
}

fn decode(owner: &str) -> Option<DictationOwner> {
    match owner {
        "assistant" => Some(DictationOwner::Assistant),
        "point" => Some(DictationOwner::PointAsk),
        other => other.strip_prefix("reply:").map(|agent| DictationOwner::Reply(agent.into())),
    }
}

impl Lantern {
    pub(super) fn toggle_dictation(&mut self, owner: DictationOwner) {
        if let Some((recording, owner)) = self.dictating.recording.take() {
            // Stop: transcribe what was said, on this computer.
            let settings = crate::speech::resolved(crate::settings::Settings::load().unwrap_or_default());
            let generation = self.dictating.generation;
            let owner = encode(&owner);
            crate::os::task(
                self.tx.clone(),
                move || recording.finish(settings),
                move |result| Event::Transcript { owner, generation, result },
            );
            return;
        }
        if let Some(setup) = &mut self.dictating.setup {
            // Already setting up: say how far it's got, and listen for this draft once it's done.
            setup.owner = Some(owner);
            let percent = setup.percent;
            return self.show_progress(percent);
        }
        let settings = crate::settings::Settings::load().unwrap_or_default();
        let readiness = crate::speech::readiness(&settings);
        self.dictation_ready(owner, readiness);
    }

    fn dictation_ready(&mut self, owner: DictationOwner, readiness: Readiness) {
        match readiness {
            Readiness::Ready => self.listen(owner),
            Readiness::SetUp { whisper, model } => self.offer_dictation(owner, whisper, !model),
            Readiness::NoRuntime => self.show(crate::speech::missing_runtime()),
        }
    }

    /// Something needs downloading: ask first, where the mic was pressed (opening the panel when
    /// it was the menu's Dictate a Reply with the panel closed).
    fn offer_dictation(&mut self, owner: DictationOwner, whisper: Option<std::path::PathBuf>, model: bool) {
        // `model`: it needs downloading.
        let point_ask = owner == DictationOwner::PointAsk;
        if !point_ask && self.model.panel.is_none() {
            match &owner {
                DictationOwner::Reply(agent) => self.open_chat(agent),
                _ => self.set_panel(Some(crate::model::PanelMode::Assistant)),
            }
        }
        let text = crate::speech::offer(model, whisper.is_none());
        self.model.dictation_offer = Some(crate::model::DictationOffer { text, point_ask });
        self.dictating.offer = Some(Offer { owner, whisper, model });
    }

    /// Download: set up as asked, then listen. Not now: nothing is downloaded or remembered.
    pub(super) fn answer_dictation_offer(&mut self, download: bool) {
        self.model.dictation_offer = None;
        let Some(offer) = self.dictating.offer.take() else { return };
        if download && self.dictating.setup.is_none() && self.dictating.recording.is_none() {
            self.set_up_dictation(offer);
        }
    }

    /// The question goes away with its draft (unanswered, so it's asked again next time).
    fn withdraw_offer(&mut self, gone: impl Fn(&DictationOwner) -> bool) {
        if self.dictating.offer.as_ref().is_some_and(|offer| gone(&offer.owner)) {
            self.dictating.offer = None;
            self.model.dictation_offer = None;
        }
    }

    /// The panel closed (its question with it) or Point & Ask ended.
    pub(super) fn withdraw_dictation_offer(&mut self, point_ask: bool) {
        self.withdraw_offer(|owner| (*owner == DictationOwner::PointAsk) == point_ask);
    }

    fn listen(&mut self, owner: DictationOwner) {
        match crate::os::Recording::start() {
            Ok(recording) => {
                self.dictating.generation += 1;
                self.model.dictating = true;
                self.model.point_ask.listening = owner == DictationOwner::PointAsk;
                self.dictating.recording = Some((recording, owner));
            }
            Err(error) => match error.downcast_ref::<crate::os::MicrophoneProblem>() {
                Some(problem) => {
                    self.show(problem.message());
                    if let Some(page) = problem.settings_page() {
                        let _ = crate::os::open(page);
                    }
                }
                None => self.show(format!("Couldn't start dictation: {error:#}")),
            },
        }
    }

    /// First use: download the speech model (and on Windows a missing runtime) in the background,
    /// then listen.
    fn set_up_dictation(&mut self, Offer { owner, whisper, model }: Offer) {
        let megabytes = crate::speech::setup_megabytes(model, whisper.is_none());
        self.dictating.setup = Some(SettingUp { owner: Some(owner), percent: 0, megabytes });
        self.show_progress(0);
        if cfg!(test) {
            return;
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let progress = tx.clone();
            let result = crate::speech::set_up(whisper, |percent| {
                let _ = progress.send(Event::SpeechSetup(Setup::Progress(percent)));
            });
            let _ = tx.send(Event::SpeechSetup(Setup::Done(result.map_err(|error| format!("{error:#}")))));
        });
    }

    /// The setup's progress stays up between updates (a slow connection takes a while per percent).
    fn show_progress(&mut self, percent: u8) {
        let megabytes = self.dictating.setup.as_ref().map_or(148, |setup| setup.megabytes);
        self.show(crate::speech::progress(megabytes, percent));
        self.toast_until = Some(Instant::now() + Duration::from_secs(30));
    }

    pub(super) fn speech_setup(&mut self, step: Setup) {
        match step {
            Setup::Progress(percent) => {
                if let Some(setup) = &mut self.dictating.setup
                    && setup.percent != percent
                {
                    setup.percent = percent;
                    self.show_progress(percent);
                }
            }
            Setup::Done(result) => {
                let Some(setup) = self.dictating.setup.take() else { return };
                match (result, setup.owner) {
                    (Err(error), _) => self.show(crate::speech::failed(&error)),
                    (Ok(()), Some(owner)) => {
                        self.show("Dictation is set up. Listening…");
                        self.listen(owner);
                    }
                    (Ok(()), None) => self.show("Dictation is set up. Press the microphone to dictate."),
                }
            }
        }
    }

    /// Ends dictation without using it (its draft went away).
    pub(super) fn cancel_dictation(&mut self) {
        if let Some(setup) = &mut self.dictating.setup {
            setup.owner = None;
        }
        self.withdraw_offer(|_| true);
        self.dictating.recording = None;
        self.dictating.generation += 1;
        self.model.dictating = false;
        self.model.point_ask.listening = false;
    }

    /// Moving to another conversation ends a dictation that belongs to a different draft.
    pub(super) fn stop_dictation_if_away(&mut self, recipient: Option<&str>) {
        if let Some(setup) = &mut self.dictating.setup
            && let Some(DictationOwner::Reply(owner)) = &setup.owner
            && Some(owner.as_str()) != recipient
        {
            setup.owner = None;
        }
        self.withdraw_offer(|owner| matches!(owner, DictationOwner::Reply(owner) if Some(owner.as_str()) != recipient));
        if let Some((_, DictationOwner::Reply(owner))) = &self.dictating.recording
            && Some(owner.as_str()) != recipient
        {
            self.cancel_dictation();
        }
    }

    pub(super) fn transcript(&mut self, owner: &str, generation: u64, result: Result<String, String>) {
        if generation != self.dictating.generation {
            return;
        }
        self.model.dictating = false;
        self.model.point_ask.listening = false;
        let text = match result {
            Ok(text) if !text.trim().is_empty() => text.trim().to_string(),
            Ok(_) => return self.show("Didn't catch that. Try again a little closer to the microphone."),
            Err(error) => return self.show(format!("Dictation: {error}")),
        };
        let append = |buffer: &mut String| {
            if !buffer.is_empty() && !buffer.ends_with(char::is_whitespace) {
                buffer.push(' ');
            }
            buffer.push_str(&text);
        };
        match decode(owner) {
            Some(DictationOwner::Reply(agent)) => self.inputs.drafts.append(&text, &agent),
            Some(DictationOwner::Assistant) => append(&mut self.inputs.assistant),
            Some(DictationOwner::PointAsk) => append(&mut self.inputs.point_ask),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lantern() -> Lantern {
        let (engine, _) = crate::sidecar::test_sender();
        Lantern::new(engine, None)
    }

    #[test]
    fn owners_round_trip() {
        for owner in [DictationOwner::Reply("claude:1:2".into()), DictationOwner::Assistant, DictationOwner::PointAsk] {
            assert_eq!(decode(&encode(&owner)), Some(owner));
        }
    }

    #[test]
    fn a_transcript_lands_only_in_the_draft_it_was_started_for() {
        let mut lantern = lantern();
        lantern.inputs.drafts.address(Some("b"));
        lantern.dictating.generation = 4;
        lantern.transcript("reply:a", 4, Ok("ship it".into()));
        assert_eq!(lantern.inputs.drafts.text_for("a"), "ship it");
        assert_eq!(lantern.inputs.drafts.text_for("b"), "");
        lantern.inputs.assistant = "find".into();
        lantern.transcript("assistant", 4, Ok(" my passport ".into()));
        assert_eq!(lantern.inputs.assistant, "find my passport");
    }

    #[test]
    fn setting_up_shows_progress_and_says_what_went_wrong() {
        let mut lantern = lantern();
        let toast = |lantern: &Lantern| lantern.model.toast.as_ref().map(|toast| toast.text.clone()).unwrap_or_default();
        lantern.set_up_dictation(Offer { owner: DictationOwner::Assistant, whisper: Some("whisper-cli".into()), model: true });
        assert_eq!(toast(&lantern), "Setting up dictation (148 MB, once)… 0%");
        lantern.speech_setup(Setup::Progress(35));
        assert_eq!(toast(&lantern), "Setting up dictation (148 MB, once)… 35%");
        // Pressing the mic again doesn't start a second download.
        lantern.toggle_dictation(DictationOwner::PointAsk);
        assert_eq!(toast(&lantern), "Setting up dictation (148 MB, once)… 35%");
        assert_eq!(lantern.dictating.setup.as_ref().and_then(|setup| setup.owner.clone()), Some(DictationOwner::PointAsk));
        lantern.speech_setup(Setup::Done(Err("the download stopped (reset)".into())));
        assert_eq!(toast(&lantern), "Couldn't set up dictation: the download stopped (reset). Try again.");
        assert!(lantern.dictating.setup.is_none() && !lantern.model.dictating);
        // A late progress report after it ended changes nothing.
        lantern.speech_setup(Setup::Progress(80));
        assert!(lantern.dictating.setup.is_none());

        // The draft went away while it downloaded: it's ready, and nothing starts listening.
        // Without a runtime (a copy updated through the zip), it's downloaded too.
        lantern.set_up_dictation(Offer { owner: DictationOwner::Reply("a".into()), whisper: None, model: true });
        assert_eq!(toast(&lantern), "Setting up dictation (149 MB, once)… 0%");
        lantern.stop_dictation_if_away(Some("b"));
        lantern.speech_setup(Setup::Done(Ok(())));
        assert_eq!(toast(&lantern), "Dictation is set up. Press the microphone to dictate.");
        assert!(!lantern.model.dictating);
    }

    fn needs(whisper: Option<&str>, model: bool) -> Readiness {
        Readiness::SetUp { whisper: whisper.map(Into::into), model }
    }

    #[test]
    fn nothing_downloads_until_the_person_says_download() {
        let mut lantern = lantern();
        lantern.model.panel = Some(crate::model::PanelMode::Assistant);
        lantern.dictation_ready(DictationOwner::Assistant, needs(Some("whisper-cli"), false));
        let offer = lantern.model.dictation_offer.clone().expect("it asks");
        assert!(!offer.point_ask);
        assert_eq!(
            offer.text,
            "Lantern turns your voice into text on this computer; your audio never leaves it. It needs to download a speech model once (148 MB, from Hugging Face)."
        );
        assert!(lantern.dictating.setup.is_none() && lantern.model.toast.is_none() && !lantern.model.dictating);

        // Download: set up as before, listening for the same draft once it's done.
        lantern.apply(crate::ui::UiAction::AnswerDictationOffer { download: true });
        assert!(lantern.model.dictation_offer.is_none());
        let setup = lantern.dictating.setup.as_ref().expect("downloading");
        assert_eq!(setup.owner, Some(DictationOwner::Assistant));
        assert_eq!(lantern.model.toast.as_ref().map(|toast| toast.text.as_str()), Some("Setting up dictation (148 MB, once)… 0%"));
        // A second answer (a double click) changes nothing.
        lantern.answer_dictation_offer(true);
        assert_eq!(lantern.dictating.setup.as_ref().map(|setup| setup.percent), Some(0));
    }

    #[test]
    fn not_now_cancels_and_the_next_press_asks_again() {
        let mut lantern = lantern();
        lantern.model.point_ask.phase = crate::model::PointAskPhase::Card;
        lantern.dictation_ready(DictationOwner::PointAsk, needs(None, false));
        assert!(lantern.model.dictation_offer.as_ref().is_some_and(|offer| offer.point_ask));
        lantern.apply(crate::ui::UiAction::AnswerDictationOffer { download: false });
        assert!(lantern.model.dictation_offer.is_none() && lantern.dictating.offer.is_none());
        assert!(lantern.dictating.setup.is_none() && lantern.model.toast.is_none());
        assert!(!lantern.model.dictating && !lantern.model.point_ask.listening);
        // Not remembered: pressing the mic again asks again.
        lantern.dictation_ready(DictationOwner::PointAsk, needs(None, false));
        assert!(lantern.model.dictation_offer.is_some() && lantern.dictating.setup.is_none());
        // Closing Point & Ask takes its question with it, unanswered.
        lantern.cancel_point_ask();
        assert!(lantern.model.dictation_offer.is_none() && lantern.dictating.offer.is_none());
    }

    #[test]
    fn the_question_says_what_it_would_download() {
        let mut lantern = lantern();
        lantern.model.panel = Some(crate::model::PanelMode::Assistant);
        let text = |lantern: &Lantern| lantern.model.dictation_offer.as_ref().map(|offer| offer.text.clone()).unwrap_or_default();
        // A copy updated through the zip: the runtime too.
        lantern.dictation_ready(DictationOwner::Assistant, needs(None, false));
        assert!(text(&lantern).ends_with(
            "It needs to download a speech model once (148 MB, from Hugging Face) and its speech program (1 MB, from Lantern's releases)."
        ));
        lantern.answer_dictation_offer(false);
        lantern.dictation_ready(DictationOwner::Assistant, needs(None, true));
        assert!(text(&lantern).ends_with("It needs to download its speech program once (1 MB, from Lantern's releases)."));
        lantern.answer_dictation_offer(true);
        assert_eq!(lantern.model.toast.as_ref().map(|toast| toast.text.as_str()), Some("Setting up dictation (1 MB, once)… 0%"));
        lantern.dictating.setup = None;
        // Nothing to download (or no runtime to get, on Linux): no question.
        lantern.dictation_ready(DictationOwner::Assistant, Readiness::NoRuntime);
        assert!(lantern.model.dictation_offer.is_none());
        assert_eq!(lantern.model.toast.as_ref().map(|toast| toast.text.as_str()), Some(crate::speech::missing_runtime()));
    }

    #[test]
    fn the_question_goes_with_its_draft_and_opens_the_panel_from_the_menu() {
        let mut lantern = lantern();
        assert!(lantern.model.panel.is_none());
        lantern.dictation_ready(DictationOwner::Assistant, needs(Some("whisper-cli"), false));
        assert_eq!(lantern.model.panel, Some(crate::model::PanelMode::Assistant), "asked where it can be seen");
        assert!(lantern.model.dictation_offer.is_some());
        lantern.set_panel(None);
        assert!(lantern.model.dictation_offer.is_none() && lantern.dictating.offer.is_none());
        lantern.model.panel = Some(crate::model::PanelMode::Agents);
        lantern.dictation_ready(DictationOwner::Reply("a".into()), needs(Some("whisper-cli"), false));
        lantern.stop_dictation_if_away(Some("a"));
        assert!(lantern.model.dictation_offer.is_some(), "still its draft");
        lantern.stop_dictation_if_away(Some("b"));
        assert!(lantern.model.dictation_offer.is_none() && lantern.dictating.offer.is_none());
    }

    #[test]
    fn a_stale_transcript_is_dropped() {
        let mut lantern = lantern();
        lantern.dictating.generation = 2;
        lantern.transcript("point", 1, Ok("late".into()));
        assert!(lantern.inputs.point_ask.is_empty());
    }
}
