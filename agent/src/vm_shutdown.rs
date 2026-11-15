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
use std::{io, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_vsock::{VMADDR_CID_HOST, VsockAddr, VsockListener};

pub const PORT: u32 = 11828;
const CONTEXT: &str = "pryx VM shutdown request v1";
const MAX_BYTES: u64 = 4096;
/// A request that names no grace keeps the original five seconds before the cgroup is killed.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(5);
/// After the workload's grace, init's bound on emptying its cgroup plus its flush and record.
pub const FLUSH_RESERVE: Duration = Duration::from_secs(15);

fn termination() -> i32 {
    libc::SIGTERM
}
fn is_termination(signal: &i32) -> bool {
    *signal == libc::SIGTERM
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub nonce: String,
    pub sandbox: String,
    pub epoch: u64,
    #[serde(default = "termination", skip_serializing_if = "is_termination")]
    pub signal: i32,
    /// How long the workload has after the signal before its cgroup is killed; signed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grace_ms: Option<u64>,
    pub signature: String,
}

/// An authenticated shutdown: the workload's signal and its grace before the kill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accepted {
    pub signal: i32,
    pub grace: Duration,
}

fn bytes(request: &Request) -> Vec<u8> {
    match request.grace_ms {
        Some(grace) => {
            serde_json::to_vec(&(CONTEXT, &request.nonce, &request.sandbox, request.epoch, request.signal, grace))
        }
        // Preserve the original TERM signature for bundles predating signal selection.
        None if request.signal == libc::SIGTERM => {
            serde_json::to_vec(&(CONTEXT, &request.nonce, &request.sandbox, request.epoch))
        }
        None => serde_json::to_vec(&(CONTEXT, &request.nonce, &request.sandbox, request.epoch, request.signal)),
    }
    .expect("fixed shutdown request serializes")
}

pub fn request(key: &SigningKey, nonce: &str, sandbox: &str, epoch: u64) -> Request {
    request_for_signal(key, nonce, sandbox, epoch, libc::SIGTERM).expect("TERM is an admitted shutdown signal")
}

pub fn request_for_signal(
    key: &SigningKey,
    nonce: &str,
    sandbox: &str,
    epoch: u64,
    signal: i32,
) -> io::Result<Request> {
    if !matches!(signal, libc::SIGTERM | libc::SIGINT) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Unsupported VM shutdown signal"));
    }
    let mut request = Request {
        nonce: nonce.into(),
        sandbox: sandbox.into(),
        epoch,
        signal,
        grace_ms: None,
        signature: String::new(),
    };
    request.signature = key.sign_bytes(&bytes(&request));
    Ok(request)
}

/// A workload stop with the spec's own grace before init kills what remains.
pub fn request_with_grace(
    key: &SigningKey,
    nonce: &str,
    sandbox: &str,
    epoch: u64,
    signal: i32,
    grace: Duration,
) -> io::Result<Request> {
    let mut request = request_for_signal(key, nonce, sandbox, epoch, signal)?;
    request.grace_ms = Some(u64::try_from(grace.as_millis()).unwrap_or(u64::MAX));
    request.signature = key.sign_bytes(&bytes(&request));
    Ok(request)
}

impl Request {
    pub fn accepted(&self) -> Accepted {
        Accepted { signal: self.signal, grace: self.grace_ms.map_or(DEFAULT_GRACE, Duration::from_millis) }
    }
}

pub fn verify(request: &Request, public_key: &str, nonce: &str, sandbox: &str, epoch: u64) -> bool {
    nonce_valid(nonce)
        && matches!(request.signal, libc::SIGTERM | libc::SIGINT)
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

    pub(crate) async fn receive(
        &self,
        public_key: &str,
        nonce: &str,
        sandbox: &str,
        epoch: u64,
    ) -> io::Result<Accepted> {
        loop {
            let (mut socket, address) = self.0.accept().await?;
            if address.cid() != VMADDR_CID_HOST {
                continue;
            }
            let received = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                transaction(&mut socket, public_key, nonce, sandbox, epoch),
            )
            .await;
            if let Ok(Ok(Some(accepted))) = received {
                return Ok(accepted);
            }
        }
    }
}

async fn transaction(
    socket: &mut (impl AsyncRead + AsyncWrite + Unpin),
    public_key: &str,
    nonce: &str,
    sandbox: &str,
    epoch: u64,
) -> io::Result<Option<Accepted>> {
    let mut bytes = Vec::new();
    // A bounded JSON line completes the request without half-closing the SDK's
    // transport. The peer can keep reading its acknowledgement on the same stream.
    let mut reader = tokio::io::BufReader::new((&mut *socket).take(MAX_BYTES + 1));
    reader.read_until(b'\n', &mut bytes).await?;
    if bytes.len() as u64 > MAX_BYTES || bytes.last() != Some(&b'\n') {
        return Ok(None);
    }
    let request = serde_json::from_slice::<Request>(&bytes)
        .ok()
        .filter(|request| verify(request, public_key, nonce, sandbox, epoch));
    let Some(request) = request else { return Ok(None) };
    let mut acknowledgement = serde_json::to_vec(&true)?;
    acknowledgement.push(b'\n');
    // A lost acknowledgement never cancels an authenticated shutdown already accepted.
    let _ = socket.write_all(&acknowledgement).await;
    let _ = AsyncWriteExt::shutdown(socket).await;
    Ok(Some(request.accepted()))
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

    #[test]
    fn selected_signal_is_authenticated_and_legacy_term_stays_valid() {
        let (key, _) = SigningKey::generate().unwrap();
        let random = ring::rand::SystemRandom::new();
        let nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(ring::rand::generate::<[u8; 32]>(&random).unwrap().expose());
        let term = request(&key, &nonce, "one", 3);
        let wire = serde_json::to_value(&term).unwrap();
        assert!(wire.get("signal").is_none());
        let legacy: Request = serde_json::from_value(wire).unwrap();
        assert_eq!(legacy.signal, libc::SIGTERM);
        assert!(verify(&legacy, &key.public_key(), &nonce, "one", 3));
        let original_bytes = serde_json::to_vec(&(CONTEXT, &nonce, "one", 3_u64)).unwrap();
        assert!(auth::verify_bytes(&key.public_key(), &original_bytes, &term.signature).is_ok());
        let interrupt = request_for_signal(&key, &nonce, "one", 3, libc::SIGINT).unwrap();
        assert!(verify(&interrupt, &key.public_key(), &nonce, "one", 3));
        let mut tampered = interrupt;
        tampered.signal = libc::SIGTERM;
        assert!(!verify(&tampered, &key.public_key(), &nonce, "one", 3));
        let mut tampered = term;
        tampered.signal = libc::SIGINT;
        assert!(!verify(&tampered, &key.public_key(), &nonce, "one", 3));
        tampered.signal = libc::SIGKILL;
        tampered.signature = key.sign_bytes(&bytes(&tampered));
        assert!(!verify(&tampered, &key.public_key(), &nonce, "one", 3));
        assert!(request_for_signal(&key, &nonce, "one", 3, libc::SIGKILL).is_err());
    }

    #[tokio::test]
    async fn authenticated_request_is_acknowledged_without_peer_eof() {
        let (key, _) = SigningKey::generate().unwrap();
        let random = ring::rand::SystemRandom::new();
        let nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(ring::rand::generate::<[u8; 32]>(&random).unwrap().expose());
        for selected in [libc::SIGTERM, libc::SIGINT] {
            let mut message =
                serde_json::to_vec(&request_for_signal(&key, &nonce, "one", 3, selected).unwrap()).unwrap();
            message.push(b'\n');
            let (mut host, mut guest) = tokio::net::UnixStream::pair().unwrap();
            let host_request = async {
                host.write_all(&message).await.unwrap();
                let mut acknowledgement = Vec::new();
                tokio::io::BufReader::new(&mut host).read_until(b'\n', &mut acknowledgement).await.unwrap();
                assert!(serde_json::from_slice::<bool>(&acknowledgement).unwrap());
            };
            let public_key = key.public_key();
            let accepted = transaction(&mut guest, &public_key, &nonce, "one", 3);
            let (_, accepted) =
                tokio::time::timeout(std::time::Duration::from_secs(1), async { tokio::join!(host_request, accepted) })
                    .await
                    .expect("A complete JSON request must not wait for peer EOF");
            assert_eq!(accepted.unwrap(), Some(Accepted { signal: selected, grace: DEFAULT_GRACE }));
        }
    }

    #[tokio::test]
    async fn shutdown_listener_refuses_the_host_before_binding() {
        assert!(Listener::bind(VMADDR_CID_HOST + 1).is_err());
    }

    #[tokio::test]
    async fn the_signed_grace_reaches_init_and_cannot_be_changed() {
        let (key, _) = SigningKey::generate().unwrap();
        let random = ring::rand::SystemRandom::new();
        let nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(ring::rand::generate::<[u8; 32]>(&random).unwrap().expose());
        let grace = Duration::from_secs(90);
        let request = request_with_grace(&key, &nonce, "one", 3, libc::SIGINT, grace).unwrap();
        assert!(verify(&request, &key.public_key(), &nonce, "one", 3));
        assert_eq!(request.accepted(), Accepted { signal: libc::SIGINT, grace });
        let mut shortened = request.clone();
        shortened.grace_ms = Some(0);
        assert!(!verify(&shortened, &key.public_key(), &nonce, "one", 3));
        let mut dropped = request.clone();
        dropped.grace_ms = None;
        assert!(!verify(&dropped, &key.public_key(), &nonce, "one", 3));
        let mut message = serde_json::to_vec(&request).unwrap();
        message.push(b'\n');
        let (mut host, mut guest) = tokio::net::UnixStream::pair().unwrap();
        host.write_all(&message).await.unwrap();
        let accepted = transaction(&mut guest, &key.public_key(), &nonce, "one", 3).await.unwrap();
        assert_eq!(accepted, Some(Accepted { signal: libc::SIGINT, grace }));
    }
}
