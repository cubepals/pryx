// SPDX-License-Identifier: FSL-1.1-ALv2
//! A bounded gated child tests descriptor inheritance without risking its serving parent.
//! The child transfers its own pidfd before a start token permits close_range. Timeout and
//! cancellation retire that exact handle; no optional clone flag or pidfd probe runs in parent.
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal, sigaction, sigprocmask};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::{ForkResult, Pid, fork};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags};
use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags};
use rustix::process::pidfd_send_signal;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const READY: &[u8] = b"R";
const START: &[u8] = b"S";

#[expect(unsafe_code, reason = "Thin checked wrapper for the kernel close_range descriptor operation")]
pub(super) fn close_on_exec() -> libc::c_long {
    // SAFETY: close_range takes only integer arguments. CLOEXEC leaves every descriptor open
    // until exec, including Rust's private pipe for reporting a spawn error to the parent.
    unsafe {
        libc::syscall(
            libc::SYS_close_range,
            (libc::STDERR_FILENO + 1) as libc::c_uint,
            libc::c_uint::MAX,
            libc::CLOSE_RANGE_CLOEXEC,
        )
    }
}

#[expect(unsafe_code, reason = "Read-only signal query establishes exclusive known-child wait ownership")]
fn owns_waits() -> io::Result<()> {
    let mut action = MaybeUninit::<libc::sigaction>::uninit();
    // SAFETY: a null action queries without changing the parent's signal state.
    if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), action.as_mut_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful sigaction initialized the complete output.
    let action = unsafe { action.assume_init() };
    if action.sa_sigaction == libc::SIG_IGN || action.sa_flags & libc::SA_NOCLDWAIT != 0 {
        return Err(io::Error::other("descriptor probes require owned child reaping"));
    }
    Ok(())
}

#[expect(unsafe_code, reason = "Only thin async-safe syscall wrappers run before the child's _exit")]
fn child(socket: i32, parent_socket: i32, parent: libc::pid_t, default: &SigAction, mask: &SigSet, seconds: u32) -> ! {
    // SAFETY: discard only the child's copy of the parent endpoint; no Rust owner is dropped.
    unsafe { libc::close(parent_socket) };
    // SAFETY: reset inherited Rust handlers; an unblocked alarm bounds even setup calls.
    let signals = unsafe {
        sigaction(Signal::SIGSYS, default).is_ok()
            && sigaction(Signal::SIGALRM, default).is_ok()
            && sigprocmask(SigmaskHow::SIG_UNBLOCK, Some(mask), None).is_ok()
    };
    // SAFETY: alarm changes only this child's process timer and runs no Rust callback.
    unsafe { libc::alarm(seconds) };
    let parent_death = rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL)).is_ok();
    // SAFETY: getppid reads identity after arming parent death, closing the setup race.
    let still_parent = unsafe { libc::getppid() } == parent;
    let ready = signals
        && parent_death
        && still_parent
        && rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable).is_ok();
    // Optional pidfd support is tested only here. This self handle transfers exact
    // kernel identity rather than reopening a PID which could already have been reused.
    if ready
        && let Ok(descriptor) =
            rustix::process::pidfd_open(rustix::process::getpid(), rustix::process::PidfdFlags::empty())
    {
        let files = [descriptor.as_fd()];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        // SAFETY: this endpoint remains open until _exit.
        let socket_fd = unsafe { BorrowedFd::borrow_raw(socket) };
        let sent = control.push(SendAncillaryMessage::ScmRights(&files))
            && rustix::net::sendmsg(
                socket_fd,
                &[io::IoSlice::new(READY)],
                &mut control,
                SendFlags::DONTWAIT | SendFlags::NOSIGNAL,
            )
            .is_ok_and(|count| count == READY.len());
        if sent {
            let mut start = [0];
            let received = loop {
                // SAFETY: receive only a fixed private start token into this stack byte.
                let count = unsafe { libc::recv(socket, start.as_mut_ptr().cast(), start.len(), 0) };
                if count < 0 && nix::errno::Errno::last() == nix::errno::Errno::EINTR {
                    continue;
                }
                break count;
            };
            if received == START.len() as isize && start == START && close_on_exec() == 0 {
                // SAFETY: no Rust destructor, allocation, formatting or unwinding follows fork.
                unsafe { libc::_exit(libc::EXIT_SUCCESS) }
            }
        }
        // SAFETY: leave this child-created descriptor to kernel exit as well.
        unsafe { libc::_exit(libc::EXIT_FAILURE) }
    }
    // SAFETY: all owned copies are closed by kernel exit, never inherited Rust cleanup.
    unsafe { libc::_exit(libc::EXIT_FAILURE) }
}

struct Probe {
    pid: Pid,
    descriptor: Option<OwnedFd>,
    gate: Option<UnixStream>,
    reaped: bool,
}

fn ready(descriptor: &impl AsFd, deadline: Instant) -> io::Result<()> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "descriptor probe deadline elapsed"))?;
        let mut descriptors = [PollFd::new(descriptor, PollFlags::IN)];
        match poll(&mut descriptors, Some(&Timespec::try_from(remaining).map_err(io::Error::other)?)) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::TimedOut, "descriptor probe deadline elapsed")),
            Ok(_) if descriptors[0].revents().intersects(PollFlags::IN | PollFlags::HUP) => return Ok(()),
            Ok(_) => return Err(io::Error::other("descriptor probe endpoint is not waitable")),
            Err(rustix::io::Errno::INTR) => continue,
            Err(cause) => return Err(cause.into()),
        }
    }
}

impl Probe {
    #[expect(unsafe_code, reason = "Only the forked child follows the syscall-only _exit path")]
    fn spawn(timeout: Duration) -> io::Result<Self> {
        owns_waits()?;
        let (parent_socket, child_socket) = UnixStream::pair()?;
        let default = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
        let mut mask = SigSet::empty();
        mask.add(Signal::SIGALRM);
        let seconds = timeout
            .as_secs()
            .saturating_add(u64::from(timeout.subsec_nanos() != 0))
            .max(1)
            .try_into()
            .map_err(io::Error::other)?;
        // SAFETY: getpid reads only this parent's identity for the child setup race check.
        let parent = unsafe { libc::getpid() };
        // SAFETY: only async-safe wrappers and _exit run in the child. Every signal state,
        // timeout and endpoint needed there was prepared before fork; no Rust cleanup runs.
        let pid = match unsafe { fork() }? {
            ForkResult::Parent { child } => child,
            ForkResult::Child => {
                child(child_socket.as_raw_fd(), parent_socket.as_raw_fd(), parent, &default, &mask, seconds)
            }
        };
        drop(child_socket);
        Ok(Self { pid, descriptor: None, gate: Some(parent_socket), reaped: false })
    }

    fn start(&mut self, deadline: Instant) -> io::Result<()> {
        let socket = self.gate.as_ref().expect("owned probe gate");
        ready(socket, deadline)?;
        let mut payload = [0];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let message = rustix::net::recvmsg(
            socket,
            &mut [io::IoSliceMut::new(&mut payload)],
            &mut control,
            RecvFlags::CMSG_CLOEXEC | RecvFlags::DONTWAIT,
        )?;
        let mut descriptor = None;
        for message in control.drain() {
            if let RecvAncillaryMessage::ScmRights(files) = message {
                for file in files {
                    if descriptor.replace(file).is_some() {
                        return Err(io::Error::other("extra descriptor probe identity"));
                    }
                }
            } else {
                return Err(io::Error::other("invalid descriptor probe identity"));
            }
        }
        if message.bytes != READY.len()
            || payload != READY
            || !message.flags.intersection(ReturnFlags::TRUNC | ReturnFlags::CTRUNC).is_empty()
        {
            return Err(io::Error::other("invalid descriptor probe handoff"));
        }
        self.descriptor = Some(descriptor.ok_or_else(|| io::Error::other("missing descriptor probe identity"))?);
        if rustix::net::send(socket, START, SendFlags::DONTWAIT | SendFlags::NOSIGNAL)? != START.len() {
            return Err(io::Error::other("descriptor probe did not start"));
        }
        drop(self.gate.take());
        Ok(())
    }

    fn reap(&mut self) -> io::Result<Option<WaitStatus>> {
        // Auto-reaping was refused before fork. The raw child is never registered with
        // Tokio, and Processes holds its spawn lock through workload/PTY registration,
        // excluding its orphan scan until this owner finishes. This PID therefore stays
        // waitable here; only the pidfd is ever used for signaling. Normal wait4 also works
        // where a host refuses the optional P_PIDFD observation mode.
        loop {
            match waitpid(self.pid, None) {
                Ok(status @ (WaitStatus::Exited(..) | WaitStatus::Signaled(..))) => {
                    self.reaped = true;
                    return Ok(Some(status));
                }
                Err(nix::errno::Errno::EINTR) => continue,
                Err(nix::errno::Errno::ECHILD) => {
                    self.reaped = true;
                    return Ok(None);
                }
                outcome => return Err(io::Error::other(format!("owned descriptor probe {outcome:?}"))),
            }
        }
    }

    fn retire(&mut self) -> io::Result<()> {
        if self.reaped {
            return Ok(());
        }
        drop(self.gate.take());
        if let Some(descriptor) = &self.descriptor {
            let signal = pidfd_send_signal(descriptor, rustix::process::Signal::KILL);
            // A failed signal still waits for the child's unblocked default alarm. All
            // signaling uses the transferred kernel handle, never a numeric PID. Once the
            // handle is terminal, consume this owner's child wait before dropping anything.
            ready(descriptor, Instant::now() + crate::process_namespaces::PROBE_TIMEOUT)?;
            self.reap()?;
            match signal {
                Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
                Err(cause) => Err(cause.into()),
            }
        } else {
            // The gate never opened, so no close_range call can be held. Closing it exits
            // a waiting child; its default alarm bounds a blocked self-pidfd setup call.
            self.reap().map(|_| ())
        }
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        if let Err(cause) = self.retire() {
            eprintln!("Process descriptor probe retirement failed: {cause}");
        }
    }
}

pub(super) fn run() -> io::Result<()> {
    let timeout = crate::process_namespaces::PROBE_TIMEOUT;
    let deadline = Instant::now() + timeout;
    let mut probe = Probe::spawn(timeout)?;
    let result = (|| {
        probe.start(deadline)?;
        ready(probe.descriptor.as_ref().expect("owned probe pidfd"), deadline)?;
        probe.reap()
    })();
    probe.retire()?;
    match result? {
        Some(WaitStatus::Exited(_, libc::EXIT_SUCCESS)) => Ok(()),
        status => Err(io::Error::other(format!("owned descriptor probe {status:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::process::{WaitId, WaitIdOptions, waitid};
    use std::os::fd::FromRawFd;

    fn deny_syscall(number: i64, rules: Vec<seccompiler::SeccompRule>) {
        use seccompiler::{BpfProgram, SeccompAction, SeccompFilter};
        let filter: BpfProgram = SeccompFilter::new(
            [(number, rules)].into_iter().collect(),
            SeccompAction::Allow,
            SeccompAction::Errno(libc::EPERM as u32),
            std::env::consts::ARCH.try_into().unwrap(),
        )
        .unwrap()
        .try_into()
        .unwrap();
        crate::syscall_abi::apply(&filter).unwrap();
    }

    fn deny_pidfd_wait() {
        use seccompiler::{SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompRule};
        let pidfd_mode = SeccompRule::new(vec![
            SeccompCondition::new(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, libc::P_PIDFD as u64).unwrap(),
        ])
        .unwrap();
        deny_syscall(libc::SYS_waitid, vec![pidfd_mode]);
    }

    #[test]
    #[expect(unsafe_code, reason = "Query only the owned isolated test process's signal disposition")]
    fn optional_pidfd_wait_denial_preserves_probe_truth_and_reaping() {
        const CASE: &str = "PRYX_TEST_OWNED_WAIT_MODE";
        let Some(case) = std::env::var_os(CASE) else {
            for case in ["success", "failure"] {
                let thread = std::thread::current();
                let result = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", thread.name().unwrap()])
                    .env_clear()
                    .env("PATH", "/usr/bin:/bin")
                    .env(CASE, case)
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{}{}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                );
                assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
            }
            return;
        };
        let descriptor = tempfile::tempfile().unwrap();
        let flags = rustix::io::fcntl_getfd(&descriptor).unwrap();
        let mut before = MaybeUninit::<libc::sigaction>::uninit();
        // SAFETY: query without changing any disposition; successful call initializes output.
        let before = unsafe {
            assert_eq!(libc::sigaction(libc::SIGCHLD, std::ptr::null(), before.as_mut_ptr()), 0);
            before.assume_init()
        };
        deny_pidfd_wait();
        if case == "failure" {
            deny_syscall(libc::SYS_close_range, Vec::new());
        }
        let mut command = std::process::Command::new("/bin/true");
        let result = super::super::inherit_only(&mut command, Vec::new());
        if case == "failure" {
            assert!(result.unwrap_err().to_string().starts_with("Process descriptor inheritance refused:"));
        } else {
            result.unwrap();
            assert!(command.status().unwrap().success());
        }
        assert_eq!(waitpid(None, Some(nix::sys::wait::WaitPidFlag::WNOHANG)), Err(nix::errno::Errno::ECHILD));
        assert_eq!(rustix::io::fcntl_getfd(&descriptor).unwrap(), flags);
        let mut after = MaybeUninit::<libc::sigaction>::uninit();
        // SAFETY: query only this owned process's disposition and initialized output.
        let after = unsafe {
            assert_eq!(libc::sigaction(libc::SIGCHLD, std::ptr::null(), after.as_mut_ptr()), 0);
            after.assume_init()
        };
        assert_eq!(after.sa_sigaction, before.sa_sigaction);
        assert_eq!(after.sa_flags, before.sa_flags);
    }

    #[expect(unsafe_code, reason = "Only an isolated owned test process creates this mediation listener")]
    fn held_listener() -> OwnedFd {
        let instructions = [
            libc::sock_filter {
                code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
                jt: 0,
                jf: 0,
                k: std::mem::offset_of!(libc::seccomp_data, nr) as u32,
            },
            libc::sock_filter {
                code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
                jt: 0,
                jf: 1,
                k: libc::SYS_close_range as u32,
            },
            libc::sock_filter {
                code: (libc::BPF_RET | libc::BPF_K) as u16,
                jt: 0,
                jf: 0,
                k: libc::SECCOMP_RET_USER_NOTIF,
            },
            libc::sock_filter { code: (libc::BPF_RET | libc::BPF_K) as u16, jt: 0, jf: 0, k: libc::SECCOMP_RET_ALLOW },
        ];
        let program = libc::sock_fprog { len: instructions.len() as u16, filter: instructions.as_ptr().cast_mut() };
        // SAFETY: this isolated test thread changes only its owned descendants' policy.
        assert_eq!(unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) }, 0);
        // SAFETY: the initialized filter is borrowed until this owned seccomp call returns.
        let raw = unsafe {
            libc::syscall(
                libc::SYS_seccomp,
                libc::SECCOMP_SET_MODE_FILTER,
                libc::SECCOMP_FILTER_FLAG_NEW_LISTENER,
                &program,
            )
        };
        assert!(raw >= 0);
        // SAFETY: successful seccomp transfers this one owned listener descriptor.
        unsafe { OwnedFd::from_raw_fd(raw as i32) }
    }

    #[test]
    #[expect(unsafe_code, reason = "Only owned isolated subprocesses install the held-notification test filter")]
    fn deadline_and_cancellation_retire_the_exact_held_child() {
        const CASE: &str = "PRYX_TEST_OWNED_PROBE_LIFETIME";
        let Some(case) = std::env::var_os(CASE) else {
            for case in ["deadline", "cancel", "autoreap", "cancel-waitid"] {
                let thread = std::thread::current();
                let result = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", thread.name().unwrap()])
                    .env_clear()
                    .env("PATH", "/usr/bin:/bin")
                    .env(CASE, case)
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{}{}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                );
                assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
            }
            return;
        };
        if case == "autoreap" {
            let ignore = SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty());
            // SAFETY: only this owned isolated test process changes its SIGCHLD disposition.
            unsafe { sigaction(Signal::SIGCHLD, &ignore) }.unwrap();
            let error = Probe::spawn(Duration::from_secs(1)).err().unwrap();
            assert!(error.to_string().contains("owned child reaping"));
            let mut after = MaybeUninit::<libc::sigaction>::uninit();
            // SAFETY: query only the disposition in this isolated owned test process.
            assert_eq!(unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), after.as_mut_ptr()) }, 0);
            // SAFETY: successful sigaction initialized the complete output.
            assert_eq!(unsafe { after.assume_init() }.sa_sigaction, libc::SIG_IGN);
            return;
        }
        if case == "cancel-waitid" {
            deny_pidfd_wait();
        }
        let listener = held_listener();
        let budget = Duration::from_secs(1);
        let mut probe = Probe::spawn(budget).unwrap();
        probe.start(Instant::now() + budget).unwrap();
        ready(&listener, Instant::now() + budget).unwrap();
        let mut notification = MaybeUninit::<libc::seccomp_notif>::zeroed();
        assert_eq!(
            // SAFETY: ioctl fills only this owned notification structure, never FD contents.
            unsafe { libc::ioctl(listener.as_raw_fd(), libc::SECCOMP_IOCTL_NOTIF_RECV, notification.as_mut_ptr()) },
            0
        );
        // SAFETY: successful ioctl initialized the complete owned notification.
        let notification = unsafe { notification.assume_init() };
        assert_eq!(notification.data.nr, libc::SYS_close_range as i32);
        let identity = probe.descriptor.as_ref().unwrap().try_clone().unwrap();
        if case == "deadline" {
            let result = ready(&identity, Instant::now() + Duration::from_millis(20));
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
            probe.retire().unwrap();
        }
        drop(probe);
        ready(&identity, Instant::now() + budget).unwrap();
        if case == "cancel-waitid" {
            assert_eq!(waitpid(None, Some(nix::sys::wait::WaitPidFlag::WNOHANG)), Err(nix::errno::Errno::ECHILD));
        } else {
            assert_eq!(
                waitid(WaitId::PidFd(identity.as_fd()), WaitIdOptions::EXITED | WaitIdOptions::NOHANG).unwrap_err(),
                rustix::io::Errno::CHILD
            );
        }
    }
}
