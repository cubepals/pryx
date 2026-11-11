// SPDX-License-Identifier: FSL-1.1-ALv2
//! Construct a hardened workload container and install its trusted agent entrypoint.
use super::*;

pub(super) async fn create(runtime: &DockerRuntime, spec: &ContainerSpec) -> Result<String, RuntimeError> {
    let info = runtime.info().await?;
    check_engine_policy(runtime, spec, &info)?;
    let mut prepared = spec.clone();
    let resolver = runtime.egress.prepare(runtime, &mut prepared).await?;
    let spec = &prepared;
    // Native Podman create owns a separate bridge and published ingress for each workload.
    if spec.internal_network && !info.podman {
        runtime.ensure_internal_network(&spec.network, &spec.labels).await?;
    }
    let mut spec = spec.clone();
    if spec.internal_network {
        spec.labels.insert(
            crate::labels::LABEL_INTERNAL_BINDINGS.into(),
            serde_json::to_string(&spec.ports)
                .map_err(|_| RuntimeError::Other("Cannot encode private bindings".into()))?,
        );
    }
    if let Some(agent) = &spec.agent {
        let image = quick("inspect image", runtime.docker()?.inspect_image(&spec.image)).await?;
        let config = image.config.unwrap_or_default();
        let original = if agent.keep_alive {
            Vec::new()
        } else {
            spec.entrypoint.take().unwrap_or_else(|| {
                config.entrypoint.unwrap_or_default().into_iter().chain(config.cmd.unwrap_or_default()).collect()
            })
        };
        let mut entrypoint = vec![
            pryx_agent::contract::BINARY_PATH.into(),
            "--sandbox".into(),
            agent.sandbox.clone(),
            "--epoch".into(),
            agent.epoch.to_string(),
            "--data".into(),
            spec.mount_path.clone(),
            "--domain".into(),
            agent.domain.clone(),
            "--port".into(),
            agent.agent_port.to_string(),
            "--secret-env".into(),
            agent.secret_env.clone(),
            "--entrypoint-cwd".into(),
            spec.workdir.clone().or(config.working_dir).filter(|d| !d.is_empty()).unwrap_or_else(|| "/".into()),
        ];
        if let Some(origin) = &agent.public_origin {
            entrypoint.extend(["--public-origin".into(), origin.clone()]);
        }
        if !agent.spare_ports.is_empty() {
            entrypoint.extend([
                "--spare-ports".into(),
                agent.spare_ports.iter().map(u16::to_string).collect::<Vec<_>>().join(","),
            ]);
        }
        if agent.keep_alive {
            entrypoint.push("--keep-alive".into());
        }
        if !original.is_empty() {
            entrypoint.push("--".into());
            entrypoint.extend(original);
        }
        spec.entrypoint = Some(entrypoint);
    }
    if info.podman {
        return podman::create(runtime, &spec, &info).await;
    }
    let options = CreateContainerOptionsBuilder::new().name(&spec.name).build();
    let mut body = body_of(&spec);
    runtime.native_tools.apply(&mut body);
    if let Some(host) = body.host_config.as_mut() {
        if !info.memory_limit {
            host.memory = None;
            host.memory_swap = None;
        }
        if !info.cpu_limit {
            host.cpu_shares = None;
        }
        if !info.pids_limit {
            host.pids_limit = None;
        }
    }
    // Docker's embedded resolver forwards to this one, from the container's side of the bridge.
    if let (Some(resolver), Some(host)) = (resolver, body.host_config.as_mut()) {
        host.dns = Some(vec![resolver.to_string()]);
    }
    let created = quick("create", runtime.docker()?.create_container(Some(options), body)).await?;
    Ok(created.id)
}

fn check_engine_policy(runtime: &DockerRuntime, spec: &ContainerSpec, info: &RuntimeInfo) -> Result<(), RuntimeError> {
    if let Some(problem) = &info.container_policy_problem {
        return Err(RuntimeError::Other(problem.clone()));
    }
    if info.rootless && !info.podman && spec.internal_network {
        return Err(RuntimeError::Other(
            "network-policy is unavailable: rootless Docker cannot provide private internal ingress; use a supported Podman engine".into(),
        ));
    }
    if !info.memory_limit && spec.allow_unenforced_memory != Some(true) {
        return Err(RuntimeError::Other(format!(
            "{} is unavailable on this engine",
            crate::protocol::Guarantee::MemoryLimit.feature()
        )));
    }
    if !info.cpu_limit && (spec.nano_cpus.is_some() || spec.require_cpu_limit) {
        return Err(RuntimeError::Other(format!(
            "{} is unavailable on this engine",
            crate::protocol::Guarantee::CpuLimit.feature()
        )));
    }
    if !info.pids_limit && spec.require_pids_limit {
        return Err(RuntimeError::Other("pids-limit is unavailable on this engine".into()));
    }
    if (info.rootless || info.podman) && (spec.egress.is_some() || runtime.native_tools.fuse()) {
        return Err(RuntimeError::Other(
            "host-firewall and native FUSE require a supported rootful Docker engine".into(),
        ));
    }
    Ok(())
}
