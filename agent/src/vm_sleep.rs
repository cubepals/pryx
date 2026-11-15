// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Memory sleep's guest half: a boot-scoped signed request asks trusted init to flush its disk
//! and hold a connection to the host; the monitor's vsock reset on restore ends that connection,
//! and init then asks the host for a signed resume: the time to step its clock to and host
//! entropy for its pool. A workload token, the serial log or another boot cannot ask for either.
//! The host's pause, snapshot and restore are not here (`src/runtime/vm/instance/memory.rs`).
use crate::{
    auth::{self, SigningKey},
    process_observation::nonce_valid,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::{io, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio_vsock::{VMADDR_CID_HOST, VsockAddr, VsockListener, VsockStream};

/// The sleep and resume exchange this init answers, as its bootstrap capabilities announce.
pub const VERSION: u16 = 1;
/// Trusted init's sleep listener, beside the shutdown listener's port.
pub const PORT: u32 = 11831;
/// The host listener init dials once its disk is flushed; the host holds it while it snapshots.
pub const HOLD_PORT: u32 = 11832;
/// The host listener a restored guest dials for its signed resume.
pub const RESUME_PORT: u32 = 11833;
const REQUEST_CONTEXT: &str = "pryx VM sleep request v1";
const RESUME_CONTEXT: &str = "pryx VM resume v1";
const MAX_BYTES: u64 = 4096;
/// Host entropy a resume carries, written into the guest's input pool.
pub const SEED_BYTES: usize = 64;
/// What init writes to the serial console once its resume hook has run.
pub const RESUMED_MARKER: &str = "PRYX_VM_RESUMED";
/// How long a woken init waits for the host's resume before going on without one.
const RESUME_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub nonce: String,
    pub sandbox: String,
    pub epoch: u64,
    pub signature: String,
}

/// What a restored guest is told: the host's wall clock when it answered, and fresh entropy.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resume {
    pub nonce: String,
    pub sandbox: String,
    pub epoch: u64,
    /// Nanoseconds since the Unix epoch, as a decimal string so no JSON number rounds it.
    pub now_nanos: String,
    pub seed: String,
    pub signature: String,
}

fn request_bytes(nonce: &str, sandbox: &str, epoch: u64) -> Vec<u8> {
    serde_json::to_vec(&(REQUEST_CONTEXT, nonce, sandbox, epoch)).expect("fixed sleep request serializes")
}

fn resume_bytes(resume: &Resume) -> Vec<u8> {
    serde_json::to_vec(&(RESUME_CONTEXT, &resume.nonce, &resume.sandbox, resume.epoch, &resume.now_nanos, &resume.seed))
        .expect("fixed resume serializes")
}

pub fn request(key: &SigningKey, nonce: &str, sandbox: &str, epoch: u64) -> Request {
    Request {
        nonce: nonce.into(),
        sandbox: sandbox.into(),
        epoch,
        signature: key.sign_bytes(&request_bytes(nonce, sandbox, epoch)),
    }
}

pub fn verify(request: &Request, public_key: &str, nonce: &str, sandbox: &str, epoch: u64) -> bool {
    nonce_valid(nonce)
        && request.nonce == nonce
        && request.sandbox == sandbox
        && request.epoch == epoch
        && auth::verify_bytes(public_key, &request_bytes(nonce, sandbox, epoch), &request.signature).is_ok()
}

/// A resume for this boot, signed, at `now_nanos` with `seed`'s bytes.
pub fn resume(key: &SigningKey, nonce: &str, sandbox: &str, epoch: u64, now_nanos: i128, seed: &[u8]) -> Resume {
    let mut resume = Resume {
        nonce: nonce.into(),
        sandbox: sandbox.into(),
        epoch,
        now_nanos: now_nanos.to_string(),
        seed: URL_SAFE_NO_PAD.encode(seed),
        signature: String::new(),
    };
    resume.signature = key.sign_bytes(&resume_bytes(&resume));
    resume
}

pub fn verify_resume(resume: &Resume, public_key: &str, nonce: &str, sandbox: &str, epoch: u64) -> bool {
    nonce_valid(nonce)
        && resume.nonce == nonce
        && resume.sandbox == sandbox
        && resume.epoch == epoch
        && resume.now_nanos.parse::<i128>().is_ok_and(|nanos| nanos > 0)
        && URL_SAFE_NO_PAD.decode(&resume.seed).is_ok_and(|seed| seed.len() == SEED_BYTES)
        && auth::verify_bytes(public_key, &resume_bytes(resume), &resume.signature).is_ok()
}

/// The boot this init serves, which every request and resume must name.
pub(crate) struct Boot<'a> {
    pub(crate) public_key: &'a str,
    pub(crate) nonce: &'a str,
    pub(crate) sandbox: &'a str,
    pub(crate) epoch: u64,
}

/// An accepted, verified sleep request whose acknowledgement init sends once its disk is flushed.
pub(crate) struct Pending(VsockStream);

impl Pending {
    pub(crate) async fn acknowledge(mut self) {
        // A lost acknowledgement leaves the host to give up; the held connection still ends.
        let _ = self.0.write_all(b"true\n").await;
        let _ = AsyncWriteExt::shutdown(&mut self.0).await;
    }
}

/// Init's held connection to the host; its end, at restore or when the host gives up, wakes init.
pub(crate) struct Held(VsockStream);

impl Held {
    /// Dials the host's hold listener. Its peer is the host, as every vsock peer of a guest is.
    pub(crate) async fn dial() -> io::Result<Self> {
        let stream =
            tokio::time::timeout(RESUME_WAIT, VsockStream::connect(VsockAddr::new(VMADDR_CID_HOST, HOLD_PORT)))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "VM sleep hold was not accepted"))??;
        Ok(Self(stream))
    }
    /// Returns when the connection ends, whichever way: the host never sends on it.
    pub(crate) async fn ended(&mut self) {
        let mut byte = [0u8; 1];
        loop {
            match self.0.read(&mut byte).await {
                Ok(0) | Err(_) => return,
                Ok(_) => (),
            }
        }
    }
}

pub(crate) struct Listener(VsockListener);

impl Listener {
    pub(crate) fn bind(cid: u32) -> io::Result<Self> {
        if rustix::process::getpid() != rustix::process::Pid::INIT || !crate::vm_boot::guest_cid(cid) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "VM sleep listener requires trusted init"));
        }
        VsockListener::bind(VsockAddr::new(cid, PORT)).map(Self)
    }

    /// The next verified request from the host; anything else is dropped unanswered.
    pub(crate) async fn receive(&self, boot: &Boot<'_>) -> io::Result<Pending> {
        loop {
            let (mut socket, address) = self.0.accept().await?;
            if address.cid() != VMADDR_CID_HOST {
                continue;
            }
            let received = tokio::time::timeout(Duration::from_secs(3), read_line(&mut socket)).await;
            if let Ok(Ok(Some(bytes))) = received
                && serde_json::from_slice::<Request>(&bytes)
                    .is_ok_and(|request| verify(&request, boot.public_key, boot.nonce, boot.sandbox, boot.epoch))
            {
                return Ok(Pending(socket));
            }
        }
    }
}

async fn read_line(socket: &mut (impl AsyncRead + Unpin)) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let mut reader = tokio::io::BufReader::new(socket.take(MAX_BYTES + 1));
    reader.read_until(b'\n', &mut bytes).await?;
    Ok((bytes.len() as u64 <= MAX_BYTES && bytes.last() == Some(&b'\n')).then_some(bytes))
}

/// A verified resume, whose acknowledgement init sends once it has applied it.
pub(crate) struct Resumed {
    pub(crate) resume: Resume,
    socket: VsockStream,
}

impl Resumed {
    pub(crate) async fn acknowledge(mut self) {
        let _ = self.socket.write_all(b"true\n").await;
        let _ = AsyncWriteExt::shutdown(&mut self.socket).await;
    }
}

/// After the held connection ended: the host's signed resume, or none when the host has none to
/// give (it gave up before snapshotting, and the guest never left this host's memory).
pub(crate) async fn resumed(boot: &Boot<'_>) -> Option<Resumed> {
    let exchange = async {
        let mut socket = VsockStream::connect(VsockAddr::new(VMADDR_CID_HOST, RESUME_PORT)).await?;
        let resume = read_line(&mut socket)
            .await?
            .and_then(|bytes| serde_json::from_slice::<Resume>(&bytes).ok())
            .filter(|resume| verify_resume(resume, boot.public_key, boot.nonce, boot.sandbox, boot.epoch));
        Ok::<_, io::Error>(resume.map(|resume| Resumed { resume, socket }))
    };
    tokio::time::timeout(RESUME_WAIT, exchange).await.ok().and_then(Result::ok).flatten()
}

/// Steps the wall clock to the host's and mixes the host's entropy into the guest's pool. The
/// monotonic clock goes on from the snapshot, as it does across any suspend.
pub(crate) fn apply(resume: &Resume) -> io::Result<()> {
    let nanos: i128 = resume.now_nanos.parse().map_err(|_| io::Error::other("Invalid resume time"))?;
    const NANOS: i128 = 1_000_000_000;
    let now = rustix::time::Timespec {
        tv_sec: i64::try_from(nanos.div_euclid(NANOS)).map_err(|_| io::Error::other("Invalid resume time"))?,
        tv_nsec: i64::try_from(nanos.rem_euclid(NANOS)).map_err(|_| io::Error::other("Invalid resume time"))?,
    };
    rustix::time::clock_settime(rustix::time::ClockId::Realtime, now)?;
    let seed = URL_SAFE_NO_PAD.decode(&resume.seed).map_err(|_| io::Error::other("Invalid resume seed"))?;
    // random(4): writing to /dev/urandom mixes the bytes into the input pool.
    std::fs::write("/dev/urandom", seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nonce() -> String {
        let random = ring::rand::SystemRandom::new();
        URL_SAFE_NO_PAD.encode(ring::rand::generate::<[u8; 32]>(&random).unwrap().expose())
    }

    #[test]
    fn a_sleep_request_names_its_boot_and_cannot_be_reused() {
        let (key, _) = SigningKey::generate().unwrap();
        let (nonce, next) = (nonce(), nonce());
        let message = request(&key, &nonce, "one", 3);
        assert!(verify(&message, &key.public_key(), &nonce, "one", 3));
        assert!(!verify(&message, &key.public_key(), &next, "one", 3));
        assert!(!verify(&message, &key.public_key(), &nonce, "other", 3));
        assert!(!verify(&message, &key.public_key(), &nonce, "one", 4));
        let (other, _) = SigningKey::generate().unwrap();
        assert!(!verify(&message, &other.public_key(), &nonce, "one", 3));
        // A shutdown request for the same boot is not a sleep request.
        let shutdown = crate::vm_shutdown::request(&key, &nonce, "one", 3);
        let forged = Request { signature: shutdown.signature, ..message };
        assert!(!verify(&forged, &key.public_key(), &nonce, "one", 3));
    }

    #[test]
    fn a_resume_is_signed_whole() {
        let (key, _) = SigningKey::generate().unwrap();
        let nonce = nonce();
        let seed = [7u8; SEED_BYTES];
        let message = resume(&key, &nonce, "one", 3, 1_800_000_000_123_456_789, &seed);
        assert!(verify_resume(&message, &key.public_key(), &nonce, "one", 3));
        let mut later = message.clone();
        later.now_nanos = "1900000000000000000".into();
        assert!(!verify_resume(&later, &key.public_key(), &nonce, "one", 3));
        let mut reseeded = message.clone();
        reseeded.seed = URL_SAFE_NO_PAD.encode([8u8; SEED_BYTES]);
        assert!(!verify_resume(&reseeded, &key.public_key(), &nonce, "one", 3));
        let short = resume(&key, &nonce, "one", 3, 1, &[1u8; 8]);
        assert!(!verify_resume(&short, &key.public_key(), &nonce, "one", 3), "a seed has its fixed size");
        assert!(!verify_resume(&message, &key.public_key(), &nonce, "one", 4));
    }

    #[tokio::test]
    async fn the_sleep_listener_refuses_an_ordinary_process() {
        assert!(Listener::bind(VMADDR_CID_HOST + 1).is_err());
    }
}
