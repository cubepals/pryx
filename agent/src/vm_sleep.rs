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

/// The sleep and resume exchange this init answers, as its bootstrap capabilities announce: 1 for
/// sleep and wake, 2 once a resume can also hand a fork with memory its own identity, 3 once it
/// can also hand the guest the ports of the host gateways its wake listens on.
pub const VERSION: u16 = 3;
/// The first version that sleeps and wakes.
pub const SLEEP_VERSION: u16 = 1;
/// The first version whose resume carries a fork's identity.
pub const FORK_VERSION: u16 = 2;
/// The first version whose resume carries the woken guest's gateway ports.
pub const GATEWAY_VERSION: u16 = 3;
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
/// How often a held guest looks for its connection's reset: a woken guest notices within this.
const HOLD_CHECK: Duration = Duration::from_millis(50);

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
    /// A fork's own identity, taken in place of its source's; absent for a sleep's own wake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<Identity>,
    /// The host gateways this wake listens on, which the guest's redirects then target; absent
    /// for a guest without a network.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateways: Option<Gateways>,
    pub signature: String,
}

/// The host ports of a woken guest's HTTP, TLS and DNS gateways. Each wake listens anew, so they
/// are rarely the ports the guest's memory still names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Gateways {
    pub http: u16,
    pub tls: u16,
    pub dns: u16,
}

impl Gateways {
    fn valid(&self) -> bool {
        let ports = [self.http, self.tls, self.dns];
        !ports.contains(&0) && ports[0] != ports[1] && ports[1] != ports[2] && ports[0] != ports[2]
    }
}

/// A fork with memory's identity: its sandbox, epoch and agent signing key (a secret, carried on
/// this private channel only, as the bootstrap carries the first one), a fresh boot nonce for its
/// own shutdown proofs, and its hostname.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    pub sandbox: String,
    pub epoch: u64,
    pub key: String,
    pub nonce: String,
    pub hostname: String,
}

impl Identity {
    fn valid(&self) -> bool {
        !self.sandbox.is_empty()
            && nonce_valid(&self.nonce)
            && !self.hostname.is_empty()
            && self.hostname.len() <= HOSTNAME_BYTES
            && self.hostname.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && SigningKey::from_encoded(&self.key).is_ok()
    }
}
// The kernel's __NEW_UTS_LEN (include/uapi/linux/utsname.h), which sethostname(2) enforces;
// libc's HOST_NAME_MAX says 255 on musl, which this init is built with.
const HOSTNAME_BYTES: usize = 64;

fn request_bytes(nonce: &str, sandbox: &str, epoch: u64) -> Vec<u8> {
    serde_json::to_vec(&(REQUEST_CONTEXT, nonce, sandbox, epoch)).expect("fixed sleep request serializes")
}

fn resume_bytes(resume: &Resume) -> Vec<u8> {
    let fields = (RESUME_CONTEXT, &resume.nonce, &resume.sandbox, resume.epoch, &resume.now_nanos, &resume.seed);
    // What versions 1 and 2 signed stays as it was; gateways are signed after the rest.
    match (&resume.identity, &resume.gateways) {
        (None, None) => serde_json::to_vec(&fields),
        (Some(identity), None) => serde_json::to_vec(&(fields, identity)),
        (identity, Some(gateways)) => serde_json::to_vec(&(fields, identity, gateways)),
    }
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
    fork(key, nonce, sandbox, epoch, now_nanos, seed, Handed::default())
}

/// What a resume hands the guest beside the time and the seed.
#[derive(Clone, Default)]
pub struct Handed {
    /// A fork's own identity.
    pub identity: Option<Identity>,
    /// The ports of the gateways the wake listens on, for a guest with a network.
    pub gateways: Option<Gateways>,
}

/// A resume signed by the guest's current identity that hands it `handed` too.
pub fn fork(
    key: &SigningKey,
    nonce: &str,
    sandbox: &str,
    epoch: u64,
    now_nanos: i128,
    seed: &[u8],
    handed: Handed,
) -> Resume {
    let mut resume = Resume {
        nonce: nonce.into(),
        sandbox: sandbox.into(),
        epoch,
        now_nanos: now_nanos.to_string(),
        seed: URL_SAFE_NO_PAD.encode(seed),
        identity: handed.identity,
        gateways: handed.gateways,
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
        && resume.identity.as_ref().is_none_or(Identity::valid)
        && resume.gateways.as_ref().is_none_or(Gateways::valid)
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

/// How a held connection ended: the host closed it, or the monitor reset every vsock connection,
/// as a snapshot does (Firecracker queues the transport reset as it saves the devices' state).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    Closed,
    Reset,
}

impl Held {
    /// Dials the host's hold listener. Its peer is the host, as every vsock peer of a guest is.
    pub(crate) async fn dial() -> io::Result<Self> {
        let stream =
            tokio::time::timeout(RESUME_WAIT, VsockStream::connect(VsockAddr::new(VMADDR_CID_HOST, HOLD_PORT)))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "VM sleep hold was not accepted"))??;
        Ok(Self(stream))
    }
    /// Returns when the connection ends, whichever way: the host never sends on it. A restore's
    /// vsock transport reset leaves the socket with only an error (ECONNRESET), which epoll
    /// reports as EPOLLERR without EPOLLIN or EPOLLHUP (af_vsock's `vsock_poll`), and tokio
    /// wakes no reader on an error alone; so the error is also looked for on a short timer.
    pub(crate) async fn ended(&mut self) -> End {
        let mut byte = [0u8; 1];
        loop {
            tokio::select! {
                read = self.0.read(&mut byte) => match read {
                    Ok(0) => return End::Closed,
                    Err(_) => return End::Reset,
                    Ok(_) => (),
                },
                () = tokio::time::sleep(HOLD_CHECK) => {
                    if !matches!(rustix::net::sockopt::socket_error(&self.0), Ok(Ok(()))) {
                        return End::Reset;
                    }
                }
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
    // Said on init's console, which the host keeps with the monitor's output.
    match tokio::time::timeout(RESUME_WAIT, exchange).await {
        Ok(Ok(Some(resumed))) => Some(resumed),
        Ok(Ok(None)) => {
            eprintln!("VM resume refused: it did not verify for this guest");
            None
        }
        Ok(Err(problem)) => {
            eprintln!("VM resume failed: {problem}");
            None
        }
        Err(_) => {
            eprintln!("VM resume did not come in time");
            None
        }
    }
}

/// What init tells the agent of a fork's identity, one JSON line on the agent's stdin.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Handover {
    pub(crate) sandbox: String,
    pub(crate) epoch: u64,
    pub(crate) key: String,
}

/// In the agent: takes trusted init's identity handovers from stdin, which no process the agent
/// starts inherits (stdin becomes /dev/null), and answers as each from then on.
pub(crate) fn take_handovers(agent: std::sync::Arc<crate::context::Agent>) -> io::Result<()> {
    let control = rustix::io::fcntl_dupfd_cloexec(rustix::stdio::stdin(), 3)?;
    rustix::stdio::dup2_stdin(std::fs::File::open("/dev/null")?)?;
    // Only init's pipe is a handover channel; any other stdin carries none.
    let Ok(receiver) = tokio::net::unix::pipe::Receiver::from_owned_fd(control) else { return Ok(()) };
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(receiver.take(MAX_BYTES * 16)).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(handover) = serde_json::from_str::<Handover>(&line) else { continue };
            let Ok(key) = SigningKey::from_encoded(&handover.key) else { continue };
            agent.ports.rename(handover.sandbox.clone());
            let forked = crate::context::Forked {
                public_key: key.public_key(),
                key,
                sandbox: handover.sandbox,
                epoch: handover.epoch,
            };
            *agent.forked.write().unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(std::sync::Arc::new(forked));
        }
    });
    Ok(())
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
    std::fs::write("/dev/urandom", seed)?;
    reseed()
}

/// Has the kernel rekey its random generator from its pool, which now holds the host's entropy,
/// so each fork of one memory draws bytes of its own at once. Writing to the pool alone doesn't
/// rekey it, and the pinned guest kernel has no VM generation ID driver, which would on restore.
#[cfg(target_arch = "x86_64")]
#[expect(unsafe_code, reason = "random(4)'s RNDRESEEDCRNG request has no safe wrapper in libc, rustix or nix")]
fn reseed() -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let random = std::fs::File::open("/dev/urandom")?;
    // SAFETY: RNDRESEEDCRNG takes no argument, and the descriptor stays open for the whole call.
    let answer = unsafe { libc::ioctl(random.as_raw_fd(), linux_raw_sys::ioctl::RNDRESEEDCRNG as libc::Ioctl) };
    if answer < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The VM tier is x86-64 only; no other guest takes a resume.
#[cfg(not(target_arch = "x86_64"))]
fn reseed() -> io::Result<()> {
    Err(io::Error::other("VM resume is x86-64 only"))
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

    #[test]
    fn a_resume_signs_its_gateways_and_keeps_what_older_ones_signed() {
        let (key, _) = SigningKey::generate().unwrap();
        let nonce = nonce();
        let seed = [7u8; SEED_BYTES];
        let gateways = Gateways { http: 41001, tls: 41002, dns: 41003 };
        let handed = Handed { identity: None, gateways: Some(gateways) };
        let message = fork(&key, &nonce, "one", 3, 1_800_000_000_123_456_789, &seed, handed);
        assert!(verify_resume(&message, &key.public_key(), &nonce, "one", 3));
        let mut moved = message.clone();
        moved.gateways = Some(Gateways { dns: 41004, ..gateways });
        assert!(!verify_resume(&moved, &key.public_key(), &nonce, "one", 3), "the ports are signed");
        let mut dropped = message.clone();
        dropped.gateways = None;
        assert!(!verify_resume(&dropped, &key.public_key(), &nonce, "one", 3));
        let repeated = Handed { identity: None, gateways: Some(Gateways { tls: 41001, ..gateways }) };
        let repeated = fork(&key, &nonce, "one", 3, 1, &seed, repeated);
        assert!(!verify_resume(&repeated, &key.public_key(), &nonce, "one", 3), "three distinct ports");
        // A resume without gateways is signed exactly as version 2 signed it.
        let plain = resume(&key, &nonce, "one", 3, 1, &seed);
        let fields = (RESUME_CONTEXT, &plain.nonce, &plain.sandbox, plain.epoch, &plain.now_nanos, &plain.seed);
        assert!(auth::verify_bytes(&key.public_key(), &serde_json::to_vec(&fields).unwrap(), &plain.signature).is_ok());
        assert!(!serde_json::to_string(&plain).unwrap().contains("gateways"));
    }

    #[tokio::test]
    async fn the_sleep_listener_refuses_an_ordinary_process() {
        assert!(Listener::bind(VMADDR_CID_HOST + 1).is_err());
    }
}
