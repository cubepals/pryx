// SPDX-License-Identifier: FSL-1.1-ALv2
//! Authenticated completion for a node exec's private stdin/stderr transport. A fresh key stays
//! in the trusted owner, never in its children. A signal exit or user-written footer is no proof
//! of cleanup; the node must retire the sandbox when this completion cannot be authenticated.
//! Sandbox lifecycle and engine I/O belong to the node, outside this private wire contract.
use base64::Engine;
use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};
use std::io::{Read, Write};

pub const VERSION: u16 = 1;
pub const KEY_BYTES: usize = 32;
/// More than the complete fixed-shape footer, retained separately from bounded user output.
pub const TAIL_BYTES: usize = 192;
pub const READY: &[u8] = b"\x1epryx-exec-ready-v1\n";
const PREFIX: &[u8] = b"\n\x1epryx-exec-v1 ";

pub struct Key(hmac::Key);
pub struct Completion {
    pub code: i32,
    pub timed_out: bool,
}
impl Key {
    pub fn new(bytes: &[u8]) -> Self {
        Self(hmac::Key::new(hmac::HMAC_SHA256, bytes))
    }
    pub fn fresh() -> Result<(Self, [u8; KEY_BYTES]), ring::error::Unspecified> {
        let mut bytes = [0; KEY_BYTES];
        SystemRandom::new().fill(&mut bytes)?;
        Ok((Self::new(&bytes), bytes))
    }
    pub fn read(input: &mut impl Read) -> std::io::Result<Self> {
        let mut bytes = [0; KEY_BYTES];
        input.read_exact(&mut bytes)?;
        let key = Self::new(&bytes);
        bytes.fill(0);
        Ok(key)
    }
    pub fn write(&self, output: &mut impl Write, code: i32, timed_out: bool) -> std::io::Result<()> {
        let payload = format!("{code} {}", u8::from(timed_out));
        let signature = hmac::sign(&self.0, payload.as_bytes());
        output.write_all(PREFIX)?;
        writeln!(output, "{payload} {}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.as_ref()))?;
        output.flush()
    }
    /// Authenticates the final footer and removes only that footer from the retained tail.
    /// No secret, untrusted payload or signature is returned on rejection.
    pub fn verify(&self, tail: &mut Vec<u8>, exit: Option<i64>) -> Option<Completion> {
        let start = tail.windows(PREFIX.len()).rposition(|part| part == PREFIX)?;
        let body = std::str::from_utf8(tail.get(start + PREFIX.len()..)?.strip_suffix(b"\n")?).ok()?;
        let mut fields = body.split(' ');
        let code: i32 = fields.next()?.parse().ok()?;
        let timed_out = match fields.next()? {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(fields.next()?).ok()?;
        if fields.next().is_some() || exit != Some(i64::from(code)) {
            return None;
        }
        let payload = format!("{code} {}", u8::from(timed_out));
        hmac::verify(&self.0, payload.as_bytes(), &signature).ok()?;
        tail.truncate(start);
        Some(Completion { code, timed_out })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_private_key_authenticates_cleanup_and_its_exact_exit() {
        let (key, _) = Key::fresh().unwrap();
        let mut bytes = b"user stderr".to_vec();
        key.write(&mut bytes, 23, false).unwrap();
        let mut wrong = bytes.clone();
        assert!(Key::fresh().unwrap().0.verify(&mut wrong, Some(23)).is_none());
        assert!(key.verify(&mut bytes.clone(), Some(24)).is_none());
        let completed = key.verify(&mut bytes, Some(23)).unwrap();
        assert_eq!(completed.code, 23);
        assert!(!completed.timed_out);
        assert_eq!(bytes, b"user stderr");
        let mut forged = Vec::new();
        Key::fresh().unwrap().0.write(&mut forged, 128 + libc::SIGKILL, true).unwrap();
        assert!(key.verify(&mut forged, Some(i64::from(128 + libc::SIGKILL))).is_none());
    }
}
