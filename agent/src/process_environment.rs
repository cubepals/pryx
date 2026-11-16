// SPDX-License-Identifier: FSL-1.1-ALv2
//! Guest environment for process sandboxes is data, never the trusted helper's environment.
//! The node queues one sealed anonymous file on an inherited private Unix socket. The agent
//! receives it only after becoming non-dumpable, closes both descriptors before workers start,
//! and retains an immutable map for explicit configuration reads and confined workload execs.
//! No value is installed into the agent's process-wide environment or written to disk.
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixDatagram;
use std::sync::OnceLock;

use rustix::fs::{MemfdFlags, Mode, SealFlags};
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags};
use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags};

pub const VERSION: u16 = 1;
pub const ARGUMENT: &str = "--process-environment-fd";
pub const WORKLOAD_ARGUMENT: &str = "--process-workload-environment-fd";
pub const CHILD_MODE: &str = "process-child";
pub const UNSUPPORTED: &str = "Process supervision requires an agent with private environment handoff support";
// JSON escaping of the node's bounded environment and its private bootstrap defaults fits here.
const MAX_BYTES: u64 = 4 * crate::contract::MAX_REQUEST_BYTES as u64;
const MESSAGE: &[u8] = b"pryx-environment-v1";
const SEALED: SealFlags = SealFlags::SHRINK.union(SealFlags::GROW).union(SealFlags::WRITE).union(SealFlags::SEAL);
static ENVIRONMENT: OnceLock<BTreeMap<String, String>> = OnceLock::new();

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid private process environment handoff")
}

/// Allocate the private descriptor before building argv; no executable starts until queue().
pub struct Pending {
    sender: UnixDatagram,
    receiver: UnixDatagram,
}

impl Pending {
    pub fn new() -> io::Result<Self> {
        let (sender, receiver) = UnixDatagram::pair()?;
        Ok(Self { sender, receiver })
    }

    pub fn descriptor(&self) -> RawFd {
        self.receiver.as_raw_fd()
    }

    /// Queue the complete resolved environment, close its original descriptor and sender,
    /// and retain only the receiver for one child. No readable frame crosses helper exec.
    pub fn queue(self, environment: &BTreeMap<String, String>) -> io::Result<OwnedFd> {
        queue(self.sender, self.receiver, environment)
    }
}

fn queue(sender: UnixDatagram, receiver: UnixDatagram, environment: &BTreeMap<String, String>) -> io::Result<OwnedFd> {
    let file = rustix::fs::memfd_create("pryx-environment", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)?;
    let mut file = File::from(file);
    rustix::fs::fchmod(&file, Mode::RUSR)?;
    serde_json::to_writer(&mut file, environment).map_err(|_| invalid())?;
    if file.metadata()?.len() > MAX_BYTES {
        return Err(invalid());
    }
    file.seek(SeekFrom::Start(0))?;
    rustix::fs::fcntl_add_seals(&file, SEALED)?;
    let files = [file.as_fd()];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    if !control.push(SendAncillaryMessage::ScmRights(&files)) {
        return Err(invalid());
    }
    let sent = rustix::net::sendmsg(
        &sender,
        &[io::IoSlice::new(MESSAGE)],
        &mut control,
        SendFlags::DONTWAIT | SendFlags::NOSIGNAL,
    )?;
    if sent != MESSAGE.len() {
        return Err(invalid());
    }
    // Only the socket crosses helper execs. A proc FD open cannot read its queued SCM_RIGHTS.
    Ok(receiver.into())
}

fn receive_file(socket: &OwnedFd) -> io::Result<File> {
    let mut payload = [0; MESSAGE.len()];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = RecvAncillaryBuffer::new(&mut space);
    let message = rustix::net::recvmsg(
        socket,
        &mut [io::IoSliceMut::new(&mut payload)],
        &mut control,
        RecvFlags::CMSG_CLOEXEC | RecvFlags::DONTWAIT,
    )?;
    if message.flags.intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
        || message.bytes != MESSAGE.len()
        || payload.as_slice() != MESSAGE
    {
        return Err(invalid());
    }
    let mut files = Vec::new();
    for message in control.drain() {
        match message {
            RecvAncillaryMessage::ScmRights(rights) => files.extend(rights),
            _ => return Err(invalid()),
        }
    }
    if files.len() != 1 {
        return Err(invalid());
    }
    let file = files.pop().expect("one received file");
    if !rustix::fs::fcntl_get_seals(&file)?.contains(SEALED) {
        return Err(invalid());
    }
    let mut file = File::from(file);
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_BYTES {
        return Err(invalid());
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(file)
}

fn receive(socket: &OwnedFd) -> io::Result<BTreeMap<String, String>> {
    serde_json::from_reader(receive_file(socket)?.take(MAX_BYTES + 1)).map_err(|_| invalid())
}

/// Receive private data only after the receiving executable has disabled dumping.
#[expect(unsafe_code, reason = "Taking the parent's validated inherited socket before any thread")]
pub fn take(number: RawFd) -> io::Result<BTreeMap<String, String>> {
    if rustix::process::dumpable_behavior()? != rustix::process::DumpableBehavior::NotDumpable {
        return Err(invalid());
    }
    if number <= libc::STDERR_FILENO {
        return Err(invalid());
    }
    // Validate the descriptor before creating an OwnedFd, whose safety contract requires an
    // open descriptor. This bootstrap is single-threaded, so it cannot close concurrently.
    // SAFETY: F_GETFD accepts an arbitrary integer and neither reads nor changes its contents.
    if unsafe { libc::fcntl(number, libc::F_GETFD) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the validated node descriptor is transferred for one adoption before any spawn.
    let socket = unsafe { OwnedFd::from_raw_fd(number) };
    rustix::io::fcntl_setfd(&socket, rustix::io::FdFlags::CLOEXEC)?;
    receive(&socket)
}

/// Adopt once, before creating any worker. No later configuration read falls back to inheritance.
pub fn adopt(number: RawFd) -> io::Result<()> {
    ENVIRONMENT.set(take(number)?).map_err(|_| invalid())
}

/// Only the trusted process-child wrapper receives this second handoff. Its final exec applies
/// the map after the child-specific network and process-group policy, never to its own loader.
pub fn parse_workload(arguments: &mut Vec<String>) -> io::Result<Option<RawFd>> {
    if arguments.first().is_none_or(|value| value != WORKLOAD_ARGUMENT) {
        return Ok(None);
    }
    let number = arguments.get(1).ok_or_else(invalid)?.parse().map_err(|_| invalid())?;
    drop(arguments.drain(..2));
    if arguments.first().is_none_or(|value| value != CHILD_MODE) {
        return Err(invalid());
    }
    Ok(Some(number))
}

pub(crate) fn prepare_workload(
    arguments: &mut Vec<String>,
    environment: &BTreeMap<String, String>,
) -> io::Result<Option<OwnedFd>> {
    if !handed() {
        return Ok(None);
    }
    if arguments.get(1).is_none_or(|value| value != CHILD_MODE) {
        return Err(invalid());
    }
    let pending = Pending::new()?;
    arguments.insert(1, WORKLOAD_ARGUMENT.into());
    arguments.insert(2, pending.descriptor().to_string());
    Ok(Some(pending.queue(environment)?))
}

/// The supervisor's inherited environment already contains only these host-owned defaults.
pub(crate) fn loader() -> BTreeMap<OsString, OsString> {
    ["PATH", "HOME", "TMPDIR", "TERM"]
        .into_iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (name.into(), value)))
        .collect()
}

#[expect(unsafe_code, reason = "Only the forked child's copy of the parent's private socket is opened")]
pub(crate) fn inherit(command: &mut std::process::Command, socket: &OwnedFd) {
    use std::os::unix::process::CommandExt;
    let number = socket.as_raw_fd();
    // SAFETY: spawn's caller holds the socket until spawn returns. This child-only closure
    // allocates nothing and calls only the async-signal-safe fcntl operation.
    unsafe {
        command.pre_exec(move || {
            rustix::io::fcntl_setfd(std::os::fd::BorrowedFd::borrow_raw(number), rustix::io::FdFlags::empty())?;
            Ok(())
        });
    }
}

pub(crate) fn handed() -> bool {
    ENVIRONMENT.get().is_some()
}

/// Read an explicit agent configuration value without affecting any trusted loader or helper.
pub fn var(name: &str) -> Result<String, std::env::VarError> {
    match ENVIRONMENT.get() {
        Some(environment) => environment.get(name).cloned().ok_or(std::env::VarError::NotPresent),
        None => std::env::var(name),
    }
}

pub fn var_os(name: &str) -> Option<OsString> {
    match ENVIRONMENT.get() {
        Some(environment) => environment.get(name).map(|value| OsString::from(value.as_str())),
        None => std::env::var_os(name),
    }
}

pub(crate) fn allowed_for_workload(name: &str, private: &[&str]) -> bool {
    !private.contains(&name) && !name.starts_with(crate::contract::SECRET_ENV_PREFIX)
}

/// Only an already confined workload command receives defaults. Bootstrap material stays out.
pub(crate) fn workload(private: &[&str]) -> BTreeMap<String, String> {
    ENVIRONMENT
        .get()
        .into_iter()
        .flatten()
        .filter(|(name, _)| allowed_for_workload(name, private))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

#[cfg(test)]
#[path = "process_environment_tests.rs"]
mod tests;
