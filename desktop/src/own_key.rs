//! Your own AI key (Swift `OwnKey`): an OpenAI, Anthropic or OpenRouter key kept in the system
//! keyring as one JSON item, its own entry beside the October sign-in. October Ask requests carry
//! it so your provider answers (and bills you); October passes it on and never stores it.
//!
//! It never goes to settings, logs, reports, analytics or the `Model`: the app holds it in memory,
//! the model only knows the provider, its last four characters and the model id, and `Debug` hides
//! it. Signing out keeps it (it's yours, on this computer); uninstalling removes it.
use crate::os::{Event, Secret};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::Sender;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    #[default]
    OpenAI,
    Anthropic,
    OpenRouter,
}

impl AiProvider {
    pub const ALL: [Self; 3] = [Self::OpenAI, Self::Anthropic, Self::OpenRouter];

    /// What October's endpoint calls it (`X-Lantern-Byok-Provider`).
    pub fn id(self) -> &'static str {
        match self {
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
            Self::OpenRouter => "openrouter",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::OpenRouter => "OpenRouter",
        }
    }
}

/// The keyring item: `{"provider":"openai","key":"…","model":"…"}`, saved and read whole.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnKey {
    pub provider: AiProvider,
    pub key: String,
    /// Empty: the provider's default on October's server.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
}

impl std::fmt::Debug for OwnKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnKey").field("provider", &self.provider).field("key", &"[hidden]").field("model", &self.model).finish()
    }
}

impl OwnKey {
    /// What was typed, trimmed; `None` without a key.
    pub fn new(provider: AiProvider, key: &str, model: &str) -> Option<Self> {
        let key = key.trim();
        (!key.is_empty()).then(|| Self { provider, key: key.into(), model: model.trim().into() })
    }

    /// "…abcd": enough to recognise the key, never the key.
    pub fn ending(&self) -> String {
        let chars: Vec<char> = self.key.chars().collect();
        format!("…{}", chars[chars.len().saturating_sub(4)..].iter().collect::<String>())
    }

    fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    fn from_json(text: &str) -> Option<Self> {
        serde_json::from_str::<Self>(text).ok().filter(|key| !key.key.trim().is_empty())
    }
}

/// Whether your key may answer: the plan doesn't say it can't (a plan that doesn't say leaves it
/// to October's server).
pub fn allowed(rights: Option<&crate::model::LanternRights>) -> bool {
    rights.is_none_or(|rights| rights.byok)
}

/// The key as last read or saved, shared with the threads that ask: `None` until the keyring's
/// first read, so a question asked at startup waits for it rather than going to October's AI.
#[derive(Default)]
pub struct Shared {
    key: std::sync::Mutex<Option<Option<OwnKey>>>,
    read: std::sync::Condvar,
}

impl Shared {
    /// Already read (tests, and fixtures with no keyring).
    pub fn read_as(key: Option<OwnKey>) -> Self {
        Self { key: std::sync::Mutex::new(Some(key)), read: std::sync::Condvar::new() }
    }

    fn set(&self, key: Option<OwnKey>) {
        *self.key.lock().unwrap_or_else(|e| e.into_inner()) = Some(key);
        self.read.notify_all();
    }

    fn loaded(&self) -> bool {
        self.key.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// The key, once the keyring has been read (at most 20 seconds' wait: a keyring asking to be
    /// unlocked; then October's AI answers).
    pub fn wait(&self) -> Option<OwnKey> {
        let key = self.key.lock().unwrap_or_else(|e| e.into_inner());
        let (key, _) =
            self.read.wait_timeout_while(key, std::time::Duration::from_secs(20), |key| key.is_none()).unwrap_or_else(|e| e.into_inner());
        key.clone().flatten()
    }
}

/// What a question may use, decided when it's asked (the plan) and read in its own thread (the
/// keyring, perhaps still being read).
pub struct ForAsk {
    pub shared: std::sync::Arc<Shared>,
    /// The plan doesn't say your key can't be used.
    pub allowed: bool,
}

impl ForAsk {
    pub fn key(&self, request: &crate::october_ai::Request) -> Option<OwnKey> {
        (self.allowed && request.use_own_key).then(|| self.shared.wait()).flatten()
    }
}

pub enum Command {
    Load,
    Save(OwnKey),
    Remove,
}

/// The keyring is used off the frame loop (Secret Service can block, or ask to be unlocked), one
/// command at a time and in order; each result arrives as `Event::OwnKey`.
pub fn keeper(events: Sender<Event>, shared: std::sync::Arc<Shared>) -> Sender<Command> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for command in rx {
            let result = match command {
                // An item that isn't a key Lantern can read counts as none; saving replaces it.
                Command::Load => crate::os::secret_lookup(Secret::AiKey).map(|text| text.as_deref().and_then(OwnKey::from_json)),
                Command::Save(key) => crate::os::secret_store(Secret::AiKey, &key.to_json()).map(|()| {
                    crate::analytics::capture("own_key_saved", serde_json::json!({"provider": key.provider.id()}));
                    Some(key)
                }),
                Command::Remove => crate::os::secret_clear(Secret::AiKey).map(|()| {
                    crate::analytics::capture("own_key_removed", serde_json::json!({}));
                    None
                }),
            };
            match &result {
                Ok(key) => shared.set(key.clone()),
                // An unreadable keyring at startup: questions go to October's AI, not wait.
                Err(_) if !shared.loaded() => shared.set(None),
                Err(_) => {}
            }
            if events.send(Event::OwnKey(result.map_err(|error| format!("{error:#}")))).is_err() {
                break;
            }
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyring_item_round_trips_and_debug_hides_the_key() {
        let key = OwnKey::new(AiProvider::OpenRouter, "  sk-or-v1-secretabcd \n", " openai/gpt-4.1 ").unwrap();
        let json = key.to_json();
        assert_eq!(json, r#"{"provider":"openrouter","key":"sk-or-v1-secretabcd","model":"openai/gpt-4.1"}"#);
        assert_eq!(OwnKey::from_json(&json), Some(key.clone()));
        let plain = OwnKey::new(AiProvider::OpenAI, "sk-1234", "").unwrap();
        assert_eq!(plain.to_json(), r#"{"provider":"openai","key":"sk-1234"}"#, "no model: the server's default");
        assert_eq!(OwnKey::from_json(&plain.to_json()), Some(plain));
        assert_eq!(OwnKey::from_json(r#"{"provider":"anthropic","key":"  "}"#), None);
        assert_eq!(OwnKey::from_json("not json"), None);
        assert_eq!(OwnKey::new(AiProvider::Anthropic, "   ", "x"), None);
        let debug = format!("{key:?}");
        assert!(!debug.contains("secret") && debug.contains("[hidden]") && debug.contains("OpenRouter"), "{debug}");
        assert_eq!(key.ending(), "…abcd");
        assert_eq!(OwnKey::new(AiProvider::OpenAI, "ab", "").unwrap().ending(), "…ab");
    }

    #[test]
    fn the_key_is_used_unless_the_plan_says_it_cant_be() {
        let rights = |byok| crate::model::LanternRights { byok, ..Default::default() };
        assert!(allowed(None), "no plan yet: October decides");
        assert!(allowed(Some(&rights(true))));
        assert!(!allowed(Some(&rights(false))));
        let key = OwnKey::new(AiProvider::OpenAI, "sk-1", "").unwrap();
        let ask = |allowed| ForAsk { shared: std::sync::Arc::new(Shared::read_as(Some(key.clone()))), allowed };
        let request = crate::october_ai::Request::default();
        assert_eq!(ask(true).key(&request), Some(key.clone()));
        assert_eq!(ask(false).key(&request), None, "the plan says no");
        let october = crate::october_ai::Request { use_own_key: false, ..Default::default() };
        assert_eq!(ask(true).key(&october), None, "Ask October's AI instead");
    }

    #[test]
    fn a_question_waits_for_the_keyrings_first_read() {
        let shared = std::sync::Arc::new(Shared::default());
        let key = OwnKey::new(AiProvider::OpenAI, "sk-1", "").unwrap();
        let reader = {
            let shared = shared.clone();
            std::thread::spawn(move || shared.wait())
        };
        std::thread::sleep(std::time::Duration::from_millis(50));
        shared.set(Some(key.clone()));
        assert_eq!(reader.join().unwrap(), Some(key));
    }
}
