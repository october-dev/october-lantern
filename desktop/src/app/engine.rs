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
                if let (Some(kind), Ok(list)) = (text(&event, "kind"), serde_json::from_value::<ModelList>(event.clone())) {
                    self.model.new_session.models.insert(kind, list);
                }
            }
            "historyResult" => self.history_result(&event),
            "replyResult" => self.reply_result(&event),
            "attachResult" => {
                if let Some(Pending::Focus { agent_id }) = text(&event, "requestId").and_then(|id| self.pending.remove(&id))
                    && event["ok"] != true
                {
                    let handle = self.model.agent(&agent_id).map_or("agent".to_string(), |agent| agent.handle.clone());
                    self.show(format!("Couldn't open @{handle}: {}", text(&event, "message").unwrap_or_else(|| "unknown error".into())));
                }
            }
            "october" => self.model.october = serde_json::from_value::<OctoberLink>(event["october"].clone()).ok(),
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
        self.model.agents = agents;
        self.model.inbox = self.turns.inbox(&self.model.agents, &self.model.prefs);
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

    fn reply_result(&mut self, event: &Value) {
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
            Some(Pending::Direct { agent_id }) if !ok => {
                let handle = self.model.agent(&agent_id).map_or("agent".to_string(), |agent| agent.handle.clone());
                self.show(format!("@{handle}: {}", text(event, "message").unwrap_or_else(|| "not sent".into())));
            }
            Some(Pending::Keys { agent_id }) if !ok => {
                let handle = self.model.agent(&agent_id).map_or("agent".to_string(), |agent| agent.handle.clone());
                self.show(format!("@{handle}: {}", text(event, "message").unwrap_or_else(|| "couldn't answer".into())));
            }
            _ => {}
        }
    }
}
