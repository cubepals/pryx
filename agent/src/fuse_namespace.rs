// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-approved FUSE runs in a nested user/mount namespace, preserving the sandbox uid.
//! Only managed mount workers retain its mount capability; ordinary children irreversibly drop it.
use nix::{
    sched::{CloneFlags, unshare},
    unistd::{getgid, getuid},
};
use rustix::thread::{CapabilitySet, CapabilitySets};
use std::{fs, io, io::Write};

static EXECUTABLE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// A bind-mounted executable's procfs link can resolve to the mount root after cloning its
/// mount namespace. Keep the kernel-resolved path from before that transition.
pub fn executable() -> io::Result<std::path::PathBuf> {
    EXECUTABLE.get().cloned().map(Ok).unwrap_or_else(std::env::current_exe)
}

pub struct Mappings {
    executable: std::path::PathBuf,
    groups: fs::File,
    uid: fs::File,
    gid: fs::File,
}

/// Pin only our own kernel mapping files before the initial non-dumpable transition.
/// Their descriptors remain private and close on exec; dumpability is never re-enabled.
pub fn prepare() -> io::Result<Option<Mappings>> {
    if !enabled() {
        return Ok(None);
    }
    let open = |name| {
        rustix::fs::open(name, rustix::fs::OFlags::PATH | rustix::fs::OFlags::CLOEXEC, rustix::fs::Mode::empty())
            .map(fs::File::from)
            .map_err(|e| io::Error::other(format!("Pin private native namespace mapping {name}: {e}")))
    };
    Ok(Some(Mappings {
        executable: std::env::current_exe()?,
        groups: open("/proc/self/setgroups")?,
        uid: open("/proc/self/uid_map")?,
        gid: open("/proc/self/gid_map")?,
    }))
}

pub fn enabled() -> bool {
    std::env::var_os(crate::contract::FUSE_ENV).as_deref() == Some(std::ffi::OsStr::new("1"))
}

pub fn enter(mappings: Option<Mappings>) -> io::Result<()> {
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
    let mappings = mappings.ok_or_else(|| io::Error::other("Missing private native namespace mappings"))?;
    EXECUTABLE
        .set(mappings.executable.clone())
        .map_err(|_| io::Error::other("Native namespace launcher was already initialized"))?;
    unshare(CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS)
        .map_err(|e| io::Error::other(format!("Create native mount namespace: {e}")))?;
    use std::os::fd::AsRawFd;
    let reopen =
        |file: &fs::File| fs::OpenOptions::new().write(true).open(format!("/proc/self/fd/{}", file.as_raw_fd()));
    reopen(&mappings.groups)?
        .write_all(b"deny")
        .map_err(|e| io::Error::other(format!("Deny native namespace setgroups: {e}")))?;
    reopen(&mappings.uid)?
        .write_all(format!("{uid} {uid} 1").as_bytes())
        .map_err(|e| io::Error::other(format!("Map native namespace uid: {e}")))?;
    reopen(&mappings.gid)?
        .write_all(format!("{gid} {gid} 1").as_bytes())
        .map_err(|e| io::Error::other(format!("Map native namespace gid: {e}")))?;
    rustix::mount::mount_change(
        "/",
        rustix::mount::MountPropagationFlags::PRIVATE | rustix::mount::MountPropagationFlags::REC,
    )
    .map_err(|e| io::Error::other(format!("Make native mount namespace private: {e}")))?;
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
    use std::os::unix::fs::MetadataExt;
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
    // FUSE helpers may close inherited descriptors or refuse another process's procfs links.
    // The working directory pins the verified target across those helper execs as well.
    rustix::process::fchdir(&file)?;
    argv[at] = ".".into();
    Ok(file)
}
