// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-approved FUSE runs in a nested user/mount namespace, preserving the sandbox uid.
//! Only managed mount workers retain its mount capability; ordinary children irreversibly drop it.
use nix::{
    sched::{CloneFlags, unshare},
    unistd::{getgid, getuid},
};
use rustix::thread::{CapabilitySet, CapabilitySets};
use std::{fs, io};

pub fn enabled() -> bool {
    std::env::var_os(crate::contract::FUSE_ENV).as_deref() == Some(std::ffi::OsStr::new("1"))
}

pub fn enter() -> io::Result<()> {
    if !enabled() {
        return Ok(());
    }
    if std::env::var_os(crate::process_policy::POLICY_ENV).is_some() {
        return Err(io::Error::other("FUSE is unsupported on the process tier"));
    }
    use std::os::unix::fs::FileTypeExt;
    if !fs::symlink_metadata("/dev/fuse")?.file_type().is_char_device() {
        return Err(io::Error::other("Host-approved FUSE device is unavailable"));
    }
    if getuid().is_root() {
        return Err(io::Error::other("Native FUSE requires a non-root sandbox user"));
    }
    let uid = getuid().as_raw();
    let gid = getgid().as_raw();
    unshare(CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS).map_err(io::Error::from)?;
    fs::write("/proc/self/setgroups", "deny")?;
    fs::write("/proc/self/uid_map", format!("{uid} {uid} 1"))?;
    fs::write("/proc/self/gid_map", format!("{gid} {gid} 1"))?;
    rustix::mount::mount_change(
        "/",
        rustix::mount::MountPropagationFlags::PRIVATE | rustix::mount::MountPropagationFlags::REC,
    )?;
    // This capability belongs only to the new user namespace. The Docker namespace keeps ALL dropped.
    rustix::thread::set_capabilities(
        None,
        CapabilitySets {
            effective: CapabilitySet::SYS_ADMIN,
            permitted: CapabilitySet::SYS_ADMIN,
            inheritable: CapabilitySet::SYS_ADMIN,
        },
    )?;
    rustix::thread::configure_capability_in_ambient_set(CapabilitySet::SYS_ADMIN, true)?;
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    Ok(())
}

pub fn child(keep_mount: bool) -> io::Result<()> {
    rustix::thread::clear_ambient_capability_set()?;
    let caps = if keep_mount { CapabilitySet::SYS_ADMIN } else { CapabilitySet::empty() };
    rustix::thread::set_capabilities(None, CapabilitySets { effective: caps, permitted: caps, inheritable: caps })?;
    if keep_mount {
        rustix::thread::configure_capability_in_ambient_set(CapabilitySet::SYS_ADMIN, true)?;
    }
    Ok(())
}

/// Workers share only the approved mountpoint with the agent. A worker cannot replace the
/// agent's executable/procfs or join its mount namespace, even if its image is hostile.
pub fn worker(argv: &mut [String]) -> io::Result<std::fs::File> {
    use seccompiler::{BpfProgram, SeccompAction, SeccompFilter};
    use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
    unshare(CloneFlags::CLONE_NEWNS).map_err(io::Error::from)?;
    let filter = SeccompFilter::new(
        std::collections::BTreeMap::from([(libc::SYS_setns, Vec::new())]),
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        std::env::consts::ARCH.try_into().map_err(io::Error::other)?,
    )
    .map_err(io::Error::other)?;
    let program: BpfProgram = filter.try_into().map_err(io::Error::other)?;
    seccompiler::apply_filter(&program).map_err(io::Error::other)?;
    let at = argv
        .iter()
        .position(|arg| {
            arg.strip_prefix("/proc/self/fd/")
                .is_some_and(|fd| !fd.is_empty() && fd.bytes().all(|b| b.is_ascii_digit()))
        })
        .ok_or_else(|| io::Error::other("Missing pinned mount target"))?;
    let original = fs::metadata(&argv[at])?;
    let path = fs::read_link(&argv[at])?;
    let relative = path
        .to_str()
        .and_then(|p| p.strip_prefix('/'))
        .ok_or_else(|| io::Error::other("Invalid pinned mount target"))?;
    let file = crate::paths::DataDir::open(std::path::Path::new("/"))?.directory_handle(relative, false)?;
    let child = file.metadata()?;
    if original.dev() != child.dev() || original.ino() != child.ino() {
        return Err(io::Error::other("Mount target changed before worker isolation"));
    }
    rustix::io::fcntl_setfd(&file, rustix::io::FdFlags::empty())?;
    argv[at] = format!("/proc/self/fd/{}", file.as_raw_fd());
    Ok(file)
}
