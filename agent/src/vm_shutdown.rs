// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Boot-scoped shutdown requests reach trusted PID 1 over a host-CID-only vsock listener.
//! A published kernel need not have a keyboard driver. Workload HTTP tokens, serial output
//! and another boot's request cannot authorize a disk flush or certify its completion.
use crate::{
    auth::{self, SigningKey},
    process_observation::nonce_valid,
};
use serde::{Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_vsock::{VMADDR_CID_HOST, VsockAddr, VsockListener};

pub const PORT: u32 = 11828;
const CONTEXT: &str = "pryx VM shutdown request v1";
const MAX_BYTES: u64 = 4096;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub nonce: String,
    pub sandbox: String,
    pub epoch: u64,
    pub signature: String,
}

fn bytes(request: &Request) -> Vec<u8> {
    serde_json::to_vec(&(CONTEXT, &request.nonce, &request.sandbox, request.epoch))
        .expect("fixed shutdown request serializes")
}

pub fn request(key: &SigningKey, nonce: &str, sandbox: &str, epoch: u64) -> Request {
    let mut request = Request { nonce: nonce.into(), sandbox: sandbox.into(), epoch, signature: String::new() };
    request.signature = key.sign_bytes(&bytes(&request));
    request
}

pub fn verify(request: &Request, public_key: &str, nonce: &str, sandbox: &str, epoch: u64) -> bool {
    nonce_valid(nonce)
        && request.nonce == nonce
        && request.sandbox == sandbox
        && request.epoch == epoch
        && auth::verify_bytes(public_key, &bytes(request), &request.signature).is_ok()
}

pub(crate) struct Listener(VsockListener);

impl Listener {
    pub(crate) fn bind(cid: u32) -> io::Result<Self> {
        if rustix::process::getpid() != rustix::process::Pid::INIT || !crate::vm_boot::guest_cid(cid) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "VM shutdown listener requires trusted init"));
        }
        VsockListener::bind(VsockAddr::new(cid, PORT)).map(Self)
    }

    pub(crate) async fn receive(&self, public_key: &str, nonce: &str, sandbox: &str, epoch: u64) -> io::Result<()> {
        loop {
            let (mut socket, address) = self.0.accept().await?;
            if address.cid() != VMADDR_CID_HOST {
                continue;
            }
            let received = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                let mut bytes = Vec::new();
                (&mut socket).take(MAX_BYTES + 1).read_to_end(&mut bytes).await?;
                if bytes.len() as u64 > MAX_BYTES {
                    return Ok(false);
                }
                let valid = serde_json::from_slice::<Request>(&bytes)
                    .is_ok_and(|request| verify(&request, public_key, nonce, sandbox, epoch));
                Ok::<_, io::Error>(valid)
            })
            .await;
            if !matches!(received, Ok(Ok(true))) {
                continue;
            }
            // A lost acknowledgement never cancels an authenticated shutdown already accepted.
            let _ = socket.write_all(&serde_json::to_vec(&true)?).await;
            let _ = AsyncWriteExt::shutdown(&mut socket).await;
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn shutdown_authority_cannot_be_forged_replayed_or_reassigned() {
        let random = ring::rand::SystemRandom::new();
        let nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(ring::rand::generate::<[u8; 32]>(&random).unwrap().expose());
        let next = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(ring::rand::generate::<[u8; 32]>(&random).unwrap().expose());
        assert_ne!(nonce, next);
        let (key, _) = SigningKey::generate().unwrap();
        let message = request(&key, &nonce, "one", 3);
        assert!(verify(&message, &key.public_key(), &nonce, "one", 3));
        assert!(!verify(&message, &key.public_key(), &next, "one", 3));
        assert!(!verify(&message, &key.public_key(), &nonce, "other", 3));
        assert!(!verify(&message, &key.public_key(), &nonce, "one", 4));
        let (other, _) = SigningKey::generate().unwrap();
        assert!(!verify(&message, &other.public_key(), &nonce, "one", 3));
        let mut tampered = message.clone();
        tampered.nonce = next.clone();
        assert!(!verify(&tampered, &key.public_key(), &next, "one", 3));
        tampered = message;
        tampered.signature.clear();
        assert!(!verify(&tampered, &key.public_key(), &nonce, "one", 3));
    }

    #[tokio::test]
    async fn shutdown_listener_refuses_the_host_before_binding() {
        assert!(Listener::bind(VMADDR_CID_HOST + 1).is_err());
    }
}
