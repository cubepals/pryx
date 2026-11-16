// SPDX-License-Identifier: FSL-1.1-ALv2
//! Construct a hardened workload container and install its trusted agent entrypoint.
use super::*;

pub(super) async fn create(runtime: &DockerRuntime, spec: &ContainerSpec) -> Result<String, RuntimeError> {
    let info = runtime.info().await?;
    check_engine_policy(runtime, spec, &info)?;
    check_platform(&spec.image, &quick("inspect image", runtime.docker()?.inspect_image(&spec.image)).await?)?;
    if let Some(problem) = ownership::data_problem(runtime, &info, &spec.data_dir).await {
        return Err(RuntimeError::Other(problem));
    }
    let mut prepared = spec.clone();
    let resolver = runtime.egress.prepare(runtime, &mut prepared).await?;
    let spec = &prepared;
    // Native Podman create owns a separate bridge and published ingress for each workload.
    if spec.internal_network && !info.podman {
        runtime.ensure_internal_network(&spec.network, &spec.labels).await?;
    }
    let mut spec = spec.clone();
    if (runtime.oci_runtime.is_some() || (info.rootless && !info.podman && spec.internal_network))
        && let Some(agent) = spec.agent.as_mut()
    {
        let binary = agent.binary.clone();
        let data = spec.data_dir.clone();
        agent.binary = tokio::task::spawn_blocking(move || super::exec_agent::stage(&binary, &data))
            .await
            .map_err(|_| RuntimeError::Other("exec agent staging failed".into()))??;
    }
    if let Some(own) = runtime.oci_runtime {
        spec.labels.insert(crate::labels::LABEL_OCI_RUNTIME.into(), own.into());
    }
    if spec.internal_network {
        spec.labels.insert(
            crate::labels::LABEL_INTERNAL_BINDINGS.into(),
            serde_json::to_string(&spec.ports)
                .map_err(|_| RuntimeError::Other("Cannot encode private bindings".into()))?,
        );
    }
    // An image's own label can never enable a private transport.
    spec.labels.insert(crate::labels::LABEL_INTERNAL_INGRESS.into(), "0".into());
    agent_entrypoint(runtime, &mut spec).await?;
    if info.rootless
        && !info.podman
        && spec.internal_network
        && spec
            .labels
            .get(crate::labels::LABEL_INTERNAL_INGRESS)
            .and_then(|value| value.parse::<u16>().ok())
            .is_none_or(|version| version < pryx_agent::exec_ingress::VERSION)
    {
        return Err(RuntimeError::Other("network-policy needs a current protected static ingress agent".into()));
    }
    if info.podman {
        return podman::create(runtime, &spec, &info).await;
    }
    let options = CreateContainerOptionsBuilder::new().name(&spec.name).build();
    let mut body = body_of(&spec);
    runtime.native_tools.apply(&mut body, runtime.oci_runtime.is_none());
    if let Some(host) = body.host_config.as_mut() {
        host.runtime = super::gvisor::engine_runtime(runtime.oci_runtime, &info);
        if runtime.oci_runtime.is_some() {
            super::gvisor::process_limit(host, spec.pids_limit);
        }
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
    // gVisor's network stack can't reach the embedded resolver: its sandboxes ask the host's.
    if runtime.oci_runtime.is_some()
        && !spec.internal_network
        && let Some(mounts) = body.host_config.as_mut().and_then(|host| host.mounts.as_mut())
    {
        mounts.push(bollard::models::Mount {
            typ: Some(bollard::models::MountType::BIND),
            source: Some(super::gvisor::resolv_conf_for(&spec.data_dir)?.to_string_lossy().into_owned()),
            target: Some("/etc/resolv.conf".into()),
            read_only: Some(true),
            ..Default::default()
        });
    }
    let created = quick("create", runtime.docker()?.create_container(Some(options), body)).await?;
    Ok(created.id)
}

/// This host's CPU in an image's terms: the OCI image spec's `architecture`, which takes Go's
/// GOARCH names (image-spec config.md), for the machine pryxd itself is built for.
fn host_architecture() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("amd64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

/// An image built for another CPU would run only under an emulator, or fail at its first exec:
/// it is refused here, naming both, rather than either in silence.
fn check_platform(image: &str, inspected: &bollard::models::ImageInspect) -> Result<(), RuntimeError> {
    let (Some(host), Some(built)) = (host_architecture(), inspected.architecture.as_deref()) else { return Ok(()) };
    if built == host {
        return Ok(());
    }
    Err(RuntimeError::Other(format!(
        "{image} is built for linux/{built} and this host runs linux/{host}; pryx runs no image under emulation, so a host of its machine runs it"
    )))
}

fn check_engine_policy(runtime: &DockerRuntime, spec: &ContainerSpec, info: &RuntimeInfo) -> Result<(), RuntimeError> {
    if let Some(problem) = &info.container_policy_problem {
        return Err(RuntimeError::Other(problem.clone()));
    }
    if info.rootless
        && !info.podman
        && spec.internal_network
        && (info.private_ingress != Some(true) || spec.agent.is_none())
    {
        return Err(RuntimeError::Other(
            "network-policy is unavailable: rootless Docker needs a current protected static ingress agent; use the installed agent or a supported Podman engine".into(),
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
    if runtime.oci_runtime.is_some() && spec.egress.is_some() {
        return Err(RuntimeError::Other(super::gvisor::HOSTNAME_EGRESS_PROBLEM.into()));
    }
    if (info.rootless || info.podman) && (spec.egress.is_some() || runtime.native_tools.fuse()) {
        return Err(RuntimeError::Other(
            "host-firewall and native FUSE require a supported rootful Docker engine".into(),
        ));
    }
    Ok(())
}

async fn agent_entrypoint(runtime: &DockerRuntime, spec: &mut ContainerSpec) -> Result<(), RuntimeError> {
    if let Some(agent) = &spec.agent {
        super::exec_agent::negotiate(&agent.binary, &mut spec.labels).await;
        let image = quick("inspect image", runtime.docker()?.inspect_image(&spec.image)).await?;
        let config = image.config.unwrap_or_default();
        let browser = browser_policy::mode(runtime, spec, config.env.as_deref()).await?;
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
        browser_policy::bind(browser, &mut entrypoint, &mut spec.labels);
        spec.entrypoint = Some(entrypoint);
    }
    Ok(())
}

#[cfg(test)]
mod platform_tests {
    #[test]
    fn an_image_of_another_machine_is_refused_by_name() {
        let host = super::host_architecture().expect("pryx is built for x86_64 or aarch64");
        let built = |architecture: Option<&str>| bollard::models::ImageInspect {
            architecture: architecture.map(str::to_owned),
            ..Default::default()
        };
        super::check_platform("alpine", &built(Some(host))).unwrap();
        super::check_platform("alpine", &built(None)).unwrap();
        let other = if host == "amd64" { "arm64" } else { "amd64" };
        let refused = super::check_platform("ghcr.io/cubepals/pryx-tools:1", &built(Some(other))).unwrap_err();
        assert!(refused.to_string().contains(&format!("linux/{other}")), "{refused}");
        assert!(refused.to_string().contains(&format!("linux/{host}")), "{refused}");
    }
}
