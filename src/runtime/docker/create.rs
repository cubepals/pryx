// SPDX-License-Identifier: FSL-1.1-ALv2
//! Construct a hardened workload container and install its trusted agent entrypoint.
use super::*;

pub(super) async fn create(runtime: &DockerRuntime, spec: &ContainerSpec) -> Result<String, RuntimeError> {
    let mut prepared = spec.clone();
    let resolver = runtime.egress.prepare(runtime, &mut prepared).await?;
    let spec = &prepared;
    if spec.internal_network {
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
    let options = CreateContainerOptionsBuilder::new().name(&spec.name).build();
    let mut body = body_of(&spec);
    runtime.native_tools.apply(&mut body);
    // Docker's embedded resolver forwards to this one, from the container's side of the bridge.
    if let (Some(resolver), Some(host)) = (resolver, body.host_config.as_mut()) {
        host.dns = Some(vec![resolver.to_string()]);
    }
    let created = quick("create", runtime.docker()?.create_container(Some(options), body)).await?;
    Ok(created.id)
}
