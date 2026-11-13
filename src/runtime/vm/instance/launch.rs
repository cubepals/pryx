// SPDX-License-Identifier: FSL-1.1-ALv2
//! Build private monitor configuration from generated owner models, then boot a confined
//! guest and expose only admitted TCP services. Spec secrets remain in the one-shot bootstrap.
use super::*;
use crate::runtime::{PortBinding, Proto};
use pryx_agent::{
    vm_boot::{self, Boot},
    vm_monitor::{Plan, Role},
};
use serde_json::Value;
use std::process::Stdio;
use tokio::{io::AsyncReadExt, process::Command, time::Instant};

const BOOT_TIMEOUT: Duration = Duration::from_secs(30);
// Docker Engine HostConfig.NanoCpus uses billionths of a CPU; Bollard exports no conversion constant.
const NANO_CPU: i64 = 1_000_000_000;

async fn output(mut command: Command, deadline: Instant) -> Result<Vec<u8>, RuntimeError> {
    command.env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    tokio::time::timeout_at(deadline, async {
        let mut child = command.spawn().map_err(error)?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| error("Missing VM configuration output"))?
            .take(pryx_agent::vm_boot::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(error)?;
        if bytes.len() > pryx_agent::vm_boot::MAX_BYTES || !child.wait().await.map_err(error)?.success() {
            return Err(error("VM configurator refused its guest resources or drive paths"));
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| RuntimeError::Timeout("VM configuration exceeded its deadline".into()))?
}

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
    if spec.stop_signal != crate::protocol::StopSignal::SIGTERM.as_str() {
        return Err(error("VM orderly shutdown supports SIGTERM only in this checkpoint"));
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
    if !spec.internal_network || spec.egress.is_some() {
        return Err(error("Production VM networking and hostname policy are not ready in this checkpoint"));
    }
    if !spec.packages.is_empty() {
        return Err(error("VM images provide their own packages; registry kernels are not enabled for this tier"));
    }
    if spec.ports.iter().any(|port| port.protocol != Proto::Tcp) {
        return Err(error("VM service forwarding currently supports TCP only"));
    }
    Ok((memory / (1024 * 1024), cpus, uid, gid))
}

impl VmRuntime {
    pub(super) async fn prepared(&self, image: &str, timeout: Duration) -> Result<Prepared, RuntimeError> {
        let bundle = self.bundle().await?;
        super::super::image::prepare(
            &bundle.root().join("bin/pryx-vm-image"),
            &super::super::image::Request {
                component: bundle.root().join("filesystem"),
                manifest_sha256: bundle.filesystems().identity().into(),
                cache: self.state.root.join("images"),
                image: image.into(),
                timeout_seconds: timeout.as_secs(),
            },
        )
        .await
    }
    pub(super) async fn launch_entry(&self, name: &str) -> Result<(), RuntimeError> {
        let entry = self.entry(name)?;
        let transition = self.transition(&entry.id);
        let _transition = transition.lock().await;
        let previous = self.state.live.lock().unwrap().get(&entry.id).cloned();
        if let Some(previous) = previous {
            let mut previous = previous.lock().await;
            if previous.monitor.exited().map_err(error)?.is_none() {
                return Ok(());
            }
            previous.forwarder.stop().await.map_err(error)?;
            drop(previous);
            self.state.live.lock().unwrap().remove(&entry.id);
        }
        let entry = self.entry(name)?;
        let (memory, cpus, uid, gid) = resources(&entry.spec)?;
        let image = self.prepared(&entry.spec.image, self.state.settings.image_timeout).await?;
        let bundle = self.bundle().await?;
        let lease = self.lease(&entry)?;
        let disk = super::super::disk::prepare(
            bundle.filesystems(),
            &entry.spec.data_dir,
            &lease,
            entry.spec.storage_bytes,
            BOOT_TIMEOUT,
            self.state.settings.free_floor_bytes,
        )
        .await?;
        let directory = boot_directory(&self.state.root.join("boots"))?;
        let channel = directory.path().join("vsock");
        let api = directory.path().join("api");
        let serial = directory.path().join("serial");
        let config_path = directory.path().join("monitor.json");
        let deadline = Instant::now() + BOOT_TIMEOUT;
        let mut arguments = vec![
            "--kernel".into(),
            bundle.root().join("kernel/vmlinux").display().to_string(),
            "--root".into(),
            bundle.root().join("bootstrap/bootstrap.erofs").display().to_string(),
            "--writes".into(),
            disk.path.display().to_string(),
            "--socket".into(),
            channel.display().to_string(),
            "--init-mode".into(),
            bundle.bootstrap().metadata.vm_init_mode.clone(),
            "--memory-mib".into(),
            memory.to_string(),
            "--vcpus".into(),
            cpus.to_string(),
        ];
        let ids = image.layers.iter().enumerate().map(|(i, _)| format!("layer{i}")).collect::<Vec<_>>();
        for (id, layer) in ids.iter().zip(&image.layers) {
            arguments.extend(["--layer".into(), format!("{id}={}", layer.display())]);
        }
        let configurator = bundle.root().join("bin/vm-config");
        let mut command = Command::new(&configurator);
        command.args(&arguments);
        let config = output(command, deadline).await?;
        crate::durable::write_atomic(&config_path, &config, 0o600).map_err(error)?;
        let mut command = Command::new(&configurator);
        command.args(&arguments).arg("--identities");
        let identities: BTreeMap<String, String> =
            serde_json::from_slice(&output(command, deadline).await?).map_err(error)?;
        let mut command = Command::new(&configurator);
        command.arg("--metadata");
        let metadata: Value = serde_json::from_slice(&output(command, deadline).await?).map_err(error)?;
        let cid = metadata["guestCid"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|cid| vm_boot::guest_cid(*cid))
            .ok_or_else(|| error("VM configurator omitted its guest CID"))?;
        let write_id =
            metadata["writeDrive"].as_str().ok_or_else(|| error("VM configurator omitted its writable drive ID"))?;
        let boot_agent = entry.spec.agent.as_ref().ok_or_else(|| error("VM agent bootstrap is missing"))?;
        let config = image.configuration.config().as_ref();
        let mut environment = config
            .and_then(|config| config.env().as_ref())
            .into_iter()
            .flatten()
            .filter_map(|value| value.split_once('=').map(|(key, value)| (key.to_owned(), value.to_owned())))
            .collect::<BTreeMap<_, _>>();
        environment.extend(entry.spec.env.iter().cloned());
        let mut agent_args = vec![
            "--sandbox".into(),
            boot_agent.sandbox.clone(),
            "--epoch".into(),
            boot_agent.epoch.to_string(),
            "--data".into(),
            entry.spec.mount_path.clone(),
            "--domain".into(),
            boot_agent.domain.clone(),
            "--port".into(),
            boot_agent.agent_port.to_string(),
            "--secret-env".into(),
            boot_agent.secret_env.clone(),
            "--vm-port-base".into(),
            vm_boot::SERVICE_PORT_BASE.to_string(),
            "--entrypoint-cwd".into(),
            entry
                .spec
                .workdir
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
        let declared = entry
            .spec
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
        let original = entry.spec.entrypoint.clone().unwrap_or_else(|| {
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
            cid,
            layers: ids
                .iter()
                .map(|id| identities.get(id).cloned().ok_or_else(|| error("VM layer identity is missing")))
                .collect::<Result<_, _>>()?,
            writes: identities.get(write_id).cloned().ok_or_else(|| error("VM disk identity is missing"))?,
            hostname: format!("pryx-{}", entry.id),
            uid,
            gid,
            data: entry.spec.mount_path.clone(),
            read_only_root: entry.spec.read_only_rootfs,
            temporary_bytes: u64::from(entry.spec.tmp_size_mb) * 1024 * 1024,
            pids_limit: entry.spec.pids_limit as u64,
            arguments: agent_args,
            environment: environment.into_iter().collect(),
            network_arguments: None,
            network: None,
        };
        let mut read = vec![
            bundle.root().join("kernel/vmlinux"),
            bundle.root().join("bootstrap/bootstrap.erofs"),
            config_path.clone(),
        ];
        read.extend(image.layers);
        let plan = Plan {
            program: bundle.root().join("bin/firecracker"),
            arguments: vec![
                "--api-sock".into(),
                api.display().to_string(),
                "--config-file".into(),
                config_path.display().to_string(),
            ],
            directory: directory.path().into(),
            lock: lease,
            parent: std::process::id(),
            read,
            write: vec![disk.path],
            role: Role::Ordinary,
        };
        let mut monitor = Machine::launch(
            &bundle.root().join("bin/vm-launcher"),
            &plan,
            &channel,
            &serial,
            &boot,
            deadline,
            usize::try_from(u64::from(entry.spec.log_max_size_mb) * 1024 * 1024).map_err(error)?,
        )
        .await?;
        let bindings = entry
            .spec
            .ports
            .iter()
            .flat_map(|port: &PortBinding| {
                port.host_ips
                    .iter()
                    .map(move |ip| super::super::forward::Binding::service(*ip, port.host_port, port.container_port))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let forwarder = Forwarder::start(
            &bundle.root().join("bin/vm-transport"),
            &channel,
            &bindings,
            self.state.settings.ingress_connections,
            deadline,
        )
        .await?;
        monitor.delivered(deadline).await?;
        // Only authenticated health reads retry. No exec, bootstrap, file write or shutdown does.
        loop {
            if monitor.exited().map_err(error)?.is_some() {
                break;
            }
            if agent::raw(
                &entry.spec,
                hyper::Method::GET,
                "/_pryx/health",
                Value::Null,
                crate::runtime::ENGINE_PROBE_TIMEOUT,
            )
            .await
            .is_ok()
            {
                break;
            }
            if Instant::now() >= deadline {
                return Err(RuntimeError::Timeout("VM agent did not become ready".into()));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        self.state.live.lock().unwrap().insert(
            entry.id.clone(),
            Arc::new(tokio::sync::Mutex::new(Live {
                monitor,
                forwarder,
                _directory: directory,
                serial,
                channel,
                nonce,
                shutdown_requested: false,
                killing: false,
            })),
        );
        if let Some(entry) = self.state.entries.lock().unwrap().get_mut(&entry.id) {
            entry.started = Some(crate::clock::now());
            entry.finished = None;
        }
        Ok(())
    }
}
