// SPDX-License-Identifier: FSL-1.1-ALv2
//! Optional guest-only management socket activation. The ordinary agent receives
//! the exact listener queues and the unread first host stream before applying its
//! normal VM ingress policy. All authentication and relay logic remain in ingress.
use crate::contract::AGENT_PORT;
use crate::vm_boot;
use clap::Parser;
#[cfg(feature = "lab-guest-supervisor-gate")]
use rustix::event::{PollFd, PollFlags, poll};
use rustix::io::{FdFlags, fcntl_getfd, fcntl_setfd};
use rustix::net::sockopt::{socket_acceptconn, socket_domain, socket_type};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
#[cfg(feature = "lab-guest-supervisor-gate")]
use std::process::Command;
use std::sync::Mutex;
use tokio_vsock::{VMADDR_CID_HOST, VsockAddr, VsockListener, VsockStream};

const MODE: &str = "lab-inherited-vm-management";
static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid lab VM management handoff")
}

fn local_port(framed: bool) -> u32 {
    if framed { vm_boot::stream_port(AGENT_PORT) } else { vm_boot::service_port(AGENT_PORT) }
}

fn ordinary_guest(cid: u32) -> io::Result<()> {
    if !vm_boot::guest_cid(cid)
        || rustix::process::getppid() != Some(rustix::process::Pid::INIT)
        || rustix::process::geteuid().is_root()
        || rustix::process::getegid().is_root()
        || rustix::process::dumpable_behavior()? != rustix::process::DumpableBehavior::NotDumpable
        || !rustix::thread::no_new_privs()?
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(feature = "lab-guest-supervisor-gate")]
pub(crate) struct Handoff {
    cid: u32,
    framed: bool,
    raw: OwnedFd,
    streams: OwnedFd,
    first: OwnedFd,
}

#[cfg(feature = "lab-guest-supervisor-gate")]
impl Handoff {
    pub(crate) fn pass(&self, command: &mut Command) -> io::Result<()> {
        command.arg(MODE).arg(self.cid.to_string()).arg(if self.framed { "framed" } else { "raw" });
        for fd in [&self.raw, &self.streams, &self.first] {
            fcntl_setfd(fd, fcntl_getfd(fd)? - FdFlags::CLOEXEC)?;
            command.arg(fd.as_raw_fd().to_string());
        }
        Ok(())
    }
}

#[cfg(feature = "lab-guest-supervisor-gate")]
pub(crate) fn wait_for_demand(arguments: &[String]) -> io::Result<Option<Handoff>> {
    let parsed = crate::bootstrap::Bootstrap::try_parse_from(
        std::iter::once("pryx-agent").chain(arguments.iter().map(String::as_str)),
    )
    .map_err(|_| invalid())?;
    let Some(cid) = parsed.lab_management_cid() else { return Ok(None) };
    ordinary_guest(cid)?;
    let raw = vsock::VsockListener::bind_with_cid_port(cid, local_port(false))?;
    let framed = vsock::VsockListener::bind_with_cid_port(cid, local_port(true))?;
    raw.set_nonblocking(true)?;
    framed.set_nonblocking(true)?;
    loop {
        let mut pollers = [PollFd::new(&raw, PollFlags::IN), PollFd::new(&framed, PollFlags::IN)];
        match poll(&mut pollers, None) {
            Ok(_) => (),
            Err(rustix::io::Errno::INTR) => continue,
            Err(err) => return Err(err.into()),
        }
        let readiness = [pollers[0].revents(), pollers[1].revents()];
        for (i, flags) in readiness.into_iter().enumerate() {
            if flags.intersects(PollFlags::ERR | PollFlags::NVAL | PollFlags::HUP) {
                return Err(invalid());
            }
            if !flags.contains(PollFlags::IN) {
                continue;
            }
            let listener = if i == 0 { &raw } else { &framed };
            let (peer, address) = match listener.accept() {
                Ok(peer) => peer,
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    continue;
                }
                Err(err) => return Err(err),
            };
            if address.cid() == VMADDR_CID_HOST {
                #[cfg(feature = "lab-first-request-proof")]
                {
                    // Functional fixture only. Nothing is read or acknowledged: the
                    // first stream contains actual client bytes and a second peer
                    // has reached this same queue before any full server exec.
                    require_readable(&peer)?;
                    require_readable(listener)?;
                }
                return Ok(Some(Handoff {
                    cid,
                    framed: i == 1,
                    raw: raw.into(),
                    streams: framed.into(),
                    first: peer.into(),
                }));
            }
        }
    }
}

#[cfg(feature = "lab-first-request-proof")]
fn require_readable(fd: &impl std::os::fd::AsFd) -> io::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let bound = rustix::time::Timespec::try_from(remaining).map_err(|_| invalid())?;
        let mut fds = [PollFd::new(fd, PollFlags::IN)];
        match poll(&mut fds, Some(&bound)) {
            Ok(0) => return Err(io::ErrorKind::TimedOut.into()),
            Ok(_) if fds[0].revents().contains(PollFlags::IN) => return Ok(()),
            Err(rustix::io::Errno::INTR) => continue,
            Ok(_) => return Err(invalid()),
            Err(error) => return Err(error.into()),
        }
    }
}

struct Pending {
    cid: u32,
    queues: [Option<OwnedFd>; 2],
    first: Option<(bool, OwnedFd, VsockAddr)>,
}

#[expect(unsafe_code, reason = "validate the inherited raw descriptor with fcntl before owning it exactly once")]
fn owned(raw: RawFd) -> io::Result<OwnedFd> {
    // This is the first operation in a single-threaded exec. A failed check exits
    // the entire invocation, so no intervening open can reuse the descriptor.
    if unsafe { libc::fcntl(raw, libc::F_GETFD) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    fcntl_setfd(&fd, fcntl_getfd(&fd)? | FdFlags::CLOEXEC)?;
    Ok(fd)
}

fn local(fd: &OwnedFd, cid: u32, framed: bool, listening: bool) -> io::Result<()> {
    if socket_domain(fd)? != rustix::net::AddressFamily::VSOCK
        || socket_type(fd)? != rustix::net::SocketType::STREAM
        || socket_acceptconn(fd)? != listening
    {
        return Err(invalid());
    }
    let address = nix::sys::socket::getsockname::<VsockAddr>(fd.as_raw_fd())?;
    if address.cid() != cid || address.port() != local_port(framed) {
        return Err(invalid());
    }
    Ok(())
}

fn frame(arguments: &[String]) -> io::Result<(u32, bool, [RawFd; 3])> {
    let [_, cid, role, raw, stream, peer, ..] = arguments else { return Err(invalid()) };
    let cid = cid.parse().map_err(|_| invalid())?;
    let framed = match role.as_str() {
        "raw" => false,
        "framed" => true,
        _ => return Err(invalid()),
    };
    let fds = [
        raw.parse().map_err(|_| invalid())?,
        stream.parse().map_err(|_| invalid())?,
        peer.parse().map_err(|_| invalid())?,
    ];
    if fds.iter().any(|&fd| fd < 3) || fds[0] == fds[1] || fds[0] == fds[2] || fds[1] == fds[2] {
        return Err(invalid());
    }
    Ok((cid, framed, fds))
}

pub fn adopt(arguments: &mut Vec<String>) -> io::Result<()> {
    if arguments.first().is_none_or(|arg| arg != MODE) {
        return Ok(());
    }
    // exec resets Linux dumpability. Close the import window before touching the
    // descriptors or the boot arguments in this new serving process.
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    let (cid, framed, [raw, stream, peer]) = frame(arguments)?;
    ordinary_guest(cid)?;
    let [raw, stream, peer] = [owned(raw)?, owned(stream)?, owned(peer)?];
    local(&raw, cid, false, true)?;
    local(&stream, cid, true, true)?;
    local(&peer, cid, framed, false)?;
    let remote = nix::sys::socket::getpeername::<VsockAddr>(peer.as_raw_fd())?;
    if remote.cid() != VMADDR_CID_HOST {
        return Err(invalid());
    }
    let parsed = crate::bootstrap::Bootstrap::try_parse_from(
        std::iter::once("pryx-agent").chain(arguments[6..].iter().map(String::as_str)),
    )
    .map_err(|_| invalid())?;
    if parsed.lab_management_cid() != Some(cid) {
        return Err(invalid());
    }
    let mut pending = PENDING.lock().unwrap();
    if pending.is_some() {
        return Err(invalid());
    }
    *pending = Some(Pending { cid, queues: [Some(raw), Some(stream)], first: Some((framed, peer, remote)) });
    arguments.drain(..6);
    Ok(())
}

pub(crate) struct Listener {
    socket: VsockListener,
    first: Mutex<Option<(VsockStream, VsockAddr)>>,
}

impl Listener {
    pub(crate) fn bind(address: VsockAddr) -> io::Result<Self> {
        let mut state = PENDING.lock().unwrap();
        if let Some(pending) = state.as_mut() {
            if pending.cid != address.cid() {
                return Err(invalid());
            }
            if let Some(framed) = [false, true].into_iter().find(|&framed| local_port(framed) == address.port()) {
                let fd = pending.queues[usize::from(framed)].take().ok_or_else(invalid)?;
                let first = if pending.first.as_ref().is_some_and(|(kind, _, _)| *kind == framed) {
                    pending.first.take().map(|(_, peer, remote)| (VsockStream::from(peer), remote))
                } else {
                    None
                };
                return Ok(Self { socket: VsockListener::from(fd), first: Mutex::new(first) });
            }
        }
        Ok(Self { socket: VsockListener::bind(address)?, first: Mutex::new(None) })
    }

    pub(crate) async fn accept(&self) -> io::Result<(VsockStream, VsockAddr)> {
        let first = self.first.lock().unwrap().take();
        if let Some(first) = first {
            return Ok(first);
        }
        self.socket.accept().await
    }
}

pub(crate) fn assert_consumed() -> io::Result<()> {
    if let Some(held) = PENDING.lock().unwrap().take() {
        if held.queues.iter().any(Option::is_some) || held.first.is_some() {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_roles_and_aliased_or_standard_descriptors_are_refused() {
        for words in ["raw 3 4 5", "framed 6 8 9", "other 3 4 5", "raw 2 4 5", "raw 3 4 3", "raw 3 3 5"] {
            let args: Vec<_> = [MODE, "7"].into_iter().chain(words.split_whitespace()).map(str::to_owned).collect();
            let should_pass = matches!(words, "raw 3 4 5" | "framed 6 8 9");
            assert_eq!(frame(&args).is_ok(), should_pass);
        }
        assert_ne!(local_port(false), local_port(true));
    }

    #[test]
    fn an_ordinary_unix_socket_cannot_be_adopted_as_either_vsock_queue() {
        use std::os::unix::net::UnixStream;
        let (a, _b) = UnixStream::pair().unwrap();
        let fd: OwnedFd = a.into();
        assert!(local(&fd, 7, false, true).is_err());
        assert!(local(&fd, 7, true, false).is_err());
    }

    #[test]
    fn ordinary_host_cannot_activate_the_guest_mode() {
        assert!(ordinary_guest(9).is_err());
        let mut args = vec!["--version".to_string()];
        assert!(adopt(&mut args).is_ok());
        assert_eq!(args, ["--version"]);
    }
}
