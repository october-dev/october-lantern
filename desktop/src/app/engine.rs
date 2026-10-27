//! Engine events into the model (Swift `AppModel` engine callbacks; protocol/README.md).
use super::{Lantern, Pending};
use crate::model::{Agent, ChatMessage, EngineHealth, HarnessStatus, ModelList, OctoberLink, inbox};
use serde_json::{Value, json};
use std::collections::HashSet;

fn text(event: &Value, key: &str) -> Option<String> {
    event[key].as_str().map(str::to_string)
}

impl Lantern {
    pub(super) fn engine_event(&mut self, event: Value) {
        match event["type"].as_str().unwrap_or("") {
            "hello" => {
                self.connected = event["protocol"] == 3;
                self.model.engine = if self.connected {
                    EngineHealth::Ready
                } else {
                    EngineHealth::Failed("this engine speaks a different protocol; reinstall Lantern".into())
                };
                self.turns.restart();
                if self.connected {
                    self.give_engine_sign_in();
                    self.invalidate_october_models();
                }
                // Agents can use this computer's apps: get Cua Driver ready (downloaded on first use).
                if self.connected && self.model.prefs.computer_use {
                    self.request(serde_json::json!({"type": "computer.prepare", "enabled": true}));
                }
            }
            "computer" => {
                if let Ok(status) = serde_json::from_value::<crate::model::ComputerStatus>(event.clone()) {
                    self.model.computer = Some(status);
                }
            }
            "snapshot" => self.snapshot(&event),
            "installed" => {
                let kinds: Vec<String> =
                    event["installed"]["kinds"].as_array().into_iter().flatten().filter_map(|k| k.as_str().map(str::to_string)).collect();
                // Most recently used first, then the engine's order.
                let recent = &self.model.prefs.recent_kinds;
                let mut ordered: Vec<String> = recent.iter().filter(|kind| kinds.contains(kind)).cloned().collect();
                ordered.extend(kinds.into_iter().filter(|kind| !recent.contains(kind)));
                self.model.new_session.installed = Some(ordered.into_iter().map(crate::model::AgentKind).collect());
                self.model.new_session.signed_in = event["installed"]["signedIn"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|k| k.as_str().map(crate::model::AgentKind::new))
                    .collect();
                self.model.new_session.never_set_up = event["installed"]["neverSetUp"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|k| k.as_str().map(crate::model::AgentKind::new))
                    .collect();
                // Windows runs Lantern's own sessions without tmux.
                self.model.new_session.in_lantern_available = event["installed"]["tmux"] == true || cfg!(windows);
                if let Ok(harness) = serde_json::from_value::<HarnessStatus>(event["harness"].clone()) {
                    self.model.harness = Some(harness);
                }
                // Without Claude Code or Codex, October Harness is what can run: install it once,
                // in the background (Swift `installHarnessIfNoOtherAgent`).
                let status = self.model.harness.as_ref().map(|h| h.status);
                let october = self.model.new_session.installed.iter().flatten().any(|k| k.as_str() == crate::model::AgentKind::OCTOBER);
                if self.needs_harness()
                    && !october
                    && !matches!(
                        status,
                        Some(
                            crate::model::HarnessState::Ready | crate::model::HarnessState::Installing | crate::model::HarnessState::Failed
                        )
                    )
                {
                    self.request(serde_json::json!({"type": "harness.prepare"}));
                }
            }
            "harness" => {
                if let Ok(harness) = serde_json::from_value::<HarnessStatus>(event.clone()) {
                    self.model.harness = Some(harness);
                    self.harness_changed();
                }
            }
            "launchResult" => {
                if let Some(Pending::Launch) = text(&event, "requestId").and_then(|id| self.pending.remove(&id)) {
                    // A launch that worked can still carry a warning (e.g. it started without October Bus).
                    let message = text(&event, "message").or_else(|| text(&event, "warning"));
                    self.launch_result(event["ok"] == true, message, text(&event, "session"));
                }
            }
            "models" => {
                if let Some(Pending::Models { kind }) = text(&event, "requestId").and_then(|id| self.pending.remove(&id))
                    && event["kind"].as_str() == Some(kind.as_str())
                    && let Ok(list) = serde_json::from_value::<ModelList>(event.clone())
                {
                    self.model.new_session.models.insert(kind, list);
                }
            }
            "historyResult" => self.history_result(&event),
            "replyResult" | "sendResult" => self.reply_result(&event),
            "attachResult" => {
                if let Some(Pending::Focus { agent_id }) = text(&event, "requestId").and_then(|id| self.pending.remove(&id))
                    && event["ok"] != true
                {
                    let handle = self.model.agent(&agent_id).map_or("agent".to_string(), |agent| agent.handle.clone());
                    self.show(format!("Couldn't open @{handle}: {}", text(&event, "message").unwrap_or_else(|| "unknown error".into())));
                }
            }
            "october" => self.model.october = serde_json::from_value::<OctoberLink>(event["october"].clone()).ok(),
            "computerHold" => {
                self.model.control_held = event["held"] == true;
                self.model.control_message = event["message"].as_str().map(str::to_string);
                if self.model.control_held {
                    self.stop_show();
                }
            }
            "controlResult" => {
                self.model.control_sending = false;
                self.model.control_message = event["message"].as_str().map(str::to_string);
            }
            "showCapture" | "showQuery" | "showState" => self.show_event(event.clone()),
            "routines" => {
                self.model.routines = event["items"].as_array().cloned().unwrap_or_default();
                if let Some(error) = event["error"].as_str() {
                    self.model.routine_error = Some(error.to_string());
                }
            }
            "routineResult" => {
                if self.model.routine_pending.as_ref().is_some_and(|(id, _)| Some(id.as_str()) == event["requestId"].as_str()) {
                    let (_, save) = self.model.routine_pending.take().unwrap();
                    self.model.routine_error = event["message"].as_str().map(str::to_string);
                    if save && event["ok"] == true {
                        self.view.routine_form.open = false;
                    }
                }
            }
            "cards" => {
                self.model.launches = serde_json::from_value(event["launches"].clone()).unwrap_or_default();
                let open: HashSet<String> = self.model.launches.iter().flat_map(|l| l.waiting()).map(|c| c.id.clone()).collect();
                self.model.card_sending.retain(|id| open.contains(id));
                self.model.inbox = self.turns.inbox(&self.model.agents, &self.model.prefs, &self.model.carded());
            }
            "cardResult" => {
                if let Some(card) = text(&event, "cardId") {
                    self.model.card_sending.remove(&card);
                    if event["ok"] == true {
                        self.model.card_errors.remove(&card);
                    } else {
                        let message = text(&event, "message").unwrap_or_else(|| "Lantern couldn't save your answer.".into());
                        self.model.card_errors.insert(card, message);
                    }
                }
            }
            "toolkit" => self.toolkit_result(&event),
            _ => {}
        }
    }

    fn snapshot(&mut self, event: &Value) {
        let mut agents: Vec<Agent> =
            event["agents"].as_array().into_iter().flatten().filter_map(|agent| serde_json::from_value(agent.clone()).ok()).collect();
        inbox::rank(&mut agents);
        // TODO(apollo): notifications for these, through the legacy notifier.
        let _fresh = self.turns.observe(&agents, &self.model.prefs, Self::now_ms());
        let live: HashSet<String> = agents.iter().map(|agent| agent.id.clone()).collect();
        self.inputs.drafts.prune(&live);
        // "Sent" stays on a question's card only while the agent is still on it.
        let prompts: HashSet<String> = agents.iter().filter_map(|agent| agent.prompt_id.clone()).collect();
        self.model.answered.retain(|prompt, _| prompts.contains(prompt));
        self.model.answer_delivery.retain(|prompt, _| prompts.contains(prompt));
        self.model.agents = agents;
        self.model.inbox = self.turns.inbox(&self.model.agents, &self.model.prefs, &self.model.carded());
        self.refresh_folders();
        self.open_launched();
        self.track_tasks();
        // Keep an open conversation current; close it when its agent is gone.
        if let Some(chat) = &self.model.chat {
            if live.contains(&chat.agent_id) {
                self.refresh_chat();
            } else {
                self.close_chat();
            }
        }
    }

    pub(super) fn refresh_chat(&mut self) {
        self.history_at = std::time::Instant::now();
        let Some(agent_id) = self.model.chat.as_ref().map(|chat| chat.agent_id.clone()) else { return };
        // A newer request replaces the one in flight; its answer will be ignored.
        if let Some(old) = self.history_request.take() {
            self.pending.remove(&old);
        }
        if let Some(id) = self.request(json!({"type": "history", "agentId": agent_id})) {
            self.pending.insert(id.clone(), Pending::History);
            self.history_request = Some(id);
        }
    }

    fn history_result(&mut self, event: &Value) {
        let Some(id) = text(event, "requestId") else { return };
        if self.asking.watch_request.as_deref() == Some(id.as_str()) {
            self.asking.watch_request = None;
            self.pending.remove(&id);
            return self.watch_result(event);
        }
        if self.history_request.as_deref() != Some(id.as_str()) {
            return; // stale: a newer request (or another chat) took over
        }
        self.history_request = None;
        self.pending.remove(&id);
        let Some(chat) = self.model.chat.as_mut() else { return };
        if text(event, "agentId").as_deref() != Some(chat.agent_id.as_str()) || event["busy"] == true {
            return;
        }
        let messages: Vec<ChatMessage> = serde_json::from_value(event["messages"].clone()).unwrap_or_default();
        let supported = event["supported"] != false;
        chat.loading = false;
        chat.supported = supported;
        chat.messages = messages;
        // The real conversation takes over from the preview once it has something to show.
        if self.model.launch_preview.is_some() && (!chat.messages.is_empty() || !supported) {
            self.model.launch_preview = None;
        }
    }

    pub(super) fn reply_result(&mut self, event: &Value) {
        let Some(id) = text(event, "requestId") else { return };
        let ok = event["ok"] == true;
        match self.pending.remove(&id) {
            Some(Pending::Reply(ticket)) => {
                self.model.sending = self.pending.values().any(|p| matches!(p, Pending::Reply(_)));
                let handle = self.model.agent(&ticket.recipient).map_or("agent".to_string(), |agent| agent.handle.clone());
                if ok {
                    self.inputs.drafts.sent(&ticket);
                } else if event["error"] == "uncertain" {
                    self.show(format!("@{handle} may not have received it. Check its terminal before sending again."));
                } else {
                    self.show(format!("@{handle}: {}", text(event, "message").unwrap_or_else(|| "not sent; your draft is kept".into())));
                }
            }
            Some(Pending::Send { turn }) => {
                let message = text(event, "message")
                    .or_else(|| (event["error"] == "uncertain").then(|| "Lantern isn't sure it went in. Check its terminal.".to_string()));
                self.task_message_sent(turn, ok, message);
            }
            Some(Pending::Direct { agent_id }) if !ok => {
                let handle = self.model.agent(&agent_id).map_or("agent".to_string(), |agent| agent.handle.clone());
                self.show(format!("@{handle}: {}", text(event, "message").unwrap_or_else(|| "not sent".into())));
            }
            Some(Pending::Keys { prompt_id: Some(prompt_id), .. }) if ok => {
                self.model.answer_delivery.insert(prompt_id, crate::model::AnswerDelivery::Sent);
            }
            Some(Pending::Keys { agent_id, prompt_id }) if !ok => {
                if let Some(prompt_id) = prompt_id {
                    if event["error"] == "uncertain" {
                        self.model.answer_delivery.insert(prompt_id, crate::model::AnswerDelivery::Unconfirmed);
                    } else {
                        self.model.answered.remove(&prompt_id);
                        self.model.answer_delivery.remove(&prompt_id);
                    }
                }
                let handle = self.model.agent(&agent_id).map_or("agent".to_string(), |agent| agent.handle.clone());
                self.show(format!("@{handle}: {}", text(event, "message").unwrap_or_else(|| "couldn't answer".into())));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod routine_receipt_tests {
    use super::*;
    #[test]
    fn routine_form_stays_open_on_failure_and_ignores_unrelated_receipts() {
        let (engine, _) = crate::sidecar::test_sender();
        let mut app = Lantern::new(engine, None);
        app.view.routine_form.open = true;
        app.view.routine_form.name = "Keep my draft".into();
        app.model.routine_pending = Some(("save-1".into(), true));
        app.engine_event(json!({"type":"routineResult","requestId":"other","ok":true}));
        assert!(app.model.routine_pending.is_some());
        app.engine_event(json!({"type":"routineResult","requestId":"save-1","ok":false,"message":"Choose a valid folder"}));
        assert!(app.model.routine_pending.is_none());
        assert!(app.view.routine_form.open);
        assert_eq!(app.view.routine_form.name, "Keep my draft");
        assert_eq!(app.model.routine_error.as_deref(), Some("Choose a valid folder"));
        app.model.routine_pending = Some(("save-2".into(), true));
        app.engine_event(json!({"type":"routineResult","requestId":"save-2","ok":true}));
        assert!(!app.view.routine_form.open);
    }
}
