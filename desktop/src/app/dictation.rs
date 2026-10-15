//! Dictation (Swift `Dictation`): speak into the draft you started it in. The Mac uses Apple's
//! on-device speech; here the audio is recorded and transcribed on this computer by whisper.cpp
//! (`crate::speech`: shipped with Windows releases, its model downloaded on first use). Each
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
}

struct SettingUp {
    owner: Option<DictationOwner>,
    percent: u8,
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
        match crate::speech::readiness(&settings) {
            Readiness::Ready => self.listen(owner),
            Readiness::NeedsModel { whisper } => self.set_up_dictation(owner, whisper),
            Readiness::NoRuntime => self.show(crate::speech::missing_runtime(cfg!(windows))),
        }
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

    /// First use: download the speech model in the background, then listen.
    fn set_up_dictation(&mut self, owner: DictationOwner, whisper: std::path::PathBuf) {
        self.dictating.setup = Some(SettingUp { owner: Some(owner), percent: 0 });
        self.show_progress(0);
        if cfg!(test) {
            return;
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let progress = tx.clone();
            let result = crate::speech::set_up(&whisper, |percent| {
                let _ = progress.send(Event::SpeechSetup(Setup::Progress(percent)));
            });
            let _ = tx.send(Event::SpeechSetup(Setup::Done(result.map_err(|error| format!("{error:#}")))));
        });
    }

    /// The setup's progress stays up between updates (a slow connection takes a while per percent).
    fn show_progress(&mut self, percent: u8) {
        self.show(crate::speech::progress(percent));
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
        lantern.set_up_dictation(DictationOwner::Assistant, "whisper-cli".into());
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
        lantern.set_up_dictation(DictationOwner::Reply("a".into()), "whisper-cli".into());
        lantern.stop_dictation_if_away(Some("b"));
        lantern.speech_setup(Setup::Done(Ok(())));
        assert_eq!(toast(&lantern), "Dictation is set up. Press the microphone to dictate.");
        assert!(!lantern.model.dictating);
    }

    #[test]
    fn a_stale_transcript_is_dropped() {
        let mut lantern = lantern();
        lantern.dictating.generation = 2;
        lantern.transcript("point", 1, Ok("late".into()));
        assert!(lantern.inputs.point_ask.is_empty());
    }
}
