//! October's control plane for phones: Supabase edge functions (`mobile-*`) with requests signed
//! by the host's Ed25519 key (`october-mobile/v1`), and Supabase REST for pairing rows.

use std::time::Duration;

use anyhow::{Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_dalek::Signer;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::store::HostIdentity;
use crate::model::iso;

pub const SUPABASE: &str = "https://latwxiqjgvluiddckvmj.supabase.co";
pub const ANON_KEY: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImxhdHd4aXFqZ3ZsdWlkZGNrdm1qIiwicm9sZSI6ImFub24iLCJpYXQiOjE3NTAzMDExMzEsImV4cCI6MjA2NTg3NzEzMX0.Aor5lE6ZSvv83Or_CxQNUdRzRUqit3fODkNSJpcDJ7E";
pub const RELAY: &str = "wss://relay.afteroctober.xyz";

/// A failure from October's servers, with its `code` when there is one (e.g. `plan_required`).
#[derive(Debug)]
pub struct ControlError {
    pub status: u16,
    pub code: Option<String>,
    pub message: String,
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code.as_deref().unwrap_or(&self.status.to_string()))
    }
}

impl std::error::Error for ControlError {}

/// The `session_id` claim of a Supabase access token.
pub fn session_id(access_token: &str) -> Option<String> {
    let payload = access_token.split('.').nth(1)?;
    let bytes = B64.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()?["session_id"].as_str().map(String::from)
}

/// `october-mobile/v1\n{operation}\n{sessionId}\n{requestId}\n{b64url(sha256(body))}\n{ts}`
pub fn signed_message(operation: &str, body: &[u8], session_id: &str, request_id: &str, ts: u64) -> String {
    format!("october-mobile/v1\n{operation}\n{session_id}\n{request_id}\n{}\n{ts}", B64.encode(Sha256::digest(body)))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(15))).build().into()
}

fn read(mut resp: ureq::http::Response<ureq::Body>) -> Result<Value> {
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let code = value["code"].as_str().or(value["error"].as_str()).map(String::from);
        let message = value["message"].as_str().or(value["msg"].as_str()).unwrap_or("request failed").to_string();
        return Err(ControlError { status, code, message }.into());
    }
    Ok(value)
}

/// POST to an edge function, signed as `operation` with the host's key.
pub fn signed_post(function: &str, operation: &str, body: &Value, id: &HostIdentity, access_token: &str) -> Result<Value> {
    let Some(session) = session_id(access_token) else { bail!("October session is missing its session id") };
    let raw = serde_json::to_vec(body)?;
    let request_id = uuid::Uuid::new_v4().hyphenated().to_string();
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let sig = id.signing_key().sign(signed_message(operation, &raw, &session, &request_id, ts).as_bytes());
    let resp = agent()
        .post(&format!("{SUPABASE}/functions/v1/{function}"))
        .header("authorization", &format!("Bearer {access_token}"))
        .header("apikey", ANON_KEY)
        .header("content-type", "application/json")
        .header("x-october-request-id", &request_id)
        .header("x-october-request-ts", &ts.to_string())
        .header("x-october-signature", &B64.encode(sig.to_bytes()))
        .send(&raw[..])?;
    read(resp)
}

fn rest_get(path: &str, access_token: &str) -> Result<Value> {
    let resp = agent()
        .get(&format!("{SUPABASE}/rest/v1/{path}"))
        .header("authorization", &format!("Bearer {access_token}"))
        .header("apikey", ANON_KEY)
        .header("accept", "application/json")
        .call()?;
    read(resp)
}

pub fn relay_ticket(id: &HostIdentity, access_token: &str) -> Result<String> {
    let v = signed_post(
        "mobile-relay-ticket",
        "ticket-host",
        &json!({"role": "host", "hostId": id.host_id, "key": id.static_public}),
        id,
        access_token,
    )?;
    v["ticket"].as_str().map(String::from).ok_or_else(|| anyhow::anyhow!("ticket response is invalid"))
}

/// Who a relay connection claims to be, from October's rows (never from the phone itself).
#[derive(Clone, Debug)]
pub struct Binding {
    pub bind: String,
    pub pairing: bool,
    pub device_static: [u8; 32],
    pub device_static_raw: String,
    pub device_sign_pub: String,
    pub label: String,
    pub platform: String,
    pub intent_id: Option<String>,
    pub state_version: Option<i64>,
}

pub fn resolve_binding(host_id: &str, bind: &str, access_token: &str) -> Result<Option<Binding>> {
    let cols = "bind,device_static_pub,device_sign_pub,label,platform,revoked_at";
    let rows = rest_get(&format!("mobile_devices?select={cols}&host_id=eq.{host_id}&bind=eq.{bind}&limit=1"), access_token)?;
    if let Some(d) = rows.as_array().and_then(|a| a.first())
        && d["revoked_at"].is_null()
        && let Some(key) = d["device_static_pub"].as_str().and_then(super::store::decode_key)
    {
        return Ok(Some(binding(d, bind, key, false)));
    }
    let now = now_iso();
    let cols = "intent_id,bind,device_static_pub,device_sign_pub,label,platform,state,state_version,expires_at";
    let rows = rest_get(
        &format!(
            "mobile_pair_intents?select={cols}&host_id=eq.{host_id}&bind=eq.{bind}&state=in.(pending,approved)&expires_at=gt.{now}&limit=1"
        ),
        access_token,
    )?;
    let Some(i) = rows.as_array().and_then(|a| a.first()) else { return Ok(None) };
    let Some(key) = i["device_static_pub"].as_str().and_then(super::store::decode_key) else { return Ok(None) };
    let mut b = binding(i, bind, key, true);
    b.intent_id = i["intent_id"].as_str().map(String::from);
    b.state_version = i["state_version"].as_i64();
    Ok(Some(b))
}

fn binding(row: &Value, bind: &str, key: [u8; 32], pairing: bool) -> Binding {
    Binding {
        bind: bind.into(),
        pairing,
        device_static: key,
        device_static_raw: row["device_static_pub"].as_str().unwrap_or("").into(),
        device_sign_pub: row["device_sign_pub"].as_str().unwrap_or("").into(),
        label: row["label"].as_str().unwrap_or("Phone").into(),
        platform: row["platform"].as_str().unwrap_or("ios").into(),
        intent_id: None,
        state_version: None,
    }
}

/// Current UTC time as ISO 8601 (for PostgREST filters and the phone's timestamps).
pub fn now_iso() -> String {
    iso(crate::hooks::now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_message_format_matches_october() {
        let body = br#"{"a":1}"#;
        let m =
            signed_message("ticket-host", body, "11111111-1111-4111-8111-111111111111", "22222222-2222-4222-8222-222222222222", 1790000000);
        let digest = B64.encode(Sha256::digest(body));
        assert_eq!(
            m,
            format!(
                "october-mobile/v1\nticket-host\n11111111-1111-4111-8111-111111111111\n22222222-2222-4222-8222-222222222222\n{digest}\n1790000000"
            )
        );
    }

    #[test]
    fn reads_session_id_and_formats_time() {
        let payload = B64.encode(br#"{"sub":"u","session_id":"s-1"}"#);
        assert_eq!(session_id(&format!("h.{payload}.sig")).as_deref(), Some("s-1"));
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_790_000_000_123), "2026-09-21T14:13:20.123Z");
        assert_eq!(crate::model::epoch_ms("2026-09-21T14:13:20.123Z"), Some(1_790_000_000_123));
        assert_eq!(crate::model::epoch_ms("2026-09-21T14:13:20Z"), Some(1_790_000_000_000));
        assert_eq!(crate::model::epoch_ms("not a time"), None);
    }
}
