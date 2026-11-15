// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Closed host-side VM process launch with a lifetime kernel lease and host-owned confinement.
//! The workload never selects these paths or arguments. Ordinary VMMs and packet proxies inherit
//! separate Landlock/seccomp grants; root executes the unchanged jailer for the VMM.
//!
//! Parts (`vm_monitor/`):
//! - `packet.rs`: inherited listener/TAP validation and packet-helper confinement.
//! - `filter.rs`: preserve the inherited kernel lease and forbid cross-process host operations.
//! - `namespace_tests.rs`: actual descriptor identity and namespace-type refusal checks.
//! - `tool.rs`: one host filesystem program over its staged files, with no sockets or capabilities.
mod filter;
#[cfg(test)]
mod namespace_tests;
mod packet;
mod tool;
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
            process::CommandExt,
        },
    },
    path::{Component, Path, PathBuf},
};

pub const MODE: &str = "vm-monitor";
// Linux's Universal TUN/TAP documentation names this device. Its actual device
// number validates a passed queue; no copied major/minor value is used.
pub const TUN_DEVICE: &str = "/dev/net/tun";
pub const VERSION: u16 = 1;

/// Fixed protected host resolver inputs for the ordinary attached-packet launcher.
/// Host callers can check this policy before creating or transferring a queue.
pub fn packet_resolver_files() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    packet::resolver_files()
}
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
    Jailer {
        uid: u32,
        gid: u32,
        id: String,
        base: PathBuf,
        monitor: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<Namespace>,
    },
    PacketTap {
        descriptor_argument: String,
    },
    PacketProxy {
        socket: PathBuf,
        descriptor_argument: String,
    },
    /// A host filesystem program over guest- or tenant-written images, as root or not.
    FilesystemTool {
        /// The largest file it may write (RLIMIT_FSIZE).
        file_bytes: u64,
        /// The e2fsprogs profile it reads, one of its read grants.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        configuration: Option<PathBuf>,
    },
}

/// A retained parent descriptor, identified by the actual kernel inode, never a workload path.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Namespace {
    pub descriptor: i32,
    pub device: u64,
    pub inode: u64,
}

fn enter_namespace(namespace: &Namespace, parent: u32) -> io::Result<()> {
    if namespace.descriptor <= libc::STDERR_FILENO {
        return Err(invalid("Invalid retained network namespace descriptor"));
    }
    let file = fs::File::open(format!("/proc/{parent}/fd/{}", namespace.descriptor))?;
    let identity = file.metadata()?;
    if identity.dev() != namespace.device
        || identity.ino() != namespace.inode
        || rustix::fs::fstatfs(&file)?.f_type != libc::NSFS_MAGIC
    {
        return Err(invalid("Retained network namespace identity changed"));
    }
    rustix::thread::move_into_link_name_space(
        std::os::fd::AsFd::as_fd(&file),
        Some(rustix::thread::LinkNameSpaceType::Network),
    )?;
    Ok(())
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
    if matches!(plan.role, Role::PacketTap { .. }) {
        read.extend(packet::resolver_files()?);
    }
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
    // Where the kernel has Landlock's abstract-socket scope (ABI 6), the process also cannot
    // reach an abstract Unix socket outside its own domain; it uses none of its own.
    let scope = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .scope(landlock::Scope::AbstractUnixSocket)
        .and_then(|rules| rules.create());
    if let Ok(scope) = scope
        && scope.restrict_self()?.ruleset != RulesetStatus::FullyEnforced
    {
        return Err("VM monitor abstract-socket scope was not enforced".into());
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
    let packet_proxy = matches!(plan.role, Role::PacketProxy { .. } | Role::PacketTap { .. });
    let tool = matches!(plan.role, Role::FilesystemTool { .. });
    if !tool && jailer != rustix::process::geteuid().is_root() {
        return Err("Root VM monitors require the jailer".into());
    }
    let created = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&plan.lock)?;
    let meta = created.metadata()?;
    if !meta.is_file() || meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o077 != 0 {
        return Err("Monitor lease must be a private owned regular file".into());
    }
    // Creation needs a writable open, but the inherited descriptor grants no writes
    // outside the monitor's filesystem boundary. Verify the reopened inode before locking.
    let lock = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&plan.lock)?;
    let opened = lock.metadata()?;
    if opened.dev() != meta.dev() || opened.ino() != meta.ino() {
        return Err("Monitor lease changed while opening its read-only descriptor".into());
    }
    drop(created);
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
    let mut packet_tap = None;
    match &plan.role {
        Role::Ordinary => confine(&plan, true)?,
        Role::FilesystemTool { file_bytes, configuration } => {
            tool::confine(&plan, *file_bytes, configuration.as_deref(), &mut command)?
        }
        Role::PacketProxy { socket, descriptor_argument } => {
            packet_listener = Some(packet::listener(&plan, socket, descriptor_argument)?);
        }
        Role::PacketTap { descriptor_argument } => {
            packet_tap = Some(packet::tap(&plan, descriptor_argument)?);
        }
        Role::Jailer { uid, gid, id, base, monitor, namespace } => {
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
            // The actual monitor lease was acquired before opening the retained handle.
            // Only this short-lived launcher enters it; the daemon keeps its original namespace.
            if let Some(namespace) = namespace {
                enter_namespace(namespace, plan.parent)?;
            }
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
    filter::enter(
        jailer,
        if packet_proxy {
            filter::Sockets::Internet
        } else if tool {
            filter::Sockets::None
        } else {
            filter::Sockets::Unix
        },
    )?;
    command.args(&plan.arguments);
    if let (Some(listener), Role::PacketProxy { descriptor_argument, .. }) = (&packet_listener, &plan.role) {
        command.args([descriptor_argument, &listener.as_raw_fd().to_string()]);
    }
    if let (Some(queue), Role::PacketTap { descriptor_argument }) = (&packet_tap, &plan.role) {
        command.args([descriptor_argument, &queue.as_raw_fd().to_string()]);
    }
    Err(command.exec().into())
}
