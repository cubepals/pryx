// SPDX-License-Identifier: FSL-1.1-ALv2
//! Assemble private guest bootstrap arguments from admitted host policy and OCI configuration.
//! Secrets remain in the returned in-memory Boot; this module writes no files or journals.
use super::{
    super::{error, image::Prepared},
    configuration::Configuration,
};
use crate::runtime::{ContainerSpec, RuntimeError};
use pryx_agent::vm_boot::{self, Boot};
use std::collections::BTreeMap;

pub(super) fn boot(
    spec: &ContainerSpec,
    id: &str,
    image: &Prepared,
    configuration: &Configuration,
    identity: (u32, u32),
) -> Result<Boot, RuntimeError> {
    let boot_agent = spec.agent.as_ref().ok_or_else(|| error("VM agent bootstrap is missing"))?;
    let config = image.configuration.config().as_ref();
    let mut environment = config
        .and_then(|config| config.env().as_ref())
        .into_iter()
        .flatten()
        .filter_map(|value| value.split_once('=').map(|(key, value)| (key.to_owned(), value.to_owned())))
        .collect::<BTreeMap<_, _>>();
    environment.extend(spec.env.iter().cloned());
    let mut agent_args = vec![
        "--sandbox".into(),
        boot_agent.sandbox.clone(),
        "--epoch".into(),
        boot_agent.epoch.to_string(),
        "--data".into(),
        spec.mount_path.clone(),
        "--domain".into(),
        boot_agent.domain.clone(),
        "--port".into(),
        boot_agent.agent_port.to_string(),
        "--secret-env".into(),
        boot_agent.secret_env.clone(),
        "--vm-port-base".into(),
        vm_boot::SERVICE_PORT_BASE.to_string(),
        "--entrypoint-cwd".into(),
        spec.workdir
            .clone()
            .or_else(|| config.and_then(|c| c.working_dir().clone()))
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "/".into()),
    ];
    if let Some(origin) = &boot_agent.public_origin {
        agent_args.extend(["--public-origin".into(), origin.clone()]);
    }
    if !boot_agent.spare_ports.is_empty() {
        agent_args.extend([
            "--spare-ports".into(),
            boot_agent.spare_ports.iter().map(u16::to_string).collect::<Vec<_>>().join(","),
        ]);
    }
    let declared = spec
        .ports
        .iter()
        .map(|p| p.container_port)
        .filter(|p| *p != boot_agent.agent_port && !boot_agent.spare_ports.contains(p))
        .collect::<std::collections::BTreeSet<_>>();
    if !declared.is_empty() {
        agent_args.extend(["--vm-ports".into(), declared.iter().map(u16::to_string).collect::<Vec<_>>().join(",")]);
    }
    if boot_agent.keep_alive {
        agent_args.push("--keep-alive".into());
    }
    let original = spec.entrypoint.clone().unwrap_or_else(|| {
        config
            .into_iter()
            .flat_map(|c| {
                c.entrypoint().clone().unwrap_or_default().into_iter().chain(c.cmd().clone().unwrap_or_default())
            })
            .collect()
    });
    if !boot_agent.keep_alive && !original.is_empty() {
        agent_args.push("--".into());
        agent_args.extend(original);
    }
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let boot = Boot {
        version: vm_boot::VERSION,
        nonce: nonce.clone(),
        cid: configuration.cid,
        layers: configuration
            .ids
            .iter()
            .map(|id| configuration.identities.get(id).cloned().ok_or_else(|| error("VM layer identity is missing")))
            .collect::<Result<_, _>>()?,
        writes: configuration
            .identities
            .get(&configuration.write_id)
            .cloned()
            .ok_or_else(|| error("VM disk identity is missing"))?,
        hostname: format!("pryx-{}", id),
        uid: identity.0,
        gid: identity.1,
        data: spec.mount_path.clone(),
        read_only_root: spec.read_only_rootfs,
        temporary_bytes: u64::from(spec.tmp_size_mb) * 1024 * 1024,
        pids_limit: spec.pids_limit as u64,
        arguments: agent_args,
        environment: environment.into_iter().collect(),
        network_arguments: None,
        network: None,
    };
    Ok(boot)
}
