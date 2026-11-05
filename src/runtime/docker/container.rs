// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Docker container construction and inspection translation. Runtime operations and error
//! classification remain in the parent module; hardening always comes from host policy.

use crate::protocol::Health;
use crate::runtime::{ContainerInfo, ContainerSpec, ContainerStatus, parse_time};
use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, ContainerStateStatusEnum, HealthStatusEnum, HostConfig,
    HostConfigLogConfig, Mount, MountType, PortBinding as DockerPortBinding, RestartPolicy as DockerRestartPolicy,
    RestartPolicyNameEnum,
};
use std::collections::HashMap;

pub(super) fn info_from(inspect: ContainerInspectResponse) -> ContainerInfo {
    let state = inspect.state.unwrap_or_default();
    let config = inspect.config.unwrap_or_default();
    let status = match state.status {
        Some(ContainerStateStatusEnum::CREATED) => ContainerStatus::Created,
        Some(ContainerStateStatusEnum::RUNNING) => ContainerStatus::Running,
        Some(ContainerStateStatusEnum::PAUSED) => ContainerStatus::Paused,
        Some(ContainerStateStatusEnum::RESTARTING) => ContainerStatus::Restarting,
        Some(ContainerStateStatusEnum::REMOVING) => ContainerStatus::Removing,
        Some(ContainerStateStatusEnum::EXITED) => ContainerStatus::Exited,
        Some(ContainerStateStatusEnum::DEAD) => ContainerStatus::Dead,
        _ => ContainerStatus::Unknown,
    };
    let health = state.health.and_then(|h| h.status).and_then(|s| match s {
        HealthStatusEnum::STARTING => Some(Health::Starting),
        HealthStatusEnum::HEALTHY => Some(Health::Healthy),
        HealthStatusEnum::UNHEALTHY => Some(Health::Unhealthy),
        _ => None,
    });
    let env = config
        .env
        .unwrap_or_default()
        .into_iter()
        .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())))
        .collect();
    ContainerInfo {
        cgroup_parent: inspect.host_config.and_then(|h| h.cgroup_parent),
        id: inspect.id.unwrap_or_default(),
        name: inspect.name.unwrap_or_default().trim_start_matches('/').to_owned(),
        labels: config.labels.unwrap_or_default().into_iter().collect(),
        env,
        status,
        exit_code: state.exit_code.unwrap_or(0),
        oom_killed: state.oom_killed.unwrap_or(false),
        started_at: parse_time(state.started_at.as_deref()),
        finished_at: parse_time(state.finished_at.as_deref()),
        restart_count: inspect.restart_count.unwrap_or(0).max(0) as u32,
        health,
    }
}

pub(super) fn body_of(spec: &ContainerSpec) -> ContainerCreateBody {
    let mut exposed = Vec::new();
    let mut bindings: HashMap<String, Option<Vec<DockerPortBinding>>> = HashMap::new();
    for port in &spec.ports {
        let key = format!("{}/{}", port.container_port, port.protocol.as_str());
        exposed.push(key.clone());
        let host = port
            .host_ips
            .iter()
            .map(|ip| DockerPortBinding { host_ip: Some(ip.to_string()), host_port: Some(port.host_port.to_string()) })
            .collect();
        if !spec.internal_network {
            bindings.insert(key, Some(host));
        }
    }
    let tmpfs = spec.read_only_rootfs.then(|| {
        // exec: the JVM's native transports and some mods load libraries from /tmp, and the data
        // mount is executable anyway, so noexec here would break things and protect nothing.
        HashMap::from([("/tmp".to_owned(), format!("rw,exec,nosuid,nodev,size={}m", spec.tmp_size_mb))])
    });
    let mut mounts = vec![Mount {
        typ: Some(MountType::BIND),
        source: Some(spec.data_dir.to_string_lossy().into_owned()),
        target: Some(spec.mount_path.clone()),
        read_only: Some(false),
        ..Default::default()
    }];
    if let Some(agent) = &spec.agent {
        mounts.push(Mount {
            typ: Some(MountType::BIND),
            source: Some(agent.binary.to_string_lossy().into_owned()),
            target: Some(pryx_agent::contract::BINARY_PATH.into()),
            read_only: Some(true),
            ..Default::default()
        });
    }
    let host_config = HostConfig {
        cgroup_parent: spec.cgroup_parent.clone(),
        memory: Some(spec.memory_bytes),
        // Equal to memory: no swap. Swapping would stall a workload.
        memory_swap: Some(spec.memory_bytes),
        nano_cpus: spec.nano_cpus,
        cpu_shares: Some(spec.cpu_shares),
        pids_limit: Some(spec.pids_limit),
        readonly_rootfs: Some(spec.read_only_rootfs),
        tmpfs,
        cap_drop: Some(vec!["ALL".into()]),
        security_opt: Some(vec!["no-new-privileges:true".into()]),
        privileged: Some(false),
        ipc_mode: Some("private".into()),
        init: Some(false),
        // Docker never restarts a workload by itself: its on-failure also fires after an unclean
        // reboot, before pryxd runs, which would bring back a fenced copy. pryxd applies the
        // spec's policy itself (`restart_supervisor`).
        restart_policy: Some(DockerRestartPolicy { name: Some(RestartPolicyNameEnum::NO), maximum_retry_count: None }),
        mounts: Some(mounts),
        port_bindings: Some(bindings),
        network_mode: Some(spec.network.clone()),
        // Rotated and bounded: a chatty sandbox can't fill the disk with logs.
        log_config: Some(HostConfigLogConfig {
            typ: Some("local".into()),
            config: Some(HashMap::from([
                ("max-size".into(), format!("{}m", spec.log_max_size_mb)),
                ("max-file".into(), spec.log_max_files.to_string()),
            ])),
        }),
        oom_score_adj: Some(i64::from(spec.oom_score_adj)),
        ..Default::default()
    };
    ContainerCreateBody {
        image: Some(spec.image.clone()),
        entrypoint: spec.entrypoint.clone(),
        // An explicit argv is complete; do not append the image's default CMD.
        cmd: spec.entrypoint.as_ref().map(|_| Vec::new()),
        env: Some(spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect()),
        labels: Some(spec.labels.clone().into_iter().collect()),
        user: Some(spec.user.clone()),
        exposed_ports: Some(exposed),
        stop_signal: Some(spec.stop_signal.clone()),
        stop_timeout: Some(i64::from(spec.stop_timeout_secs)),
        host_config: Some(host_config),
        ..Default::default()
    }
}
