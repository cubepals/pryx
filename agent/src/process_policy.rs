// SPDX-License-Identifier: FSL-1.1-ALv2
//! Process tier restrictions are host supplied, applied before the agent starts workers, and
//! inherited by every command. Network-off restrictions are added in a fresh command launcher,
//! leaving the trusted supervisor's preallocated listener reachable.
//!
//! Parts (`process_policy/`):
//! - `filter.rs`: deny dangerous syscalls and namespace creation.
//! - `watchdog.rs`: resident-memory sampling outside the filesystem-restricted worker threads.
mod filter;
mod watchdog;
use landlock::{
    ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
    Scope, path_beneath_rules,
};
use rustix::process::{Resource, Rlimit, setrlimit};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

pub const POLICY_ENV: &str = "PRYX_PROCESS_POLICY";
pub const NETWORK_ENV: &str = "PRYX_PROCESS_NETWORK_OFF";
static IPC_SCOPED: AtomicBool = AtomicBool::new(false);

/// An installed supervisor restriction, distinct from the disposable capability probe.
pub fn ipc_scoped() -> bool {
    IPC_SCOPED.load(Ordering::Acquire)
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub data: PathBuf,
    pub memory_bytes: u64,
    pub address_space_bytes: u64,
    pub open_files: u64,
    pub processes: u64,
    pub cpu_seconds: u64,
    pub nice: i32,
    pub network_off: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<crate::process_tool_policy::Policy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_ipc_scope: Option<bool>,
}

/// A trusted tool worker and its supervisor must inherit the same private abstract-socket domain.
pub fn scope() -> Result<(), Box<dyn std::error::Error>> {
    if ipc_scoped() {
        return Ok(());
    }
    if !crate::process_capabilities::private_ipc_probe().supported() {
        return Err("Native process tools require enforced Landlock abstract-socket isolation".into());
    }
    let status = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .scope(Scope::AbstractUnixSocket)?
        .create()?
        .restrict_self()?;
    if status.ruleset != RulesetStatus::FullyEnforced {
        return Err("Native process tools require enforced Landlock abstract-socket isolation".into());
    }
    IPC_SCOPED.store(true, Ordering::Release);
    Ok(())
}

pub fn enter(policy: &Policy) -> Result<(), Box<dyn std::error::Error>> {
    enter_variant(policy, false, false)
}

/// The fixed helper has already installed private user, mount, PID and proc namespaces.
pub fn browser(policy: &Policy) -> Result<(), Box<dyn std::error::Error>> {
    enter_variant(policy, true, false)
}

/// The fixed desktop helper has a private temporary mount and inherits its supervisor's IPC scope.
pub fn desktop(policy: &Policy) -> Result<(), Box<dyn std::error::Error>> {
    enter_variant(policy, false, true)
}

fn enter_variant(policy: &Policy, browser: bool, desktop: bool) -> Result<(), Box<dyn std::error::Error>> {
    if !crate::process_capabilities::probe().filesystem {
        return Err(crate::process_capabilities::FILESYSTEM_UNAVAILABLE.into());
    }
    watchdog::start(policy.memory_bytes)?;
    for (resource, value) in [
        (Resource::As, policy.address_space_bytes),
        (Resource::Nofile, policy.open_files),
        (Resource::Nproc, policy.processes),
        (Resource::Cpu, policy.cpu_seconds),
        (Resource::Core, 0),
    ] {
        setrlimit(resource, Rlimit { current: Some(value), maximum: Some(value) })?;
    }
    rustix::process::setpriority_process(None, policy.nice)?;
    let abi = ABI::V3;
    let read = [
        "/usr",
        "/bin",
        "/sbin",
        "/lib",
        "/lib64",
        "/etc/ssl/certs",
        "/etc/ld.so.cache",
        "/etc/passwd",
        "/etc/group",
        "/etc/resolv.conf",
        "/etc/nsswitch.conf",
        "/etc/mime.types",
        "/etc/debian_version",
        "/etc/hosts",
        "/etc/ssl/openssl.cnf",
        "/dev/urandom",
        "/dev/zero",
    ];
    let mut read: Vec<PathBuf> = read.into_iter().map(PathBuf::from).filter(|p| p.exists()).collect();
    read.push(std::env::current_exe()?);
    read.push(PathBuf::from(format!("/proc/{}/task", std::process::id())));
    if let Some(tools) = &policy.tools
        && tools.unavailable.is_none()
    {
        read.push(tools.cache.clone());
    }
    if browser {
        read.extend([PathBuf::from("/proc"), PathBuf::from("/sys/devices/system/cpu")]);
    }
    let mut write = vec![
        policy.data.as_path(),
        std::path::Path::new("/dev/null"),
        std::path::Path::new("/dev/pts"),
        std::path::Path::new("/dev/ptmx"),
    ];
    if let Some(tools) = &policy.tools
        && tools.unavailable.is_none()
    {
        write.push(tools.scratch.as_path());
    }
    if browser {
        write.extend([std::path::Path::new("/proc"), std::path::Path::new("/dev/shm")]);
    }
    if desktop {
        write.extend([std::path::Path::new("/tmp"), std::path::Path::new("/dev/shm")]);
    }
    let scoped = browser || desktop || ipc_scoped() || crate::process_capabilities::private_ipc_probe().supported();
    if policy.require_ipc_scope == Some(true) && !scoped {
        return Err("This process requires enforced private IPC before guest execution".into());
    }
    let mut rules =
        Ruleset::default().set_compatibility(CompatLevel::HardRequirement).handle_access(AccessFs::from_all(abi))?;
    if scoped && !browser && !desktop && !ipc_scoped() {
        rules = rules.scope(Scope::AbstractUnixSocket)?;
    }
    let status = rules
        .create()?
        .add_rules(path_beneath_rules(read, AccessFs::from_read(abi)))?
        .add_rules(path_beneath_rules(write, AccessFs::from_all(abi)))?
        .restrict_self()?;
    if status.ruleset != RulesetStatus::FullyEnforced {
        return Err("Landlock filesystem policy is unavailable".into());
    }
    if browser {
        filter::browser()?;
    } else {
        filter::apply(false)?;
    }
    IPC_SCOPED.store(scoped, Ordering::Release);
    Ok(())
}

pub fn network_off() -> Result<(), Box<dyn std::error::Error>> {
    // Landlock network filtering covers TCP; seccomp also denies datagram/Unix egress.
    let status = Ruleset::default().handle_access(AccessNet::ConnectTcp)?.create()?.restrict_self()?;
    let _landlock = status.ruleset == RulesetStatus::FullyEnforced;
    filter::apply(true)?;
    Ok(())
}

pub fn child(supervisor: u32, network: bool) -> Result<(), Box<dyn std::error::Error>> {
    if rustix::process::getpgrp().as_raw_nonzero().get() as u32 == supervisor {
        return Err("Guest must have a separate process group".into());
    }
    filter::child(supervisor)?;
    if network {
        network_off()?;
    }
    Ok(())
}

pub fn wrap(argv: Vec<String>) -> std::io::Result<Vec<String>> {
    if std::env::var_os(POLICY_ENV).is_some() {
        let mut wrapped = vec![
            std::env::current_exe()?.to_string_lossy().into_owned(),
            "process-child".into(),
            std::process::id().to_string(),
            std::env::var_os(NETWORK_ENV).is_some().to_string(),
        ];
        wrapped.extend(argv);
        return Ok(wrapped);
    }
    if std::env::var_os(NETWORK_ENV).is_none() {
        return Ok(argv);
    }
    let mut wrapped = vec![std::env::current_exe()?.to_string_lossy().into_owned(), "network-off".into()];
    wrapped.extend(argv);
    Ok(wrapped)
}
