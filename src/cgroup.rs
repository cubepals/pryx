// SPDX-License-Identifier: FSL-1.1-ALv2
//! The daemon's cgroup v2 location and a process sandbox's membership, fencing and release.
//! These operations need only a writable delegated hierarchy; BPF programs and their pins
//! belong to `bpf/`, and per-sandbox runtime policy belongs to `runtime/process/`.
use std::{
    fs, io,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Command,
};

/// Why a host can't give the guarantee, and what would.
#[derive(Debug)]
pub(crate) struct Unavailable {
    pub(crate) evidence: String,
    pub(crate) action: &'static str,
}

pub(crate) fn unavailable(evidence: impl Into<String>, action: &'static str) -> Unavailable {
    Unavailable { evidence: evidence.into(), action }
}

/// One mount: where, of what type, and which part of its filesystem it shows.
pub(crate) struct Mount {
    pub(crate) root: PathBuf,
    pub(crate) point: PathBuf,
    pub(crate) kind: String,
}

/// mountinfo escapes space, tab, newline and backslash as three octal digits.
fn unescape(field: &str) -> PathBuf {
    let mut bytes = Vec::with_capacity(field.len());
    let raw = field.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\\'
            && let Some(value) = field.get(i + 1..i + 4).and_then(|digits| u8::from_str_radix(digits, 8).ok())
        {
            bytes.push(value);
            i += 4;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes))
}

pub(crate) fn mounts(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let (before, after) = line.split_once(" - ")?;
            let fields: Vec<&str> = before.split(' ').collect();
            Some(Mount {
                root: unescape(fields.get(3)?),
                point: unescape(fields.get(4)?),
                kind: after.split(' ').next()?.to_owned(),
            })
        })
        .collect()
}

/// This process's cgroup v2 directory, from its cgroup path and where that hierarchy is mounted.
pub(crate) fn own_cgroup(cgroup: &str, mountinfo: &str) -> Result<PathBuf, Unavailable> {
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::")).ok_or_else(|| {
        unavailable("this process has no cgroup v2 membership", "Boot with the unified cgroup v2 hierarchy")
    })?;
    let path = Path::new(path.trim());
    mounts(mountinfo)
        .into_iter()
        .filter(|mount| mount.kind == "cgroup2")
        .find_map(|mount| {
            let below = path.strip_prefix(&mount.root).ok()?;
            Some(mount.point.join(below))
        })
        .ok_or_else(|| {
            unavailable(
                format!("cgroup {} is outside every mounted cgroup v2 hierarchy", path.display()),
                "Mount the cgroup v2 hierarchy this daemon's cgroup belongs to",
            )
        })
}

/// A directory this process may make entries in.
pub(crate) fn writable(path: &Path) -> bool {
    rustix::fs::access(path, rustix::fs::Access::WRITE_OK | rustix::fs::Access::EXEC_OK).is_ok()
}

/// A sandbox's cgroup, made if missing. A leftover one from a lost run is reused once empty.
pub(crate) fn create_cgroup(path: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o755).create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if members(path)?.is_empty() {
                Ok(())
            } else {
                Err(io::Error::other(format!("cgroup {} still holds processes", path.display())))
            }
        }
        Err(error) => Err(error),
    }
}

/// The processes in a cgroup now.
pub(crate) fn members(path: &Path) -> io::Result<Vec<u32>> {
    Ok(fs::read_to_string(path.join("cgroup.procs"))?.lines().filter_map(|pid| pid.parse().ok()).collect())
}

/// Stops every task and leaves the cgroup frozen, so a late join cannot run after the kill.
/// The programs remain attached until every member is gone.
pub(crate) fn fence_cgroup(path: &Path) -> io::Result<()> {
    let frozen = fs::write(path.join("cgroup.freeze"), b"1");
    let killed = fs::write(path.join("cgroup.kill"), b"1");
    frozen.and(killed)
}

/// Waits before detaching a stopped cgroup's programs. A task that hasn't exited keeps them.
pub(crate) fn wait_empty(path: &Path) -> io::Result<()> {
    const STOP: std::time::Duration = std::time::Duration::from_secs(5);
    let deadline = std::time::Instant::now() + STOP;
    while !members(path)?.is_empty() {
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::other(format!("cgroup {} still holds stopped processes", path.display())));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(())
}

/// Removes an empty cgroup; a missing one is already gone. Exiting processes can hold it for a
/// moment, so it is tried a few times.
pub(crate) fn remove_cgroup(path: &Path) -> io::Result<()> {
    const ATTEMPTS: u32 = 50;
    for _ in 0..ATTEMPTS {
        match fs::remove_dir(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) if error.raw_os_error() == Some(libc::EBUSY) => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            Err(error) => return Err(error),
        }
    }
    fs::remove_dir(path)
}

/// Makes `command`'s process join `cgroup` before it runs anything, so every socket it opens is
/// the cgroup's. The descriptor is opened here, so the kernel checks this daemon's right to move
/// it even after the child has become its sandbox's user.
pub(crate) fn join_on_exec(command: &mut Command, cgroup: &Path) -> io::Result<()> {
    use std::os::unix::process::CommandExt;
    let procs = fs::OpenOptions::new().write(true).open(cgroup.join("cgroup.procs"))?;
    // SAFETY: between fork and exec the closure makes one write(2) to a descriptor it owns, which
    // is async-signal-safe and touches no lock or allocation.
    #[expect(unsafe_code, reason = "joining a cgroup between fork and exec needs pre_exec")]
    unsafe {
        command.pre_exec(move || {
            rustix::io::write(&procs, b"0")?;
            Ok(())
        });
    }
    Ok(())
}

/// Keeps this daemon's ambient capabilities (CAP_BPF and CAP_NET_ADMIN where it was granted them
/// to attach programs) from passing to `command`, whose process then runs a sandbox's code.
pub(crate) fn drop_ambient_on_exec(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: between fork and exec the closure makes one prctl(2), which is async-signal-safe
    // and touches no lock or allocation.
    #[expect(unsafe_code, reason = "clearing the ambient set between fork and exec needs pre_exec")]
    unsafe {
        command.pre_exec(|| {
            rustix::thread::clear_ambient_capability_set()?;
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTINFO: &str = "\
25 30 0:22 / /sys rw,nosuid shared:7 - sysfs sysfs rw
29 25 0:26 / /sys/fs/cgroup rw,nosuid shared:9 - cgroup2 cgroup2 rw,nsdelegate
33 25 0:30 / /sys/fs/bpf rw,nosuid shared:12 - bpf bpf rw,mode=700
90 30 0:40 / /run/pryx\\040bpf rw shared:50 - bpf none rw
";

    #[test]
    fn the_own_cgroup_is_the_v2_path_below_its_mount() {
        assert_eq!(
            own_cgroup("0::/user.slice/user-1001.slice/user@1001.service/app.slice/pryxd.service\n", MOUNTINFO)
                .unwrap(),
            Path::new("/sys/fs/cgroup/user.slice/user-1001.slice/user@1001.service/app.slice/pryxd.service")
        );
        let namespaced = "40 30 0:26 /system.slice/docker-1.scope /sys/fs/cgroup ro - cgroup2 cgroup2 rw\n";
        assert_eq!(
            own_cgroup("0::/system.slice/docker-1.scope/x\n", namespaced).unwrap(),
            Path::new("/sys/fs/cgroup/x")
        );
        assert!(own_cgroup("1:name=systemd:/\n", MOUNTINFO).is_err());
        assert!(own_cgroup("0::/a\n", "25 30 0:22 / /sys rw - sysfs sysfs rw\n").is_err());
    }

    #[test]
    fn mount_points_are_unescaped() {
        let points: Vec<_> = mounts(MOUNTINFO).into_iter().filter(|m| m.kind == "bpf").map(|m| m.point).collect();
        assert_eq!(points, [PathBuf::from("/sys/fs/bpf"), PathBuf::from("/run/pryx bpf")]);
    }
}
