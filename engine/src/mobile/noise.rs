//! The end-to-end encrypted channel with the phone: Noise_XX_25519_ChaChaPoly_BLAKE2b, the same
//! as October Desktop's `src/shared/remoteChannel.ts` (holepunch noise-handshake). Lantern is
//! always the responder; the phone's static key is pinned to the one on October's servers.

use anyhow::{Result, bail};
use snow::{HandshakeState, TransportState};

pub const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2b";
const PROLOGUE_PREFIX: &[u8] = b"october-remote/1";

pub fn uuid_bytes(value: &str) -> Result<[u8; 16]> {
    let u = uuid::Uuid::parse_str(value)?;
    if u.hyphenated().to_string() != value {
        bail!("not a canonical lowercase UUID: {value}");
    }
    Ok(*u.as_bytes())
}

/// "october-remote/1" | hostId (16) | bind (16) | connectionId (u64 BE).
pub fn prologue(host_id: &str, bind: &str, connection_id: u64) -> Result<Vec<u8>> {
    let mut p = PROLOGUE_PREFIX.to_vec();
    p.extend_from_slice(&uuid_bytes(host_id)?);
    p.extend_from_slice(&uuid_bytes(bind)?);
    p.extend_from_slice(&connection_id.to_be_bytes());
    Ok(p)
}

/// The 6-digit code both screens show during pairing: first 4 hash bytes (BE) mod 1,000,000.
pub fn pairing_code(handshake_hash: &[u8]) -> String {
    let n = u32::from_be_bytes([handshake_hash[0], handshake_hash[1], handshake_hash[2], handshake_hash[3]]);
    format!("{:06}", n % 1_000_000)
}

/// A new X25519 static key pair: (secret, public).
pub fn generate_static() -> Result<([u8; 32], [u8; 32])> {
    let kp = snow::Builder::new(PATTERN.parse()?).generate_keypair()?;
    let mut secret = [0u8; 32];
    let mut public = [0u8; 32];
    secret.copy_from_slice(&kp.private);
    public.copy_from_slice(&kp.public);
    Ok((secret, public))
}

pub struct Responder {
    state: HandshakeState,
}

#[allow(clippy::large_enum_variant)] // One per handshake message; the responder state is the big one.
pub enum Step {
    /// Send this handshake message and wait for the next one.
    Reply(Responder, Vec<u8>),
    /// Handshake done: the channel, the phone's static key and the handshake hash.
    Done(Channel, Vec<u8>, Vec<u8>),
}

impl Responder {
    pub fn new(static_secret: &[u8; 32], prologue: &[u8]) -> Result<Self> {
        Self::build(static_secret, prologue, None)
    }

    pub(crate) fn build(static_secret: &[u8; 32], prologue: &[u8], ephemeral: Option<&[u8; 32]>) -> Result<Self> {
        let mut b = snow::Builder::new(PATTERN.parse()?).local_private_key(static_secret)?.prologue(prologue)?;
        if let Some(e) = ephemeral {
            b = b.fixed_ephemeral_key_for_testing_only(e);
        }
        Ok(Responder { state: b.build_responder()? })
    }

    /// Feeds one handshake message from the phone (XX: message 1, then message 3).
    pub fn read(mut self, message: &[u8]) -> Result<Step> {
        let mut payload = vec![0u8; 65535];
        self.state.read_message(message, &mut payload).map_err(|e| anyhow::anyhow!("noise handshake failed: {e}"))?;
        if self.state.is_handshake_finished() {
            let remote = self.state.get_remote_static().map(|k| k.to_vec()).unwrap_or_default();
            let hash = self.state.get_handshake_hash().to_vec();
            let transport = self.state.into_transport_mode()?;
            return Ok(Step::Done(Channel { transport }, remote, hash));
        }
        let mut out = vec![0u8; 65535];
        let n = self.state.write_message(&[], &mut out)?;
        out.truncate(n);
        Ok(Step::Reply(self, out))
    }
}

/// Accepts only the expected, non-zero 32-byte key (constant-time comparison).
pub fn pinned(remote: &[u8], expected: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    remote.len() == 32 && expected.len() == 32 && remote.iter().any(|b| *b != 0) && bool::from(remote.ct_eq(expected))
}

pub struct Channel {
    transport: TransportState,
}

impl Channel {
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut out = vec![0u8; plaintext.len() + 16];
        let n = self.transport.write_message(plaintext, &mut out)?;
        out.truncate(n);
        Ok(out)
    }

    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let mut out = vec![0u8; ciphertext.len()];
        let n = self.transport.read_message(ciphertext, &mut out).map_err(|e| anyhow::anyhow!("decrypt failed: {e}"))?;
        out.truncate(n);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blake2::Digest;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// libsodium crypto_kx_seed_keypair: secret = BLAKE2b-256(seed).
    fn seed_secret(seed: &str) -> [u8; 32] {
        let out = blake2::Blake2b::<blake2::digest::consts::U32>::digest(hex(seed));
        let mut s = [0u8; 32];
        s.copy_from_slice(&out);
        s
    }

    /// October's own vectors (october-desktop src/shared/remoteChannel.vectors.ts).
    #[test]
    fn matches_october_vectors() {
        let host = "01234567-89ab-4def-8123-456789abcdef";
        let bind = "fedcba98-7654-4321-9abc-def012345678";
        let p = prologue(host, bind, 0x0102_0304_0506_0708).unwrap();
        let init_s = seed_secret("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20");
        let resp_s = seed_secret("4142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f60");
        let init_e = seed_secret("8182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0");
        let resp_e = seed_secret("c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0");

        let mut initiator = snow::Builder::new(PATTERN.parse().unwrap())
            .local_private_key(&init_s)
            .unwrap()
            .prologue(&p)
            .unwrap()
            .fixed_ephemeral_key_for_testing_only(&init_e)
            .build_initiator()
            .unwrap();
        let mut buf = vec![0u8; 1024];
        let n = initiator.write_message(&[], &mut buf).unwrap();
        let m1 = buf[..n].to_vec();
        assert_eq!(m1, hex("cc27c49ae6a3bc6b2ccba388318fbc79ab8e668f6d0ea1dcc76dd4027c90c72b"));

        let r = Responder::build(&resp_s, &p, Some(&resp_e)).unwrap();
        let Step::Reply(r, m2) = r.read(&m1).unwrap() else { panic!("expected a reply") };
        assert_eq!(
            m2,
            hex(
                "0a8665a960bb70b15a5ccb90411c80f563553f56721ff5d47c8cf79762dd605c130dd5a6734e665b473fdfc79e1f1eafd20e7f19475a8e8493b2ad0aeb0b3edcfbb3f477714150bd432c9e4de16287dee1a57e3d8ad70bf468615e711cc8e2d0"
            )
        );

        let mut payload = vec![0u8; 1024];
        initiator.read_message(&m2, &mut payload).unwrap();
        let n = initiator.write_message(&[], &mut buf).unwrap();
        let m3 = buf[..n].to_vec();
        assert_eq!(
            m3,
            hex(
                "70c58eb1e7608486a37e7242a5caa666e0629c4ed242b5e90465f480df357f91d086797606dafafcd2d0632e908902a4912addc3d7993ceb4488249bc469c539"
            )
        );

        let Step::Done(mut channel, remote, hash) = r.read(&m3).unwrap() else { panic!("expected done") };
        assert_eq!(
            hash,
            hex(
                "12f17393018735d1d35a2ceb1b453521d259313ba87fa329445cb86add148b4be8131a78d970da5c98b9cdb59d1f4a4f079fbc3539859a01e425d9dddc2fdce5"
            )
        );
        assert_eq!(remote.len(), 32);

        let mut itx = initiator.into_transport_mode().unwrap();
        let n = itx.write_message(&hex("6f63746f626572"), &mut buf).unwrap();
        assert_eq!(buf[..n].to_vec(), hex("b1b5c66508e5e0ef3a205afc462e5b168b296bd225fe5e"));
        assert_eq!(channel.decrypt(&buf[..n]).unwrap(), b"october");
        assert_eq!(pairing_code(&hash).len(), 6);
    }
}
