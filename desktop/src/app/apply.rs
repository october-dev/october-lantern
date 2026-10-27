//! The one place UI intent becomes effects (CONTRACT §4). Actions carry what the person saw
//! (prompt ids, turn keys); stale ones are dropped here.
use super::{Lantern, Pending};
use crate::model::{AgentAnswer, Chat, PanelMode};
use crate::ui::UiAction;
use serde_json::json;

impl Lantern {
    pub(super) fn apply(&mut self, action: UiAction) {
        match action {
            UiAction::Control { action, launch } => {
                if self.model.control_sending {
                    return;
                }
                let request = json!({"type":"task.control","requestId":uuid::Uuid::new_v4().to_string(),"action":action,"launchId":launch});
                match self.engine.send(&request) {
                    Ok(()) => self.model.control_sending = true,
                    Err(error) => self.model.control_message = Some(error.to_string()),
                }
            }
            UiAction::ReviewRoutine(value) => {
                if let Ok(mut form) = serde_json::from_value::<crate::ui::routines::Form>(value) {
                    form.open = true;
                    self.view.routine_form = form;
                    self.set_panel(Some(PanelMode::Routines));
                }
            }

            UiAction::StartShow(question) => self.start_show(question),
            UiAction::Guide(action) => self.show_action(&action),
            UiAction::Routine(mut value) => {
                value["type"] = serde_json::json!("routine");
                value["requestId"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
                if let Err(error) = self.engine.send(&value) {
                    self.model.routine_error = Some(error.to_string());
                }
            }
            UiAction::SetPanel(panel) => self.set_panel(panel),
            UiAction::TogglePanel(mode) => self.set_panel(if self.model.panel == Some(mode) { None } else { Some(mode) }),
            UiAction::OpenChat { agent_id } => self.open_chat(&agent_id),
            UiAction::CloseChat => self.close_chat(),
            UiAction::Compose { agent_id: Some(agent_id) } => self.open_chat(&agent_id),
            UiAction::Compose { agent_id: None } => {
                if self.model.panel != Some(PanelMode::Assistant) {
                    self.set_panel(Some(PanelMode::Assistant));
                }
                self.model.compose_focus += 1;
            }
            UiAction::SendReply { agent_id } => self.send_reply(&agent_id),
            UiAction::OpenAgent { agent_id } => {
                crate::os::allow_focus(self.engine.pid());
                if let Some(id) = self.request(json!({"type": "focus", "agentId": agent_id})) {
                    self.pending.insert(id, Pending::Focus { agent_id });
                }
            }
            UiAction::DismissTurns { turn_keys } => {
                self.turns.dismiss(&turn_keys, &self.model.agents);
                self.model.inbox = self.turns.inbox(&self.model.agents, &self.model.prefs, &self.model.carded());
                self.save_prefs();
            }
            UiAction::AnswerQuestion { agent_id, prompt_id, answer } => self.answer_question(&agent_id, &prompt_id, answer),
            UiAction::StartNewSession => self.start_new_session(),
            UiAction::ChooseSessionAgent { kind } => self.choose_session_agent(kind),
            UiAction::ChooseSessionModel { kind, model } => {
                if self.inputs.new_session.kind.as_ref() == Some(&kind) {
                    self.inputs.new_session.model = model;
                }
            }
            UiAction::ChooseFolder => {
                // Losing focus to the picker doesn't close the panel (CONTRACT §6).
                self.windows.native_dialog = true;
                crate::os::task(self.tx.clone(), crate::os::folder, crate::os::Event::Folder);
            }
            UiAction::SetSessionScreenshot(on) => self.set_session_screenshot(on),
            // Asking again shows the desktop's screen-capture permission (Linux portal).
            UiAction::RequestCapturePermission | UiAction::RetakeSessionScreenshot => {
                if self.inputs.new_session.screenshot {
                    self.capture_screen();
                }
            }
            UiAction::SetOpenInTerminal(terminal) => {
                self.inputs.new_session.open_in_terminal = terminal;
                self.model.prefs.open_in_terminal = terminal;
                self.save_prefs();
            }
            UiAction::GetAgent { kind } => {
                if let Some(url) = kind.website() {
                    self.open_url(url);
                }
            }
            UiAction::Launch(request) => {
                self.launch(*request);
            }
            UiAction::SignInProvider(provider) => {
                let provider = match provider {
                    crate::ui::Provider::Google => "google",
                    crate::ui::Provider::GitHub => "github",
                    crate::ui::Provider::Apple => "apple",
                };
                self.sign_in(move |generation, current| crate::account::oauth(provider, generation, current));
            }
            UiAction::SignInEmail { email, password } => {
                self.sign_in(move |_, _| crate::account::password(email.trim().to_string(), password));
            }
            UiAction::CancelSignIn => self.cancel_sign_in(),
            UiAction::SignOut => self.sign_out(),
            UiAction::ToggleDictation { owner } => self.toggle_dictation(owner),
            UiAction::AnswerDictationOffer { download } => self.answer_dictation_offer(download),
            UiAction::ShowSettings => {
                self.refresh_notifications();
                self.refresh_hooks();
                self.windows.dialogs.settings = true;
            }
            UiAction::ChangeSetting(change) => self.change_setting(change),
            UiAction::SetHooks { enabled } => self.set_hooks(enabled),
            UiAction::RefreshToolkit => self.refresh_toolkit(false),
            UiAction::OpenToolkit => self.refresh_toolkit(true),
            UiAction::OpenNotificationSettings => self.open_notification_settings(),
            UiAction::CheckUpdates => self.check_updates(),
            // The confirmation step is the Settings view's own state.
            UiAction::ConfirmUninstall => self.uninstall(),
            UiAction::RemoveAppAnyway => self.remove_app(),
            UiAction::DismissUninstall => self.dismiss_uninstall(),
            UiAction::ShowWelcome => self.show_welcome(),
            UiAction::FinishWelcome => self.finish_welcome(),
            UiAction::StartPointAsk => self.start_point_ask(),
            UiAction::ReselectPointAsk => self.reselect_point_ask(),
            UiAction::CancelPointAsk => self.cancel_point_ask(),
            UiAction::ChoosePointAskRecipient(agent_id) => self.choose_point_recipient(agent_id),
            UiAction::SubmitPointAsk => self.submit_point_ask(),
            UiAction::SendPointAskToAgent => self.send_point(),
            UiAction::CopyPointAskAnswer => self.copy_point_ask_answer(),
            UiAction::SubmitAssistant => self.submit_assistant(),
            UiAction::ClearAssistant => self.clear_assistant(),
            UiAction::RetryAssistantTask { turn } => self.retry_assistant_task(turn),
            UiAction::OpenUpgrade { url, from } => {
                crate::analytics::capture("upgrade_opened", serde_json::json!({"from": from}));
                self.open_url(&url);
            }
            UiAction::SetShareUsage(on) => crate::analytics::set_enabled(on),
            UiAction::PrepareHarness => {
                self.request(serde_json::json!({"type": "harness.prepare"}));
            }
            UiAction::ToggleAskLanternInstead => {
                self.model.assistant.ask_lantern_instead = !self.model.assistant.ask_lantern_instead;
                self.model.compose_focus += 1;
            }
            UiAction::OpenConversation { id } => self.open_conversation(&id),
            UiAction::DeleteConversation { id } => self.delete_conversation(&id),
            UiAction::CloseLaunchPreview => self.close_preview(),
            UiAction::RetryPreviewInTerminal => self.retry_preview_in_terminal(),
            UiAction::OpenUrl(url) => self.open_url(&url),
            UiAction::RevealFile(path) => {
                if let Err(error) = crate::os::reveal(std::path::Path::new(&path)) {
                    self.show(format!("Couldn't show {path}: {error:#}"));
                }
            }
            UiAction::AnswerCard { card_id, revision, answer } => self.answer_card(card_id, revision, answer),
            UiAction::PasteAssistantImages => {
                let dir = super::images::dir();
                crate::os::task(self.tx.clone(), move || super::images::from_clipboard(&dir), crate::os::Event::AssistantImages);
            }
            UiAction::AddAssistantImages(files) => {
                let dir = super::images::dir();
                crate::os::task(self.tx.clone(), move || save_images(&files, &dir), crate::os::Event::AssistantImages);
            }
            UiAction::PickAssistantImages => {
                // Losing focus to the picker doesn't close the panel (CONTRACT §6).
                self.windows.native_dialog = true;
                let dir = super::images::dir();
                crate::os::task(self.tx.clone(), move || save_images(&crate::os::images()?, &dir), crate::os::Event::AssistantImages);
            }
            UiAction::RemoveAssistantImage(path) => {
                self.model.assistant.attachments.retain(|p| *p != path);
                self.model.assistant.attach_problem = None;
            }
            UiAction::OpenImage(path) => self.open_url(&path),
            // October Desktop (engine/src/october_link.rs).
            UiAction::DesktopConnect => {
                self.request(json!({"type": "october.pair"}));
            }
            UiAction::DesktopCancelPairing => {
                self.request(json!({"type": "october.cancelPair"}));
            }
            UiAction::DesktopDisconnect => {
                self.request(json!({"type": "october.forget"}));
            }
            // The Mac opens /Applications/October.app; elsewhere October registers its october:
            // URL scheme, so the system starts whichever October is installed.
            UiAction::OpenOctober => {
                if crate::os::open("october://").is_err() {
                    self.show("Couldn't open October Desktop. Is it installed?");
                }
            }
            UiAction::GetOctober => self.open_url("https://www.october.dev"),
            UiAction::ReportProblem => {
                let url = problem_report(&self.model.agents);
                if crate::os::open(&url).is_err() {
                    self.show(format!("Write to {SUPPORT_EMAIL} about the problem."));
                }
            }
            UiAction::SendLogs => self.send_logs(),
            UiAction::Copy(text) => self.windows.copy(text),
            UiAction::Quit => self.windows.quit(),
        }
    }

    /// Changing panels (or closing it) also leaves any open conversation; leaving Waiting means
    /// you've seen what was in it.
    pub(super) fn set_panel(&mut self, panel: Option<PanelMode>) {
        if self.model.panel == Some(PanelMode::ShowMe) && panel != Some(PanelMode::ShowMe) {
            self.stop_show();
        }
        let old = self.model.panel;
        if old == panel {
            return;
        }
        super::trace!("panel {old:?} -> {panel:?}");
        self.model.panel = panel;
        self.close_chat();
        if panel.is_none() {
            self.withdraw_dictation_offer(false);
        }
        if old == Some(PanelMode::Inbox) && self.turns.mark_seen(&self.model.agents, &self.model.prefs) {
            self.model.inbox = self.turns.inbox(&self.model.agents, &self.model.prefs, &self.model.carded());
            self.save_prefs();
        }
    }

    /// Opens an agent's conversation in the panel; the composer then talks to it.
    pub(super) fn open_chat(&mut self, agent_id: &str) {
        if self.model.agent(agent_id).is_none() {
            return;
        }
        if !self.model.panel.is_some_and(PanelMode::is_list) {
            self.set_panel(Some(PanelMode::Inbox));
        }
        if self.model.chat.as_ref().map(|chat| chat.agent_id.as_str()) != Some(agent_id) {
            self.model.chat = Some(Chat { agent_id: agent_id.into(), supported: true, loading: true, ..Default::default() });
        }
        self.stop_dictation_if_away(Some(agent_id));
        self.inputs.drafts.address(Some(agent_id));
        self.refresh_chat();
        self.model.compose_focus += 1;
    }

    /// Leaving a conversation also ends a starting session's preview (Swift `closeChat`). Launches
    /// set the panel before the preview, so this never ends the preview they just showed (§9.1).
    pub(super) fn close_chat(&mut self) {
        if self.model.launch_preview.is_some() {
            self.end_preview();
        }
        if let Some(old) = self.history_request.take() {
            self.pending.remove(&old);
        }
        self.model.chat = None;
    }

    fn send_reply(&mut self, agent_id: &str) {
        if self.inputs.drafts.recipient() != Some(agent_id) || self.model.sending {
            return;
        }
        let Some(ticket) = self.inputs.drafts.ticket() else { return };
        let Some(agent) = self.model.agent(agent_id).cloned() else { return };
        if !agent.can_reply {
            // Swift `AppModel.send`: copy, bring the terminal forward, and the draft is done.
            self.stop_dictation_if_away(None);
            self.copy_to_terminal(&agent, ticket.text.clone());
            self.inputs.drafts.sent(&ticket);
            return;
        }
        if let Some(id) = self.request(json!({"type": "reply", "agentId": agent_id, "text": ticket.text})) {
            self.pending.insert(id, Pending::Reply(ticket));
            self.model.sending = true;
        }
    }

    /// Answers exactly the question the person saw, once, or nothing. The engine presses the
    /// terminal's keys for it.
    /// Answers a card; the engine saves it only if the agent is still waiting for it.
    fn answer_card(&mut self, card_id: String, revision: i64, answer: crate::model::CardAnswer) {
        let waiting = self.model.launches.iter().flat_map(|l| &l.cards).any(|c| c.id == card_id && c.is_waiting());
        if !waiting || self.model.card_sending.contains(&card_id) {
            return;
        }
        self.model.card_errors.remove(&card_id);
        let request = json!({"type": "card.answer", "cardId": card_id, "revision": revision, "answer": answer});
        if self.request(request).is_some() {
            self.model.card_sending.insert(card_id);
        } else {
            self.model.card_errors.insert(card_id, "Lantern's engine isn't running. Try again in a moment.".into());
        }
    }

    fn answer_question(&mut self, agent_id: &str, prompt_id: &str, answer: AgentAnswer) {
        let current = self.model.agent(agent_id).filter(|agent| agent.is_answerable() && agent.prompt_id.as_deref() == Some(prompt_id));
        if current.is_none() || self.model.answered.contains_key(prompt_id) {
            return;
        }
        let request = json!({"type": "answer", "agentId": agent_id, "promptId": prompt_id, "answer": answer});
        if let Some(id) = self.request(request) {
            self.model.answered.insert(prompt_id.into(), answer);
            self.model.answer_delivery.insert(prompt_id.into(), crate::model::AnswerDelivery::Sending);
            self.pending.insert(id, Pending::Keys { agent_id: agent_id.into(), prompt_id: Some(prompt_id.into()) });
        }
    }
}

impl Lantern {
    fn open_url(&mut self, url: &str) {
        if let Err(error) = crate::os::open(url) {
            self.show(format!("Couldn't open {url}: {error:#}"));
        }
    }
}

/// Saves the images among `files` for Ask Lantern; only other kinds of file: says so.
fn save_images(files: &[std::path::PathBuf], dir: &std::path::Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let images: Vec<_> = files.iter().filter(|f| super::images::is_image(f)).collect();
    if images.is_empty() && !files.is_empty() {
        anyhow::bail!("Only PNG, JPEG, GIF and WebP images can be added.");
    }
    images.into_iter().map(|f| super::images::save_file(f, dir)).collect()
}

/// Where problem reports go (Swift `Support.email`).
const SUPPORT_EMAIL: &str = "hey@october.dev";

/// Report a Problem (Swift `Support.reportProblem`): an email to the October team with the version,
/// the system and a summary of the agents and terminals, never their conversations, folders or code.
fn problem_report(agents: &[crate::model::Agent]) -> String {
    let count = |labels: Vec<String>| {
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for label in labels {
            *counts.entry(label).or_default() += 1;
        }
        let joined: Vec<String> = counts.into_iter().map(|(label, n)| format!("{label} ×{n}")).collect();
        if joined.is_empty() { "none".to_string() } else { joined.join(", ") }
    };
    let version = env!("CARGO_PKG_VERSION");
    let kinds = count(agents.iter().map(|agent| agent.kind.display_name().to_string()).collect());
    let terminals = count(
        agents
            .iter()
            .map(|agent| {
                let host = agent.host.as_ref().map_or("?", |host| host.app.as_str());
                let via = agent.route.as_ref().map_or("none", |route| route.via.as_str());
                format!("{host} via {via}")
            })
            .collect(),
    );
    let system = format!("{} {}", crate::updates::system_name(), std::env::consts::ARCH);
    let body = format!(
        "What happened, and what did you expect?\n\n\n---\nOctober Lantern {version}\n{system}\nAgents: {kinds}\nTerminals: {terminals}\n"
    );
    // Strict percent-encoding: mail apps read `+` in a mailto body literally.
    let encode = |text: &str| {
        text.bytes()
            .map(|byte| match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (byte as char).to_string(),
                _ => format!("%{byte:02X}"),
            })
            .collect::<String>()
    };
    format!("mailto:{SUPPORT_EMAIL}?subject={}&body={}", encode(&format!("October Lantern {version}: a problem")), encode(&body))
}

#[cfg(test)]
mod reply_tests {
    use crate::model::{Agent, AgentKind, AgentState, HostApp};

    #[test]
    fn a_reply_lantern_cannot_type_is_copied_with_the_paste_shortcut_and_the_draft_clears() {
        let (engine, _) = crate::sidecar::test_sender();
        let mut lantern = super::Lantern::new(engine, None);
        lantern.connected = true;
        lantern.model.agents = vec![Agent {
            id: "codex:7".into(),
            kind: AgentKind::new("codex"),
            handle: "codex-7".into(),
            state: AgentState::Waiting,
            can_reply: false,
            host: Some(HostApp { app: "Windows Terminal".into(), ..Default::default() }),
            ..Default::default()
        }];
        lantern.model.caps.platform = crate::model::Platform::Windows;
        lantern.inputs.drafts.address(Some("codex:7"));
        lantern.inputs.drafts.edit_text(None, |text| text.push_str("run the tests"));
        lantern.send_reply("codex:7");
        assert_eq!(lantern.windows.clipboard(), Some("run the tests"));
        assert_eq!(lantern.inputs.drafts.text(), "", "the draft is done");
        assert!(!lantern.model.sending);
        let toast = lantern.model.toast.as_ref().map(|toast| toast.text.as_str()).unwrap_or_default();
        assert_eq!(toast, "Copied. Paste into @codex-7 in Windows Terminal with Ctrl+V");
    }
}

#[cfg(test)]
mod report_tests {
    use crate::model::{Agent, AgentKind, HostApp, Route};

    #[test]
    fn a_problem_report_is_an_email_with_counts_and_nothing_private() {
        let agent = |kind: &str, app: &str, via: &str| Agent {
            kind: AgentKind::new(kind),
            cwd: Some("/home/me/secret-project".into()),
            last_message: Some("private words".into()),
            host: Some(HostApp { app: app.into(), ..Default::default() }),
            route: Some(Route { via: via.into(), ..Default::default() }),
            ..Default::default()
        };
        let url =
            super::problem_report(&[agent("claude", "Ptyxis", "tmux"), agent("claude", "Ptyxis", "tmux"), agent("codex", "Code", "pty")]);
        assert!(url.starts_with("mailto:hey@october.dev?subject=October%20Lantern%20"));
        assert!(!url.contains('+') && !url.contains(' '));
        let body = url.split("&body=").nth(1).unwrap();
        let bytes = body.as_bytes();
        let mut raw = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                raw.push(u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap(), 16).unwrap());
                i += 3;
            } else {
                raw.push(bytes[i]);
                i += 1;
            }
        }
        let decoded = String::from_utf8(raw).unwrap();
        assert!(decoded.contains("Agents: Claude Code ×2, Codex ×1"), "{decoded}");
        assert!(decoded.contains("Terminals: Code via pty ×1, Ptyxis via tmux ×2"), "{decoded}");
        assert!(!decoded.contains("secret-project") && !decoded.contains("private words"));
    }
}

#[cfg(test)]
mod question_tests {
    #[test]
    fn a_question_is_answered_once_and_sent_comes_back_off_when_the_keys_did_not_go_in() {
        use crate::model::{Agent, AgentAnswer, AgentState, Route};
        let (engine, _) = crate::sidecar::test_sender();
        let mut lantern = super::Lantern::new(engine, None);
        lantern.connected = true;
        lantern.model.agents = vec![Agent {
            id: "claude:1:1".into(),
            handle: "claude-1".into(),
            state: AgentState::NeedsInput,
            question_kind: Some("plan".into()),
            prompt_id: Some("t-1".into()),
            route: Some(Route { via: "tmux".into(), ..Default::default() }),
            ..Default::default()
        }];
        lantern.answer_question("claude:1:1", "t-1", AgentAnswer::Yes);
        lantern.answer_question("claude:1:1", "t-1", AgentAnswer::No);
        assert_eq!(lantern.model.answered.get("t-1"), Some(&AgentAnswer::Yes), "the first answer only");
        assert_eq!(lantern.pending.len(), 1);
        // A question that has moved on isn't answered.
        lantern.answer_question("claude:1:1", "t-0", AgentAnswer::Yes);
        assert_eq!(lantern.pending.len(), 1);
        let id = lantern.pending.keys().next().unwrap().clone();
        lantern.reply_result(&serde_json::json!({"type": "replyResult", "requestId": id, "ok": false, "error": "prompt_changed",
            "message": "That prompt was already answered or replaced. Check the terminal."}));
        assert!(lantern.model.answered.is_empty());
    }
}
