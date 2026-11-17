// SPDX-License-Identifier: FSL-1.1-ALv2
//! Read-only binding of one Docker-reported host PID to its pidfd and an owned cgroup v2
//! directory. An unreadable or deleted counter is an explicit diagnostic failure, never zero.
use super::oom_fields::{self as fields, Limit};
use rustix::fd::OwnedFd;
use rustix::fs::{CWD, Mode, OFlags, ResolveFlags, fstat, fstatfs, openat, openat2};
use rustix::process::{Pid, PidfdFlags, pidfd_open};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub(super) struct Failure {
    pub(super) stage: &'static str,
    pub(super) errno: Option<i32>,
}

impl Failure {
    pub(super) fn invalid(stage: &'static str) -> Self {
        Self { stage, errno: None }
    }
    pub(super) fn io(stage: &'static str, error: std::io::Error) -> Self {
        Self { stage, errno: error.raw_os_error() }
    }
}

pub(super) struct Process {
    pub(super) pid: i32,
    pub(super) ticks: u64,
    pidfd: OwnedFd,
    proc_dir: OwnedFd,
}

impl Process {
    pub(super) fn capture(pid: i32) -> Result<Self, Failure> {
        let value = Pid::from_raw(pid).filter(|_| pid > 1).ok_or(Failure::invalid("invalid_pid"))?;
        let proc_dir = directory(Path::new(&format!("/proc/{pid}")), "proc_directory")?;
        let before = read_at(&proc_dir, "stat", 16384, "proc_stat")?;
        let ticks = fields::start_ticks(&before, pid).ok_or(Failure::invalid("invalid_proc_stat"))?;
        let pidfd = pidfd_open(value, PidfdFlags::empty()).map_err(|e| Failure::io("pidfd_open", e.into()))?;
        let held = Self { pid, ticks, pidfd, proc_dir };
        held.confirm()?;
        Ok(held)
    }

    pub(super) fn alive(&self) -> Result<bool, Failure> {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        let mut descriptors = [PollFd::new(&self.pidfd, PollFlags::IN)];
        poll(&mut descriptors, Some(&Timespec { tv_sec: 0, tv_nsec: 0 }))
            .map_err(|e| Failure::io("pidfd_poll", e.into()))?;
        Ok(descriptors[0].revents().is_empty())
    }

    pub(super) fn confirm(&self) -> Result<(), Failure> {
        let stat = read_at(&self.proc_dir, "stat", 16384, "proc_stat_confirm")?;
        if fields::start_ticks(&stat, self.pid) != Some(self.ticks) || !self.alive()? {
            return Err(Failure::invalid("process_identity_changed"));
        }
        Ok(())
    }
}

#[derive(Clone, serde::Serialize)]
pub(super) struct GroupReport {
    pub(super) boot_id: String,
    pub(super) path_sha256: String,
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) cgroup2: bool,
    pub(super) mount_read_only: bool,
    pub(super) filesystem_read_only: bool,
    pub(super) memory_localevents: bool,
}

#[derive(Clone, serde::Serialize)]
pub(super) struct Sample {
    pub(super) observed_unix_ns: i128,
    pub(super) oom_kill: u64,
    pub(super) memory_max: Limit,
    pub(super) pidfd_alive: bool,
}

pub(super) struct Group {
    pub(super) process: Process,
    pub(super) report: GroupReport,
    directory: OwnedFd,
    path: PathBuf,
}

impl Group {
    pub(super) fn capture(process: Process, id: &str) -> Result<Self, Failure> {
        use sha2::{Digest, Sha256};
        let text = read_at(&process.proc_dir, "cgroup", 16384, "proc_cgroup")?;
        let path = fields::unified_group(&text, id).ok_or(Failure::invalid("unowned_unified_cgroup"))?;
        let mounts = read_file(&format!("/proc/{}/mountinfo", std::process::id()), 262144, "mountinfo")?;
        let mount = fields::group_mount(&mounts, &path).ok_or(Failure::invalid("unmapped_unified_cgroup"))?;
        let root = directory(&mount.point, "cgroup_mount")?;
        let directory = openat2(
            &root,
            &mount.relative,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|e| Failure::io("cgroup_directory", e.into()))?;
        let fs = fstatfs(&directory).map_err(|e| Failure::io("cgroup_fstatfs", e.into()))?;
        if fs.f_type != libc::CGROUP2_SUPER_MAGIC {
            return Err(Failure::invalid("not_cgroup2"));
        }
        let stat = fstat(&directory).map_err(|e| Failure::io("cgroup_fstat", e.into()))?;
        let boot = read_file("/proc/sys/kernel/random/boot_id", 64, "boot_id")?;
        let boot = uuid::Uuid::parse_str(boot.trim()).map_err(|_| Failure::invalid("invalid_boot_id"))?;
        let report = GroupReport {
            boot_id: boot.to_string(),
            path_sha256: hex::encode(Sha256::digest(path.as_os_str().as_encoded_bytes())),
            device: stat.st_dev,
            inode: stat.st_ino,
            cgroup2: true,
            mount_read_only: mount.read_only,
            filesystem_read_only: (fs.f_flags as u64 & libc::ST_RDONLY) != 0,
            memory_localevents: mount.local_events,
        };
        let held = Self { process, report, directory, path };
        held.confirm()?;
        Ok(held)
    }

    pub(super) fn confirm(&self) -> Result<(), Failure> {
        self.process.confirm()?;
        let text = read_at(&self.process.proc_dir, "cgroup", 16384, "proc_cgroup_confirm")?;
        let current = text.lines().find_map(|line| line.strip_prefix("0::")).map(Path::new);
        if current != Some(self.path.as_path()) {
            return Err(Failure::invalid("process_cgroup_changed"));
        }
        Ok(())
    }

    pub(super) fn sample(&self) -> Result<Sample, Failure> {
        let alive = self.process.alive()?;
        if alive {
            self.confirm()?;
        }
        let events = read_at(&self.directory, "memory.events", 4096, "memory_events")?;
        let oom_kill = fields::oom_kills(&events).ok_or(Failure::invalid("invalid_memory_events"))?;
        let limit = read_at(&self.directory, "memory.max", 64, "memory_max")?;
        let memory_max = fields::memory_limit(&limit).ok_or(Failure::invalid("invalid_memory_max"))?;
        Ok(Sample {
            observed_unix_ns: time::OffsetDateTime::now_utc().unix_timestamp_nanos(),
            oom_kill,
            memory_max,
            pidfd_alive: alive,
        })
    }
}

fn directory(path: &Path, stage: &'static str) -> Result<OwnedFd, Failure> {
    openat2(
        CWD,
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|e| Failure::io(stage, e.into()))
}

fn read_file(path: &str, limit: u64, stage: &'static str) -> Result<String, Failure> {
    let fd = openat2(
        CWD,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|e| Failure::io(stage, e.into()))?;
    read(fd, limit, stage)
}

fn read_at(directory: &OwnedFd, name: &str, limit: u64, stage: &'static str) -> Result<String, Failure> {
    let fd = openat(directory, name, OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW, Mode::empty())
        .map_err(|e| Failure::io(stage, e.into()))?;
    read(fd, limit, stage)
}

fn read(fd: OwnedFd, limit: u64, stage: &'static str) -> Result<String, Failure> {
    let mut value = String::new();
    File::from(fd).take(limit + 1).read_to_string(&mut value).map_err(|e| Failure::io(stage, e))?;
    if value.len() as u64 > limit {
        return Err(Failure::invalid("diagnostic_field_too_large"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_child_pidfd_is_alive_then_terminal() {
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let captured = Process::capture(child.id() as i32);
        child.kill().unwrap();
        child.wait().unwrap();
        let held = captured.unwrap_or_else(|failure| panic!("{} {:?}", failure.stage, failure.errno));
        assert!(held.ticks > 0);
        assert!(!held.alive().unwrap_or_else(|failure| panic!("{} {:?}", failure.stage, failure.errno)));
        assert!(held.confirm().is_err(), "a terminal pidfd cannot be rebound to another start");
    }
}
