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

/// A sealed, execute-only copy of the agent binary, for the exec of a mount worker.
///
/// The kernel leaves a process dumpable after exec only when the caller may read the executed
/// file. The agent's own file stays readable, since the fleet hands the same file out, so a worker
/// exec'd from it would be dumpable until its `main` turned that off, and in that window a same-uid
/// sandbox command could read its `/proc` environment, which holds the bucket credentials, or
/// trace it. This copy lives in memory that only the agent holds, is mode 0111 so the worker is
/// never dumpable from exec onward, and is sealed so nothing can alter it after the copy. The
/// caller execs it as `/proc/self/fd/<fd>` and drops it once the spawn returns.
pub fn sealed_executable() -> io::Result<std::os::fd::OwnedFd> {
    use rustix::fs::{MemfdFlags, Mode, SealFlags};
    let mut source = fs::File::open(executable()?)?;
    let flags = MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING;
    // Kernels before 6.3 refuse MFD_EXEC with EINVAL; their memfds are executable by default.
    let memfd = match rustix::fs::memfd_create("pryx-agent", flags | MemfdFlags::EXEC) {
        Err(rustix::io::Errno::INVAL) => rustix::fs::memfd_create("pryx-agent", flags),
        other => other,
    }
    .map_err(|e| io::Error::other(format!("Create sealed agent copy: {e}")))?;
    let mut copy = fs::File::from(memfd);
    io::copy(&mut source, &mut copy)?;
    rustix::fs::fchmod(&copy, Mode::XUSR | Mode::XGRP | Mode::XOTH)?;
    rustix::fs::fcntl_add_seals(&copy, SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE | SealFlags::SEAL)?;
    Ok(copy.into())
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

/// The exit status of a mount worker that refused a readable helper: sysexits.h's EX_CONFIG.
pub const READABLE_HELPER: i32 = 78;

/// Resolves a mount helper the way exec's PATH search would, to an executable regular file.
/// Relative PATH entries are skipped: a worker's working directory is the mount target.
pub fn helper(name: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let runnable =
        |path: &std::path::Path| fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if name.contains('/') {
        return Some(std::path::PathBuf::from(name)).filter(|p| runnable(p));
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).filter(|dir| dir.is_absolute()).map(|dir| dir.join(name)).find(|p| runnable(p))
}

/// The kernel leaves a process dumpable after exec only when the caller may read the file, and a
/// dumpable helper's /proc/<pid>/environ, which holds its bucket credentials, is open to every
/// same-uid sandbox command. An execute-only helper is not dumpable, so its environment is not.
pub fn readable(path: &std::path::Path) -> bool {
    rustix::fs::access(path, rustix::fs::Access::READ_OK).is_ok()
}

pub fn enabled() -> bool {
    std::env::var_os(crate::contract::FUSE_ENV).as_deref() == Some(std::ffi::OsStr::new("1"))
        && (std::env::var_os(crate::contract::TOOL_KIND_ENV).is_some()
            || std::env::var_os(crate::contract::MOUNTS_ENV).is_some())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;

    fn path(fd: &std::os::fd::OwnedFd) -> String {
        format!("/proc/self/fd/{}", fd.as_raw_fd())
    }

    #[test]
    fn sealed_executable_is_execute_only_and_sealed() {
        let fd = sealed_executable().unwrap();
        let mode = rustix::fs::fstat(&fd).unwrap().st_mode & 0o7777;
        assert_eq!(mode, 0o111);
        if getuid().is_root() {
            eprintln!("note: running as root, which may read any file; the readable() check needs a non-root user");
        } else {
            assert!(!readable(std::path::Path::new(&path(&fd))));
        }
        assert_eq!(rustix::io::write(&fd, b"x"), Err(rustix::io::Errno::PERM));
        assert_eq!(rustix::fs::ftruncate(&fd, 0), Err(rustix::io::Errno::PERM));
        let status =
            std::process::Command::new(path(&fd)).arg("--list").stdout(std::process::Stdio::null()).status().unwrap();
        assert!(status.success());
    }

    /// Runs only when the probe below execs this harness, to keep a child alive while it looks.
    #[test]
    #[ignore = "a long-running child for sealed_executable_child_is_not_dumpable"]
    fn sleeps_for_sealed_probe() {
        std::thread::sleep(std::time::Duration::from_secs(2));
    }

    #[test]
    fn sealed_executable_child_is_not_dumpable() {
        let spawn = |program: &str| {
            std::process::Command::new(program)
                .args(["--ignored", "--exact", "fuse_namespace::tests::sleeps_for_sealed_probe"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap()
        };
        let owner = |child: &std::process::Child| fs::metadata(format!("/proc/{}/status", child.id())).unwrap().uid();
        let fd = sealed_executable().unwrap();
        let mut sealed = spawn(&path(&fd));
        drop(fd);
        let mut readable = spawn(executable().unwrap().to_str().unwrap());
        let (sealed_owner, readable_owner) = (owner(&sealed), owner(&readable));
        for child in [&mut sealed, &mut readable] {
            let _ = child.kill();
            let _ = child.wait();
        }
        if getuid().is_root() {
            eprintln!("note: running as root; procfs ownership cannot tell a non-dumpable child apart");
            return;
        }
        // procfs gives a non-dumpable task's entries to root, and a dumpable one's to its owner.
        assert_eq!(sealed_owner, 0);
        assert_eq!(readable_owner, getuid().as_raw());
    }
}
