//! Phone intents; pairing URLs never travel through generic Copy actions or logs.
use super::Lantern;
use crate::model::{
    PanelMode,
    phone::{Action, Phone},
};
use serde_json::{Value, json};

impl Lantern {
    pub(super) fn phone_event(&mut self, event: Value) {
        if self.model.account.signed_in.is_none() {
            return;
        }
        let Ok(next) = serde_json::from_value::<Phone>(event) else {
            return;
        };
        let before = self.model.phone.as_ref();
        let new_pairing = next
            .pairing
            .as_ref()
            .is_some_and(|p| before.is_none_or(|s| s.pairing.as_ref().is_none_or(|last| last.intent_id != p.intent_id)));
        let fresh = next.pairing.as_ref().is_some_and(|p| {
            p.code.is_some()
                && !p.finishing
                && before.is_none_or(|old| old.pairing.as_ref().is_none_or(|last| last.intent_id != p.intent_id || last.code != p.code))
        });
        let paired = next.outcome.as_deref() == Some("paired");
        if paired && before.is_none_or(|s| s.outcome.as_deref() != Some("paired")) {
            crate::analytics::capture("phone_paired", json!({}));
        }
        if !next.busy {
            self.view.phone_sending = false;
        }
        self.model.phone = Some(next);
        if new_pairing {
            self.view.phone_reveal = true;
        }
        if fresh {
            self.view.phone_reveal = true;
            self.windows.show_pill(true);
            self.set_panel(Some(PanelMode::October));
            self.windows.raise_panel();
        }
    }

    pub(super) fn phone_action(&mut self, action: Action) {
        if action == Action::Open {
            self.view.phone_reveal = true;
            self.set_panel(Some(PanelMode::October));
            if self.model.account.signed_in.is_some() && self.model.phone.as_ref().is_none_or(|s| s.pairing.is_none()) {
                self.phone_action(Action::Pair);
            }
            return;
        }
        if self.model.account.signed_in.is_none() {
            self.set_panel(Some(PanelMode::October));
            return;
        }
        if self.view.phone_sending || self.model.phone.as_ref().is_some_and(|p| p.busy) {
            return;
        }
        let request = match action {
            Action::Open => unreachable!(),
            Action::Pair => {
                self.view.phone_reveal = true;
                self.view.phone_copied = false;
                json!({"type":"phone.pair"})
            }
            Action::Cancel => {
                self.view.phone_copied = false;
                json!({"type":"phone.cancelPair"})
            }
            Action::Retry => json!({"type":"phone.retry"}),
            Action::CopyLink => {
                let Some(p) = self.model.phone.as_ref().and_then(|s| s.pairing.as_ref()) else {
                    return;
                };
                if p.remaining(Self::now_ms()) == 0 {
                    return;
                }
                if let Some(link) = &p.qr {
                    self.windows.copy(link.clone());
                    self.view.phone_copied = true;
                }
                return;
            }
            Action::Decide { intent_id, bind, code, allow } => {
                let Some(p) = self.model.phone.as_ref().and_then(|s| s.pairing.as_ref()) else {
                    return;
                };
                if p.finishing || p.intent_id != intent_id || p.bind.as_deref() != Some(&bind) || p.code.as_deref() != Some(&code) {
                    return;
                }
                json!({"type":"phone.decide","intentId":intent_id,"bind":bind,"code":code,"allow":allow})
            }
            Action::Remove(bind) => json!({"type":"phone.revoke","bind":bind}),
        };
        self.view.phone_error = None;
        if self.engine.send(&request).is_ok() {
            self.view.phone_sending = true;
        } else {
            self.view.phone_error = Some("Lantern's connection is restarting. Try again when it is ready.".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> Lantern {
        let (engine, _) = crate::sidecar::test_sender();
        let mut app = Lantern::new(engine, None);
        app.model.account.signed_in = Some(crate::model::Account::default());
        app
    }
    fn sent(app: &Lantern) -> Vec<Value> {
        let mut events = Vec::new();
        while let Ok(Ok(v)) = app.engine.events.recv_timeout(std::time::Duration::from_millis(100)) {
            events.push(v);
        }
        events
    }
    fn confirmation(code: &str) -> Value {
        json!({"type":"phone", "status":"connected", "pairing":{"intentId":"intent", "bind":"phone", "code":code, "expiresAt": 9999999999999_f64}})
    }
    #[test]
    fn a_new_confirmation_opens_the_phone_panel_and_an_old_click_cannot_approve_it() {
        let mut app = app();
        app.phone_event(confirmation("123456"));
        assert_eq!(app.model.panel, Some(PanelMode::October));
        assert!(app.windows.raising_panel());
        app.phone_event(confirmation("654321"));
        app.phone_action(Action::Decide { intent_id: "intent".into(), bind: "phone".into(), code: "123456".into(), allow: true });
        assert!(!app.view.phone_sending);
        assert!(sent(&app).is_empty());
        app.phone_action(Action::Decide { intent_id: "intent".into(), bind: "phone".into(), code: "654321".into(), allow: true });
        assert_eq!(sent(&app).last().unwrap()["code"], "654321");
    }
    #[test]
    fn signed_out_cta_routes_to_account_and_ignores_phone_events() {
        let mut app = app();
        app.model.account.signed_in = None;
        app.phone_action(Action::Open);
        app.phone_action(Action::Pair);
        app.phone_event(confirmation("123456"));
        assert_eq!(app.model.panel, Some(PanelMode::October));
        assert!(app.model.phone.is_none());
        assert!(sent(&app).is_empty());
    }
    #[test]
    fn connect_cta_creates_one_intent_and_reopening_preserves_a_live_comparison() {
        let mut app = app();
        app.phone_action(Action::Open);
        assert_eq!(app.model.panel, Some(PanelMode::October));
        assert_eq!(sent(&app), vec![json!({"type":"phone.pair"})]);
        app.phone_action(Action::Open);
        assert!(sent(&app).is_empty(), "a request is already in flight");
        app.view.phone_reveal = false;
        app.phone_event(json!({"type":"phone", "status":"connected", "pairing":{
            "intentId":"intent", "qr":"https://october.dev/pair#synthetic-test-only", "expiresAt":9999999999999_f64
        }}));
        assert!(app.view.phone_reveal, "scroll again when the asynchronous QR arrives");
        app.phone_action(Action::Open);
        assert!(sent(&app).is_empty(), "reopening preserves the QR too");
        app.phone_event(confirmation("123456"));
        app.phone_action(Action::Open);
        assert!(sent(&app).is_empty(), "opening the panel must not replace the phone's code");
        assert_eq!(app.model.phone.as_ref().unwrap().pairing.as_ref().unwrap().code.as_deref(), Some("123456"));
    }
    #[test]
    fn pairing_secret_is_absent_from_model_serialization_and_debug_output() {
        let mut app = app();
        let mut data = confirmation("123456");
        data["pairing"]["qr"] = json!("https://october.dev/pair#test-secret");
        app.phone_event(data);
        assert!(!format!("{:?}", app.model).contains("test-secret"));
        assert!(!serde_json::to_string(&app.model).unwrap().contains("test-secret"));
    }
}
