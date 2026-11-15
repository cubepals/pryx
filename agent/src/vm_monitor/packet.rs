// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Validate inherited packet listeners or root-created TAP queues with exact filesystem grants.
//! The maintained helper owns networking; this module never implements a packet transport.
use super::{Plan, TUN_DEVICE};
use std::{
    fs,
    os::{
        fd::OwnedFd,
        unix::{fs::MetadataExt, net::UnixListener},
    },
    path::{Path, PathBuf},
};

pub(super) fn listener(
    plan: &Plan,
    socket: &Path,
    descriptor_argument: &str,
) -> Result<UnixListener, Box<dyn std::error::Error>> {
    if socket.parent() != Some(plan.directory.as_path())
        || !plan.write.is_empty()
        || descriptor_argument
            .strip_prefix("--")
            .is_none_or(|name| name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-'))
        || plan.arguments.iter().any(|arg| arg.split('=').next() == Some(descriptor_argument))
    {
        return Err("Packet proxy requires its own private listener and inherited descriptor".into());
    }
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    rustix::io::fcntl_setfd(&listener, rustix::io::FdFlags::empty())?;
    super::confine(plan, false)?;
    Ok(listener)
}

// Go's net resolver reads these host configuration files. Grant only their resolved,
// protected regular files; no workload path or directory-wide /etc grant is accepted.
const RESOLVER_FILES: &[&str] = &["/etc/resolv.conf", "/etc/hosts", "/etc/nsswitch.conf"];

pub(super) fn resolver_files() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    for configured in RESOLVER_FILES {
        match fs::canonicalize(configured) {
            Ok(path) => {
                super::regular(&path).map_err(|reason| format!("Host resolver configuration {path:?}: {reason}"))?;
                files.push(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(files)
}

pub(super) fn tap(plan: &Plan, descriptor_argument: &str) -> Result<OwnedFd, Box<dyn std::error::Error>> {
    // Stdio transfers the queue without unsafe post-fork code or reopening a TUN file
    // description. Consume the stderr carrier and restore ordinary diagnostics to null.
    let queue = rustix::io::fcntl_dupfd_cloexec(rustix::stdio::stderr(), libc::STDERR_FILENO + 1)?;
    rustix::stdio::dup2_stderr(fs::OpenOptions::new().write(true).open("/dev/null")?)?;
    let actual = rustix::fs::fstat(&queue)?;
    if actual.st_mode & libc::S_IFMT != libc::S_IFCHR
        || actual.st_rdev != fs::metadata(TUN_DEVICE)?.rdev()
        || !plan.write.is_empty()
        || descriptor_argument
            .strip_prefix("--")
            .is_none_or(|name| name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-'))
        || plan.arguments.iter().any(|arg| arg.split('=').next() == Some(descriptor_argument))
    {
        return Err("Packet TAP requires an inherited kernel TUN queue and private read-only configuration".into());
    }
    // The actual maintained helper verifies TUNGETIFF name/format before reading any frame.
    super::confine(plan, false)?;
    rustix::io::fcntl_setfd(&queue, rustix::io::FdFlags::empty())?;
    Ok(queue)
}
