// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Admitted monitor resources and the host-owned ordinary confinement plan. This module
//! does not acquire images, launch compute, select a tier or change host authority.
use super::*;
use crate::runtime::Proto;
use pryx_agent::vm_monitor::{Plan, Role};

// Docker Engine HostConfig.NanoCpus uses billionths of a CPU; Bollard exports no conversion constant.
const NANO_CPU: i64 = 1_000_000_000;

pub(super) fn resources(spec: &ContainerSpec) -> Result<(u64, u32, u32, u32), RuntimeError> {
    let memory = u64::try_from(spec.memory_bytes).map_err(error)?;
    if memory == 0
        || memory % (1024 * 1024) != 0
        || spec.storage_bytes == 0
        || spec.pids_limit <= 0
        || spec.tmp_size_mb == 0
    {
        return Err(error("VM requires whole MiB of memory and positive admitted disk/PID limits"));
    }
    if spec.stop_signal != crate::protocol::StopSignal::SIGTERM.as_str()
        && spec.stop_signal != crate::protocol::StopSignal::SIGINT.as_str()
    {
        return Err(error("VM orderly shutdown requires SIGTERM or SIGINT"));
    }
    if spec.cpu_weight_requested {
        return Err(error("VM CPU weights require host controller integration; this checkpoint cannot enforce them"));
    }
    let cpus = match spec.nano_cpus {
        Some(n) if n > 0 && n % NANO_CPU == 0 => u32::try_from(n / NANO_CPU).map_err(error)?,
        Some(_) => return Err(error("VM guest-size CPU limits require a positive whole vCPU count")),
        None => 1,
    };
    let (uid, gid) = spec.user.split_once(':').unwrap_or((&spec.user, &spec.user));
    let (uid, gid) = (uid.parse::<u32>().map_err(error)?, gid.parse::<u32>().map_err(error)?);
    if uid == 0 || gid == 0 {
        return Err(error("VM host policy requires non-root numeric guest IDs"));
    }
    if spec.internal_network && spec.egress.is_some() {
        return Err(error("A disconnected VM cannot request hostname egress"));
    }
    if !spec.packages.is_empty() {
        return Err(error("VM images provide their own packages; registry kernels are not enabled for this tier"));
    }
    if spec.ports.iter().any(|port| port.protocol != Proto::Tcp) {
        return Err(error("VM service forwarding currently supports TCP only"));
    }
    Ok((memory / (1024 * 1024), cpus, uid, gid))
}

pub(super) fn plan(
    bundle: &Bundle,
    layers: Vec<PathBuf>,
    disk: PathBuf,
    directory: &Path,
    lease: PathBuf,
    api: &Path,
    config_path: &Path,
) -> Plan {
    let mut read = vec![
        bundle.root().join("kernel/vmlinux"),
        bundle.root().join("bootstrap/bootstrap.erofs"),
        config_path.to_owned(),
    ];
    read.extend(layers);
    Plan {
        program: bundle.root().join("bin/firecracker"),
        arguments: vec![
            "--api-sock".into(),
            api.display().to_string(),
            "--config-file".into(),
            config_path.display().to_string(),
        ],
        directory: directory.into(),
        lock: lease,
        parent: std::process::id(),
        read,
        write: vec![disk],
        role: Role::Ordinary,
    }
}
