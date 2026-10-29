//! The local engine's phone state, kept out of saved model/settings data.
use serde::Deserialize;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Phone {
    pub status: String,
    pub message: Option<String>,
    pub error_code: Option<String>,
    pub outcome: Option<String>,
    pub devices: Vec<Device>,
    pub pairing: Option<Pairing>,
    pub busy: bool,
    pub install_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Device {
    pub bind: String,
    pub label: String,
    pub platform: String,
    pub last_seen_at: Option<f64>,
    pub removal_pending: bool,
}

#[derive(Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pairing {
    pub intent_id: String,
    pub bind: Option<String>,
    pub qr: Option<String>,
    pub expires_at: f64,
    pub code: Option<String>,
    pub label: Option<String>,
    pub platform: Option<String>,
    pub finishing: bool,
}

impl std::fmt::Debug for Pairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pairing").field("finishing", &self.finishing).finish_non_exhaustive()
    }
}
impl Pairing {
    pub fn remaining(&self, now: f64) -> u64 {
        ((self.expires_at - now) / 1000.0).ceil().max(0.0) as u64
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Open,
    Pair,
    Cancel,
    Retry,
    CopyLink,
    Decide { intent_id: String, bind: String, code: String, allow: bool },
    Remove(String),
}

pub fn recovery(code: Option<&str>, outcome: Option<&str>) -> &'static str {
    match code {
        Some("plan_required") => "Upgrade",
        Some("SESSION_REQUIRED") => "Sign in again",
        Some("PAIRING_EXPIRED" | "PAIRING_USED" | "PAIRING_REJECTED" | "PAIRING_DISCONNECTED") => "New code",
        Some("PAIRING_DENIED") => "Start again",
        _ if outcome == Some("denied") => "Start again",
        _ => "Retry",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_expiry_and_distinct_recovery_actions() {
        let p = Pairing { expires_at: 100_000.0, ..Default::default() };
        assert_eq!(p.remaining(99_001.0), 1);
        assert_eq!(p.remaining(100_000.0), 0);
        assert_eq!(p.remaining(101_000.0), 0);
        assert_eq!(recovery(Some("plan_required"), None), "Upgrade");
        assert_eq!(recovery(Some("plan_authority_unavailable"), None), "Retry");
        assert_eq!(recovery(Some("PAIRING_USED"), None), "New code");
        assert_eq!(recovery(None, Some("denied")), "Start again");
    }
}
