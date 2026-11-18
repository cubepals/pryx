// SPDX-License-Identifier: FSL-1.1-ALv2
//! Process-tier PTYs retain portable-pty allocation, resize and I/O. This spawn adapter adds
//! the child-only descriptor inheritance its CommandBuilder API cannot express. Guest values
//! enter only the final confined command; no parent descriptor ever becomes inheritable.
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal, sigaction, sigprocmask};
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

#[expect(
    unsafe_code,
    reason = "Only async-signal-safe signal, session and terminal operations run in the forked child"
)]
pub(super) fn spawn(
    argv: &[String],
    cwd: &Path,
    master: &dyn portable_pty::MasterPty,
    socket: &OwnedFd,
) -> io::Result<Box<dyn portable_pty::Child + Send + Sync>> {
    let path = master.tty_name().ok_or_else(|| io::Error::other("Terminal slave has no kernel path"))?;
    let slave = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(path)?;
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .env_clear()
        .envs(crate::process_environment::loader())
        .current_dir(cwd)
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    // Match portable-pty's inherited signal contract as well as its session setup. An
    // ignored HUP/QUIT or a blocked mask survives exec, including the process-child wrapper.
    // SAFETY: these maintained syscall wrappers perform no allocation and affect only this
    // forked child's signal state, session and fd 0.
    unsafe {
        command.pre_exec(|| {
            let default = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
            for signal in
                [Signal::SIGCHLD, Signal::SIGHUP, Signal::SIGINT, Signal::SIGQUIT, Signal::SIGTERM, Signal::SIGALRM]
            {
                sigaction(signal, &default)?;
            }
            sigprocmask(SigmaskHow::SIG_SETMASK, Some(&SigSet::empty()), None)?;
            rustix::process::setsid()?;
            rustix::process::ioctl_tiocsctty(rustix::stdio::stdin())?;
            Ok(())
        });
    }
    crate::process_environment::inherit(&mut command, socket);
    Ok(Box::new(command.spawn()?))
}
