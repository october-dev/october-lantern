//! Lantern's identity as an October host, and the phones paired with it.
//!
//! Kept in `~/Library/Application Support/October Lantern/phone/` (folder 0700, files 0600):
//! - `host.json`: hostId, Ed25519 signing seed, X25519 static key pair, the synthetic canvas id.
//!   October's servers pin these per hostId, so they must stay stable.
//! - `devices.json`: paired phones, with only a SHA-256 hash of each phone's credential, and
//!   each phone's Expo push token and mute switch once it registers for notifications.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

fn dir() -> PathBuf {
    crate::hooks::support_dir().join("phone")
}

fn write_private(path: &PathBuf, bytes: &[u8]) -> Result<()> {
    let d = dir();
    fs::create_dir_all(&d)?;
    crate::platform::set_mode(&d, 0o700)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    crate::platform::set_mode(&tmp, 0o600)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HostIdentity {
    pub host_id: String,
    sign_seed: String,
    static_secret: String,
    pub static_public: String,
    pub canvas_id: String,
    /// Set once October's servers know this host (after the first pairing).
    #[serde(default)]
    pub registered: bool,
}

impl HostIdentity {
    /// Loads the identity, refusing (rather than silently replacing) a corrupt one: October's
    /// servers pin these keys, so a fresh identity would be a different host.
    pub fn load_or_create() -> Result<HostIdentity> {
        let path = dir().join("host.json");
        if let Ok(bytes) = fs::read(&path) {
            let id: HostIdentity = serde_json::from_slice(&bytes).context("phone/host.json is unreadable")?;
            if decode_key(&id.sign_seed).is_none() || decode_key(&id.static_secret).is_none() {
                bail!("phone/host.json holds invalid keys; move it aside to pair again as a new host");
            }
            return Ok(id);
        }
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|e| anyhow::anyhow!("{e}"))?;
        let (secret, public) = super::noise::generate_static()?;
        let id = HostIdentity {
            host_id: uuid::Uuid::new_v4().hyphenated().to_string(),
            sign_seed: B64.encode(seed),
            static_secret: B64.encode(secret),
            static_public: B64.encode(public),
            canvas_id: uuid::Uuid::new_v4().hyphenated().to_string(),
            registered: false,
        };
        id.save()?;
        Ok(id)
    }

    pub fn save(&self) -> Result<()> {
        write_private(&dir().join("host.json"), &serde_json::to_vec_pretty(self)?)
    }

    pub fn signing_key(&self) -> ed25519_dalek::SigningKey {
        // Checked in `load_or_create`.
        ed25519_dalek::SigningKey::from_bytes(&decode_key(&self.sign_seed).unwrap_or_default())
    }

    pub fn sign_public(&self) -> String {
        B64.encode(self.signing_key().verifying_key().to_bytes())
    }

    pub fn static_secret(&self) -> [u8; 32] {
        decode_key(&self.static_secret).unwrap_or_default()
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub bind: String,
    pub label: String,
    pub platform: String,
    credential_hash: String,
    pub paired_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<u64>,
    /// Access is blocked locally even if the server couldn't yet confirm removal.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removal_pending: bool,
    /// The phone's Expo push token, set by `remote.push.register`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_token: Option<String>,
    /// The phone turned agent notifications off (`remote.push.mute`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
}

/// Serializes read-modify-write of `devices.json` (the host thread and push senders both write).
static DEVICES_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn edit_devices(change: impl FnOnce(&mut Vec<Device>) -> Result<()>) -> Result<()> {
    let _guard = DEVICES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut devices = load_devices();
    change(&mut devices)?;
    save_devices(&devices)
}

/// Changes a paired phone's push settings; refused for a phone that isn't paired (or was revoked).
fn edit_push(bind: &str, change: impl FnOnce(&mut Device)) -> Result<()> {
    edit_devices(|devices| match devices.iter_mut().find(|d| d.bind == bind) {
        Some(d) if !d.removal_pending => {
            change(d);
            Ok(())
        }
        _ => bail!("this phone isn't paired"),
    })
}

pub fn register_push(bind: &str, token: &str) -> Result<()> {
    let token = token.to_string();
    edit_push(bind, |d| d.push_token = Some(token))
}

pub fn unregister_push(bind: &str) -> Result<()> {
    edit_push(bind, |d| d.push_token = None)
}

pub fn mute_push(bind: &str, muted: bool) -> Result<()> {
    edit_push(bind, |d| d.muted = muted)
}

/// Clears a token Expo said is no longer registered, unless the phone has since registered another.
pub fn forget_push_token(bind: &str, token: &str) -> Result<()> {
    edit_devices(|devices| {
        if let Some(d) = devices.iter_mut().find(|d| d.bind == bind && d.push_token.as_deref() == Some(token)) {
            d.push_token = None;
        }
        Ok(())
    })
}

/// (bind, Expo token) for every paired phone that wants notifications.
pub fn push_targets() -> Vec<(String, String)> {
    push_targets_from(load_devices())
}

fn push_targets_from(devices: Vec<Device>) -> Vec<(String, String)> {
    devices.into_iter().filter(|d| !d.muted && !d.removal_pending).filter_map(|d| Some((d.bind, d.push_token?))).collect()
}

fn credential_matches(devices: &[Device], bind: &str, credential: &str) -> bool {
    let h = hash_credential(credential);
    devices.iter().any(|d| !d.removal_pending && d.bind == bind && bool::from(d.credential_hash.as_bytes().ct_eq(h.as_bytes())))
}

pub fn hash_credential(credential: &str) -> String {
    Sha256::digest(credential.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn devices_path() -> std::path::PathBuf {
    dir().join("devices.json")
}

pub fn load_devices() -> Vec<Device> {
    fs::read(devices_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_devices(devices: &[Device]) -> Result<()> {
    write_private(&dir().join("devices.json"), &serde_json::to_vec_pretty(devices)?)
}

/// An ephemeral credential for an approved phone, before it acknowledges and finalizes pairing.
pub fn new_credential() -> Result<String> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(B64.encode(raw))
}

/// Store only after finalization; pending phones never appear as connected after a restart.
pub fn add_device(bind: &str, label: &str, platform: &str, credential: &str) -> Result<()> {
    edit_devices(|devices| {
        devices.retain(|d| d.bind != bind);
        devices.push(Device {
            bind: bind.into(),
            label: label.into(),
            platform: platform.into(),
            credential_hash: hash_credential(credential),
            paired_at: crate::hooks::now_ms(),
            last_seen_at: None,
            removal_pending: false,
            push_token: None,
            muted: false,
        });
        Ok(())
    })
}

/// Forgets a phone, and with it its push token: a revoked phone gets no more notifications.
pub fn remove_device(bind: &str) -> Result<()> {
    edit_devices(|devices| {
        devices.retain(|d| d.bind != bind);
        Ok(())
    })
}

pub fn block_device(bind: &str) -> Result<()> {
    edit_devices(|devices| {
        if let Some(d) = devices.iter_mut().find(|d| d.bind == bind) {
            d.removal_pending = true;
            d.push_token = None;
        }
        Ok(())
    })
}

/// Authenticated activity only; coalesce disk writes while the phone is active.
pub fn seen(bind: &str, now: u64) -> Result<()> {
    let _guard = DEVICES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut devices = load_devices();
    if let Some(d) = devices.iter_mut().find(|d| d.bind == bind && !d.removal_pending)
        && d.last_seen_at.is_none_or(|last| now.saturating_sub(last) >= 60_000)
    {
        d.last_seen_at = Some(now);
        save_devices(&devices)?;
    }
    Ok(())
}

/// Constant-time check of a phone's credential against the stored hash.
pub fn credential_ok(bind: &str, credential: &str) -> bool {
    credential_matches(&load_devices(), bind, credential)
}

pub fn decode_key(value: &str) -> Option<[u8; 32]> {
    let bytes = B64.decode(value.trim_end_matches('=')).ok()?;
    <[u8; 32]>::try_from(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_blocks_credentials_and_push_before_the_cloud_confirms() {
        let mut d: Device = serde_json::from_value(serde_json::json!({"bind":"phone", "label":"Test phone", "platform":"ios",
            "credentialHash":hash_credential("secret"), "pairedAt":1, "pushToken":"ExponentPushToken[test]"}))
        .unwrap();
        assert!(credential_matches(&[d.clone()], "phone", "secret"));
        assert_eq!(push_targets_from(vec![d.clone()]).len(), 1);
        d.removal_pending = true;
        assert!(!credential_matches(&[d.clone()], "phone", "secret"));
        assert!(push_targets_from(vec![d.clone()]).is_empty());
        assert!(!serde_json::to_string(&d).unwrap().contains("secret"), "only a credential hash is persisted");
    }

    #[test]
    fn devices_from_older_lanterns_load_without_push_settings() {
        let old = r#"{"bind":"b","label":"Phone","platform":"ios","credentialHash":"h","pairedAt":1}"#;
        let d: Device = serde_json::from_str(old).unwrap();
        assert_eq!((d.push_token.as_deref(), d.muted), (None, false));
        assert_eq!(serde_json::to_value(&d).unwrap(), serde_json::from_str::<serde_json::Value>(old).unwrap());
        let d = Device { push_token: Some("ExponentPushToken[x]".into()), muted: true, ..d };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!((v["pushToken"].as_str(), v["muted"].as_bool()), (Some("ExponentPushToken[x]"), Some(true)));
    }
}
