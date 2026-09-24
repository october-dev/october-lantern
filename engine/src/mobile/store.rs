//! Lantern's identity as an October host, and the phones paired with it.
//!
//! Kept in `~/Library/Application Support/October Lantern/phone/` (folder 0700, files 0600):
//! - `host.json`: hostId, Ed25519 signing seed, X25519 static key pair, the synthetic canvas id.
//!   October's servers pin these per hostId, so they must stay stable.
//! - `devices.json`: paired phones, with only a SHA-256 hash of each phone's credential.

use std::fs;
use std::os::unix::fs::PermissionsExt;
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
    fs::set_permissions(&d, fs::Permissions::from_mode(0o700))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
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

/// Stores a newly paired phone and returns the credential to hand it (only its hash is kept).
pub fn add_device(bind: &str, label: &str, platform: &str) -> Result<String> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| anyhow::anyhow!("{e}"))?;
    let credential = B64.encode(raw);
    let mut devices = load_devices();
    devices.retain(|d| d.bind != bind);
    devices.push(Device {
        bind: bind.into(),
        label: label.into(),
        platform: platform.into(),
        credential_hash: hash_credential(&credential),
        paired_at: crate::hooks::now_ms(),
    });
    save_devices(&devices)?;
    Ok(credential)
}

pub fn remove_device(bind: &str) -> Result<()> {
    let mut devices = load_devices();
    devices.retain(|d| d.bind != bind);
    save_devices(&devices)
}

/// Constant-time check of a phone's credential against the stored hash.
pub fn credential_ok(bind: &str, credential: &str) -> bool {
    let h = hash_credential(credential);
    load_devices().iter().any(|d| d.bind == bind && bool::from(d.credential_hash.as_bytes().ct_eq(h.as_bytes())))
}

pub fn decode_key(value: &str) -> Option<[u8; 32]> {
    let bytes = B64.decode(value.trim_end_matches('=')).ok()?;
    <[u8; 32]>::try_from(bytes).ok()
}
