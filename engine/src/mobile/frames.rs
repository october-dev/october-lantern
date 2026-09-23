//! Framing, matching October Desktop's `remoteFrames.ts` and `remote-relay-client.ts`.
//!
//! - Relay (outer, unencrypted) frames on the host socket: `type(1) | bind(16) | connectionId(u64 BE) | payload`,
//!   each acknowledged with `04 | connectionId(u64 BE) | byteCount(u32 BE)`.
//! - Inner frames (inside Noise): `ver=1 | kind | flags(u16)=0 | messageId(u32) | seq(u16) | totalChunks(u16) | totalBytes(u32)`,
//!   big-endian, at most 65,503 data bytes per chunk, one Noise message per chunk.

use anyhow::{Result, bail};

pub const OUTER_OPEN: u8 = 0x01;
pub const OUTER_CLOSE: u8 = 0x02;
pub const OUTER_DATA: u8 = 0x03;
pub const RELAY_ACK: u8 = 0x04;

pub const CLOSE_ENDED: u16 = 4400;
pub const CLOSE_AUTH_FAILED: u16 = 4401;
pub const CLOSE_REVOKED: u16 = 4403;

pub struct Outer {
    pub kind: u8,
    pub bind: String,
    pub connection_id: u64,
    pub payload: Vec<u8>,
}

pub fn decode_outer(bytes: &[u8]) -> Result<Outer> {
    if bytes.len() < 25 {
        bail!("relay frame too short");
    }
    let mut b = [0u8; 16];
    b.copy_from_slice(&bytes[1..17]);
    let mut c = [0u8; 8];
    c.copy_from_slice(&bytes[17..25]);
    Ok(Outer {
        kind: bytes[0],
        bind: uuid::Uuid::from_bytes(b).hyphenated().to_string(),
        connection_id: u64::from_be_bytes(c),
        payload: bytes[25..].to_vec(),
    })
}

pub fn encode_outer(kind: u8, bind: &str, connection_id: u64, payload: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(25 + payload.len());
    out.push(kind);
    out.extend_from_slice(&super::noise::uuid_bytes(bind)?);
    out.extend_from_slice(&connection_id.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

pub fn host_ack(connection_id: u64, received_bytes: usize) -> Vec<u8> {
    let mut out = vec![RELAY_ACK];
    out.extend_from_slice(&connection_id.to_be_bytes());
    out.extend_from_slice(&(received_bytes as u32).to_be_bytes());
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Auth = 1,
    Req = 2,
    Res = 3,
    Sub = 4,
    Unsub = 5,
    Ev = 6,
    PairOffer = 7,
    PairCredential = 8,
    PairAck = 9,
    PairActive = 10,
    Ping = 11,
    Pong = 12,
    Cancel = 13,
}

impl Kind {
    fn from(v: u8) -> Option<Kind> {
        use Kind::*;
        Some(match v {
            1 => Auth,
            2 => Req,
            3 => Res,
            4 => Sub,
            5 => Unsub,
            6 => Ev,
            7 => PairOffer,
            8 => PairCredential,
            9 => PairAck,
            10 => PairActive,
            11 => Ping,
            12 => Pong,
            13 => Cancel,
            _ => return None,
        })
    }
}

pub const MAX_CHUNK: usize = 65_503;
const MAX_BUFFERED: usize = 8 * 1024 * 1024;
const MAX_IN_FLIGHT: usize = 16;

/// One message as one or more plaintext chunks (each to be encrypted separately).
pub fn encode_message(kind: Kind, data: &[u8], message_id: u32) -> Vec<Vec<u8>> {
    let total_chunks = data.len().div_ceil(MAX_CHUNK).max(1);
    (0..total_chunks)
        .map(|seq| {
            let chunk = &data[(seq * MAX_CHUNK).min(data.len())..((seq + 1) * MAX_CHUNK).min(data.len())];
            let mut f = Vec::with_capacity(16 + chunk.len());
            f.push(1);
            f.push(kind as u8);
            f.extend_from_slice(&0u16.to_be_bytes());
            f.extend_from_slice(&message_id.to_be_bytes());
            f.extend_from_slice(&(seq as u16).to_be_bytes());
            f.extend_from_slice(&(total_chunks as u16).to_be_bytes());
            f.extend_from_slice(&(data.len() as u32).to_be_bytes());
            f.extend_from_slice(chunk);
            f
        })
        .collect()
}

/// `res` body: status(u16 BE) | serverTimeMs(u64 BE) | JSON.
pub fn encode_response(status: u16, server_time_ms: u64, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(10 + body.len());
    out.extend_from_slice(&status.to_be_bytes());
    out.extend_from_slice(&server_time_ms.to_be_bytes());
    out.extend_from_slice(body);
    out
}

struct Partial {
    kind: Kind,
    total_chunks: u16,
    total_bytes: usize,
    next: u16,
    data: Vec<u8>,
}

/// Reassembles chunked messages. Returns (kind, messageId, data) when a message is complete.
#[derive(Default)]
pub struct Assembler {
    partial: std::collections::HashMap<u32, Partial>,
}

impl Assembler {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Option<(Kind, u32, Vec<u8>)>> {
        if chunk.len() < 16 || chunk[0] != 1 || chunk[2] != 0 || chunk[3] != 0 {
            bail!("invalid frame header");
        }
        let kind = Kind::from(chunk[1]).ok_or_else(|| anyhow::anyhow!("unknown frame kind"))?;
        let message_id = u32::from_be_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        let seq = u16::from_be_bytes([chunk[8], chunk[9]]);
        let total_chunks = u16::from_be_bytes([chunk[10], chunk[11]]);
        let total_bytes = u32::from_be_bytes([chunk[12], chunk[13], chunk[14], chunk[15]]) as usize;
        let data = &chunk[16..];
        if total_chunks == 0 || data.len() > MAX_CHUNK || total_bytes > MAX_BUFFERED {
            bail!("invalid frame size");
        }
        if total_chunks == 1 {
            if seq != 0 || data.len() != total_bytes {
                bail!("invalid single frame");
            }
            return Ok(Some((kind, message_id, data.to_vec())));
        }
        if seq == 0 {
            if self.partial.len() >= MAX_IN_FLIGHT || self.partial.contains_key(&message_id) {
                bail!("too many messages in flight");
            }
            self.partial.insert(message_id, Partial { kind, total_chunks, total_bytes, next: 0, data: Vec::new() });
        }
        let p = self.partial.get_mut(&message_id).ok_or_else(|| anyhow::anyhow!("chunk without a start"))?;
        if p.kind != kind || p.total_chunks != total_chunks || p.total_bytes != total_bytes || p.next != seq {
            bail!("chunk out of order");
        }
        p.data.extend_from_slice(data);
        p.next += 1;
        if p.data.len() > p.total_bytes {
            bail!("message larger than announced");
        }
        if p.next == p.total_chunks {
            let p = self.partial.remove(&message_id).unwrap();
            if p.data.len() != p.total_bytes {
                bail!("message size mismatch");
            }
            return Ok(Some((p.kind, message_id, p.data)));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inner_frames_round_trip_and_chunk() {
        let big = vec![7u8; MAX_CHUNK * 2 + 10];
        let chunks = encode_message(Kind::Res, &big, 42);
        assert_eq!(chunks.len(), 3);
        assert_eq!(&chunks[0][..16], &[1, 3, 0, 0, 0, 0, 0, 42, 0, 0, 0, 3, 0, 1, 0xff, 0xc8]);
        let mut a = Assembler::default();
        assert!(a.push(&chunks[0]).unwrap().is_none());
        assert!(a.push(&chunks[1]).unwrap().is_none());
        let (k, id, data) = a.push(&chunks[2]).unwrap().unwrap();
        assert_eq!((k, id, data.len()), (Kind::Res, 42, big.len()));
        let ping = encode_message(Kind::Ping, &[], 9);
        assert_eq!(ping[0], vec![1, 11, 0, 0, 0, 0, 0, 9, 0, 0, 0, 1, 0, 0, 0, 0]);
        assert_eq!(a.push(&ping[0]).unwrap().unwrap().0, Kind::Ping);
        assert!(a.push(&[2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn outer_frames_and_acks() {
        let bind = "fedcba98-7654-4321-9abc-def012345678";
        let f = encode_outer(OUTER_DATA, bind, 0x0102030405060708, b"hi").unwrap();
        assert_eq!(f.len(), 27);
        let o = decode_outer(&f).unwrap();
        assert_eq!((o.kind, o.bind.as_str(), o.connection_id, o.payload.as_slice()), (OUTER_DATA, bind, 0x0102030405060708, &b"hi"[..]));
        assert_eq!(host_ack(5, 27), vec![4, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 27]);
        assert_eq!(&encode_response(200, 1, b"{}")[..10], &[0, 200, 0, 0, 0, 0, 0, 0, 0, 1]);
    }
}
