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

// Go's net resolver reads these fixed host configuration files. A protected root-owned
// symlink can delegate its generated file to a host service such as systemd-resolved.
// No workload path or directory-wide /etc or /run grant is accepted.
const RESOLVER_FILES: &[&str] = &["/etc/resolv.conf", "/etc/hosts", "/etc/nsswitch.conf"];

fn host_configuration(configured: &Path, authority: u32) -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind};
    let invalid = || Error::new(ErrorKind::PermissionDenied, "Host resolver delegation is not protected");
    super::ancestors(configured.parent().ok_or_else(invalid)?)?;
    let selected = fs::symlink_metadata(configured)?;
    if selected.uid() != authority || selected.nlink() != 1 {
        return Err(invalid());
    }
    if !selected.file_type().is_symlink() {
        super::regular(configured)?;
        return Ok(configured.into());
    }
    let path = fs::canonicalize(configured)?;
    let target = fs::symlink_metadata(&path)?;
    if !target.is_file() || target.mode() & 0o022 != 0 || target.nlink() != 1 {
        return Err(invalid());
    }
    for ancestor in path.parent().ok_or_else(invalid)?.ancestors() {
        let info = fs::symlink_metadata(ancestor)?;
        let root = info.uid() == rustix::process::Uid::ROOT.as_raw();
        let sticky = root && info.mode() & libc::S_ISVTX != 0;
        if !info.is_dir() || (!sticky && info.mode() & 0o022 != 0) || (!root && info.uid() != target.uid()) {
            return Err(invalid());
        }
    }
    Ok(path)
}

pub(super) fn resolver_files() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    for configured in RESOLVER_FILES {
        match host_configuration(Path::new(configured), rustix::process::Uid::ROOT.as_raw()) {
            Ok(path) => files.push(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Host resolver configuration {configured:?}: {error}").into()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn configured_links_grant_only_protected_single_files() {
        let root = tempfile::tempdir().unwrap();
        let generated = root.path().join("generated");
        let configured = root.path().join("resolv.conf");
        let owner = rustix::process::geteuid().as_raw();
        let contents = uuid::Uuid::new_v4();
        fs::write(&generated, contents.as_bytes()).unwrap();
        symlink(&generated, &configured).unwrap();
        assert_eq!(host_configuration(&configured, owner).unwrap(), generated);
        assert_eq!(fs::read(&configured).unwrap(), contents.as_bytes());
        assert!(host_configuration(&configured, owner.checked_add(1).unwrap()).is_err());
        fs::set_permissions(&generated, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(host_configuration(&configured, owner).is_err());
        fs::set_permissions(&generated, fs::Permissions::from_mode(0o644)).unwrap();
        fs::hard_link(&generated, root.path().join("alias")).unwrap();
        assert!(host_configuration(&configured, owner).is_err());
    }
}
