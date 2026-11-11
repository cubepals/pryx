// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Kernel-owned facts, with fixture roots for cgroup and device observations. Capability use
//! runs only inside the private child, never inside the multithreaded daemon.
use super::{Capability, Report, State};
use crate::capability::c_library;
use std::{
    fs,
    path::{Component, Path},
};

pub(super) fn read(root: &Path) -> Report {
    let mut report =
        Report { architecture: std::env::consts::ARCH.into(), c_library: c_library(), ..Report::default() };
    report.capabilities.insert(
        "process-tool-runtime".into(),
        crate::capability::process_tools::prerequisite(&report.architecture, &report.c_library),
    );
    let (abi, capability) = landlock();
    report.landlock_abi = abi;
    report.capabilities.insert("landlock".into(), capability);
    for device in ["kvm", "fuse"] {
        let path = root.join("dev").join(device);
        let capability = match fs::OpenOptions::new().read(true).write(true).open(&path) {
            Ok(_) => Capability::on(format!("{} opened read/write", path.display())),
            Err(error) => Capability::error(error, &format!("Grant this user access to /dev/{device}")),
        };
        report.capabilities.insert(format!("{device}-device"), capability);
    }
    report.capabilities.insert("cgroup-controllers".into(), cgroup(root));
    report
}

fn cgroup(root: &Path) -> Capability {
    let result = (|| -> std::io::Result<String> {
        let membership = fs::read_to_string(root.join("proc/self/cgroup"))?;
        let relative = membership
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .ok_or_else(|| std::io::Error::other("No unified cgroup membership"))?;
        if Path::new(relative).components().any(|c| c == Component::ParentDir) {
            return Err(std::io::Error::other("Cgroup path is outside this mount namespace"));
        }
        let directory = root.join("sys/fs/cgroup").join(relative.trim_start_matches('/'));
        let controllers = fs::read_to_string(directory.join("cgroup.controllers"))?;
        Ok(format!("{}: {}", directory.display(), controllers.trim()))
    })();
    match result {
        Ok(evidence) => Capability::on(evidence),
        Err(error) => {
            Capability::new(State::Unknown, error.to_string(), "Delegate cgroup v2 controllers to this user's service")
        }
    }
}

fn landlock() -> (Option<u32>, Capability) {
    // Linux UAPI linux/landlock.h: LANDLOCK_CREATE_RULESET_VERSION requests the ABI only.
    const VERSION: libc::c_uint = 1;
    #[expect(unsafe_code, reason = "Read-only Landlock ABI query; libc has no typed wrapper")]
    // SAFETY: null attr and size zero are the documented version-query inputs.
    let abi = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, std::ptr::null::<u8>(), 0, VERSION) };
    if abi >= 0 {
        let abi = abi as u32;
        return (Some(abi), Capability::on(format!("Kernel returned ABI {abi}")));
    }
    (None, Capability::error(std::io::Error::last_os_error(), "Enable Landlock in the kernel and permit its syscalls"))
}

#[expect(unsafe_code, reason = "Credential use is tested only in a disposable subprocess")]
pub(super) fn separate_users_child() -> anyhow::Result<()> {
    // The process runtime's default pool is host policy, not a sandbox request.
    let uid = crate::config::ProcessConfig::default().uid_first;
    // SAFETY: all calls change only this child, which exits immediately; no pointer is retained.
    anyhow::ensure!(unsafe { libc::setgroups(0, std::ptr::null()) } == 0, "{}", std::io::Error::last_os_error());
    // SAFETY: dropping this child's gid cannot affect the parent.
    anyhow::ensure!(unsafe { libc::setresgid(uid, uid, uid) } == 0, "{}", std::io::Error::last_os_error());
    // SAFETY: dropping this child's uid cannot affect the parent.
    anyhow::ensure!(unsafe { libc::setresuid(uid, uid, uid) } == 0, "{}", std::io::Error::last_os_error());
    println!("Used SETUID and SETGID to enter the process uid pool");
    Ok(())
}

#[expect(unsafe_code, reason = "Namespace syscalls are confined to a disposable subprocess")]
pub(super) fn user_namespace_child() -> anyhow::Result<()> {
    let uid = rustix::process::geteuid().as_raw();
    let gid = rustix::process::getegid().as_raw();
    // SAFETY: unshare affects only this single-threaded disposable child.
    anyhow::ensure!(unsafe { libc::unshare(libc::CLONE_NEWUSER) } == 0, "{}", std::io::Error::last_os_error());
    match fs::write("/proc/self/setgroups", "deny") {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    fs::write("/proc/self/uid_map", format!("0 {uid} 1\n"))?;
    fs::write("/proc/self/gid_map", format!("0 {gid} 1\n"))?;
    // SAFETY: a private UTS namespace keeps the host's hostname unchanged.
    anyhow::ensure!(unsafe { libc::unshare(libc::CLONE_NEWUTS) } == 0, "{}", std::io::Error::last_os_error());
    let hostname = b"pryxd-probe";
    anyhow::ensure!(
        // SAFETY: hostname points to exactly len initialized bytes; only this child's UTS changes.
        unsafe { libc::sethostname(hostname.as_ptr().cast(), hostname.len()) } == 0,
        "CAP_SYS_ADMIN in user namespace: {}",
        std::io::Error::last_os_error()
    );
    println!("Created a user namespace and used CAP_SYS_ADMIN in its private UTS namespace");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controllers_are_read_from_the_callers_cgroup_and_cannot_escape() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("proc/self")).unwrap();
        fs::create_dir_all(root.path().join("sys/fs/cgroup/user.slice")).unwrap();
        fs::write(root.path().join("proc/self/cgroup"), "0::/user.slice\n").unwrap();
        fs::write(root.path().join("sys/fs/cgroup/user.slice/cgroup.controllers"), "memory pids\n").unwrap();
        let result = cgroup(root.path());
        assert_eq!(result.state, State::On);
        assert!(result.evidence.ends_with("memory pids"));
        fs::write(root.path().join("proc/self/cgroup"), "0::/../../outside\n").unwrap();
        assert_eq!(cgroup(root.path()).state, State::Unknown);
    }
    #[test]
    fn permission_is_not_absence() {
        assert_eq!(
            Capability::error(std::io::Error::from(std::io::ErrorKind::PermissionDenied), "fix").state,
            State::Unknown
        );
        assert_eq!(Capability::error(std::io::Error::from(std::io::ErrorKind::NotFound), "fix").state, State::Off);
    }
}
