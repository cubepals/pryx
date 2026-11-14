// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Closed host-side VM process launch with a lifetime kernel lease and host-owned confinement.
//! The workload never selects these paths or arguments. Ordinary VMMs and packet proxies inherit
//! separate Landlock/seccomp grants; root executes the unchanged jailer for the VMM.
//!
//! Parts (`vm_monitor/`):
//! - `filter.rs`: preserve the inherited kernel lease and forbid cross-process host operations.
mod filter;
use landlock::{
    ABI, Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
    path_beneath_rules,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt},
            net::UnixListener,
            process::CommandExt,
        },
    },
    path::{Component, Path, PathBuf},
};

pub const MODE: &str = "vm-monitor";
pub const VERSION: u16 = 1;
const MAX_PLAN_BYTES: u64 = 128 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub directory: PathBuf,
    pub lock: PathBuf,
    pub parent: u32,
    pub read: Vec<PathBuf>,
    pub write: Vec<PathBuf>,
    pub role: Role,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum Role {
    Ordinary,
    Jailer { uid: u32, gid: u32, id: String, base: PathBuf, monitor: PathBuf },
    PacketProxy { socket: PathBuf, descriptor_argument: String },
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn ancestors(path: &Path) -> io::Result<()> {
    if !path.is_absolute() || path.components().any(|part| matches!(part, Component::ParentDir)) {
        return Err(invalid("Monitor paths must be absolute"));
    }
    for ancestor in path.ancestors() {
        let meta = fs::symlink_metadata(ancestor)?;
        let sticky = meta.uid() == 0 && meta.mode() & libc::S_ISVTX != 0;
        if !meta.is_dir()
            || (!sticky && meta.mode() & 0o022 != 0)
            || (meta.uid() != 0 && meta.uid() != rustix::process::geteuid().as_raw())
        {
            return Err(invalid(&format!("Unsafe monitor path ancestor: {}", ancestor.display())));
        }
    }
    Ok(())
}

fn private(path: &Path) -> io::Result<()> {
    ancestors(path)?;
    let meta = fs::symlink_metadata(path)?;
    if meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o077 != 0 {
        return Err(invalid("Monitor state must be a private owned directory"));
    }
    Ok(())
}

fn regular(path: &Path) -> io::Result<()> {
    ancestors(path.parent().ok_or_else(|| invalid("Missing monitor path parent"))?)?;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file()
        || meta.mode() & 0o022 != 0
        || (meta.uid() != 0 && meta.uid() != rustix::process::geteuid().as_raw())
    {
        return Err(invalid("Monitor input must be an owned regular file"));
    }
    Ok(())
}

fn confine(plan: &Plan, kvm: bool) -> Result<(), Box<dyn std::error::Error>> {
    for path in plan.read.iter().chain(&plan.write) {
        if fs::symlink_metadata(path)?.is_dir() {
            ancestors(path)?;
        } else {
            regular(path)?;
        }
    }
    let mut read = plan.read.clone();
    read.push(plan.program.clone());
    read.extend(
        [
            "/proc/cpuinfo",
            "/proc/meminfo",
            "/proc/sys/kernel/osrelease",
            "/sys/devices/system/cpu",
            "/dev/urandom",
            "/dev/zero",
        ]
        .map(PathBuf::from),
    );
    read.push(PathBuf::from(format!("/proc/{}", std::process::id())));
    read.retain(|path| path.exists());
    let mut write = plan.write.clone();
    write.extend([plan.directory.clone(), PathBuf::from("/dev/null")]);
    if kvm {
        write.push(PathBuf::from("/dev/kvm"));
    }
    write.retain(|path| path.exists());
    let status = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(ABI::V3))?
        .create()?
        .add_rules(path_beneath_rules(read, AccessFs::from_read(ABI::V3)))?
        .add_rules(path_beneath_rules(write, AccessFs::from_all(ABI::V3)))?
        .restrict_self()?;
    if status.ruleset != RulesetStatus::FullyEnforced {
        return Err("VM monitor filesystem confinement is unavailable".into());
    }
    crate::process_policy::monitor()?;
    Ok(())
}

pub fn launch() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::io::stdin().take(MAX_PLAN_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PLAN_BYTES {
        return Err("Monitor launch plan exceeds its bound".into());
    }
    let plan: Plan = serde_json::from_slice(&bytes)?;
    let parent = rustix::process::Pid::from_raw(plan.parent.try_into()?);
    if parent.is_none() || rustix::process::getppid() != parent {
        return Err("Monitor launch parent changed".into());
    }
    private(&plan.directory)?;
    private(plan.lock.parent().ok_or("Missing monitor lease directory")?)?;
    regular(&plan.program)?;
    if std::iter::once(&plan.directory).chain(&plan.write).any(|path| plan.lock.starts_with(path)) {
        return Err("Monitor lease must be outside every writable grant".into());
    }
    // Firecracker 1.18's CLI (src/firecracker/src/main.rs) exports no flag constants.
    // A workload cannot disable the pinned monitor's own filter or replace it.
    if plan.arguments.iter().any(|arg| matches!(arg.split('=').next(), Some("--no-seccomp" | "--seccomp-filter"))) {
        return Err("Monitor syscall policy cannot be replaced".into());
    }
    let jailer = matches!(plan.role, Role::Jailer { .. });
    let packet_proxy = matches!(plan.role, Role::PacketProxy { .. });
    if jailer != rustix::process::geteuid().is_root() {
        return Err("Root VM monitors require the jailer".into());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&plan.lock)?;
    let meta = lock.metadata()?;
    if !meta.is_file() || meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o077 != 0 {
        return Err("Monitor lease must be a private owned regular file".into());
    }
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)?;
    rustix::stdio::dup2_stdin(&lock)?;
    // The jailer deliberately preserves only stdio. The lease therefore lives on
    // stdin, outside every writable grant/chroot; its bytes are never configuration.
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL))?;
    if rustix::process::getppid() != parent {
        return Err("Monitor launch parent changed".into());
    }
    let mut command = std::process::Command::new(&plan.program);
    command.env_clear().current_dir(&plan.directory);
    let mut packet_listener = None;
    match &plan.role {
        Role::Ordinary => confine(&plan, true)?,
        Role::PacketProxy { socket, descriptor_argument } => {
            if socket.parent() != Some(plan.directory.as_path())
                || !plan.write.is_empty()
                || descriptor_argument.strip_prefix("--").is_none_or(|name| {
                    name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-')
                })
                || plan.arguments.iter().any(|arg| arg.split('=').next() == Some(descriptor_argument.as_str()))
            {
                return Err("Packet proxy requires its own private listener and inherited descriptor".into());
            }
            let listener = UnixListener::bind(socket)?;
            fs::set_permissions(socket, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
            rustix::io::fcntl_setfd(&listener, rustix::io::FdFlags::empty())?;
            confine(&plan, false)?;
            packet_listener = Some(listener);
        }
        Role::Jailer { uid, gid, id, base, monitor } => {
            if *uid == 0
                || *gid == 0
                || *uid == u32::MAX
                || *gid == u32::MAX
                || id.is_empty()
                || !id.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
                || plan.program.file_name().is_none_or(|name| name != "jailer")
            {
                return Err("Jailer requires fixed non-root identities and a valid private ID".into());
            }
            private(base)?;
            regular(monitor)?;
            // The unchanged jailer CLI owns these documented flags, with no exported constants.
            command.args([
                "--id",
                id,
                "--uid",
                &uid.to_string(),
                "--gid",
                &gid.to_string(),
                "--exec-file",
                monitor.to_str().ok_or("Invalid monitor path")?,
                "--chroot-base-dir",
                base.to_str().ok_or("Invalid jailer base")?,
                "--",
            ]);
        }
    }
    filter::enter(jailer, packet_proxy)?;
    command.args(&plan.arguments);
    if let (Some(listener), Role::PacketProxy { descriptor_argument, .. }) = (&packet_listener, &plan.role) {
        command.args([descriptor_argument, &listener.as_raw_fd().to_string()]);
    }
    Err(command.exec().into())
}
