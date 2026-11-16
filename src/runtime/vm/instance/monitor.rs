// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Prepare ordinary or jailed monitor paths from host policy and verified backing files.
//! Disk ownership recovery requires actual lease retirement; this never grants host authority,
//! implements root networking, registers placement or accepts workload-selected monitor IDs.
use super::*;
use crate::runtime::Proto;
use crate::runtime::vm::{disk, jail};
use pryx_agent::vm_monitor::{Plan, Role};
use std::os::unix::fs::PermissionsExt;
use tokio::time::Instant;

pub(super) struct Launch {
    pub channel: PathBuf,
    pub configuration: configuration::Configuration,
    pub plan: Plan,
    pub jail: Option<jail::Jail>,
}

pub(super) struct Inputs<'a> {
    pub directory: &'a Path,
    pub disk: &'a Path,
    pub network: Option<&'a network::Network>,
}

pub(super) fn owner(owner: Option<monitor_ids::MonitorIds>) -> Result<(), RuntimeError> {
    match (rustix::process::geteuid().is_root(), owner) {
        (false, None) => Ok(()),
        (true, Some(ids)) if ids.valid() => Ok(()),
        _ => Err(RuntimeError::Unavailable(
            "Root VM launch requires a range of non-root host monitor IDs; ordinary users cannot select them".into(),
        )),
    }
}

impl VmRuntime {
    pub(super) fn recover_disk(&self, data: &Path) -> Result<(), RuntimeError> {
        if self.state.settings.monitor_owner.is_some() {
            jail::Jail::recover_disk(&self.state.root.join("jails"), &data.join(disk::FILE))?;
        }
        Ok(())
    }

    pub(super) async fn prepare_monitor(
        &self,
        bundle: &Bundle,
        entry: &Entry,
        image: &Prepared,
        inputs: Inputs<'_>,
        deadline: Instant,
    ) -> Result<Launch, RuntimeError> {
        let Inputs { directory, disk, network } = inputs;
        let (memory, cpus, _, _) = resources(&entry.spec)?;
        let lease = self.lease(entry)?;
        let output = directory.join("monitor.json");
        if let Some(owner) = self.monitor_owner(&entry.id)? {
            if !entry.spec.internal_network && network.is_none() {
                return Err(RuntimeError::Unavailable("Root VM TAP/firewall integration is not available".into()));
            }
            let id = entry.id.strip_prefix("vm-").ok_or_else(|| error("Invalid root VM identity"))?;
            let monitor = bundle.root().join("bin/firecracker");
            let base = self.state.root.join("jails");
            let mut readonly = BTreeMap::from([
                (PathBuf::from("kernel/vmlinux"), bundle.root().join("kernel/vmlinux")),
                (PathBuf::from("bootstrap/bootstrap.erofs"), bundle.root().join("bootstrap/bootstrap.erofs")),
            ]);
            for (index, layer) in image.layers.iter().enumerate() {
                readonly.insert(PathBuf::from(format!("layers/{index}")), layer.clone());
            }
            let mut jail = jail::Jail::prepare(jail::Request {
                base: &base,
                id,
                monitor: &monitor,
                disk,
                lease: &lease,
                owner,
                readonly: &readonly,
            })?;
            let root = jail.root().to_owned();
            let staged = Prepared {
                digest: image.digest.clone(),
                configuration: image.configuration.clone(),
                layers: image
                    .layers
                    .iter()
                    .enumerate()
                    .map(|(index, _)| root.join(format!("layers/{index}")))
                    .collect(),
            };
            let channel = root.join("vsock");
            let configuration = configuration::configure(
                bundle,
                &staged,
                &root.join(disk::FILE),
                &channel,
                configuration::Destination {
                    output: &output,
                    jail: Some(&root),
                    tap: network.map(network::Network::tap).transpose()?.flatten(),
                    working: None,
                },
                (memory, cpus),
                deadline,
            )
            .await?;
            fs::set_permissions(&output, fs::Permissions::from_mode(0o444)).map_err(error)?;
            jail.add_readonly(Path::new("monitor.json"), &output)?;
            let plan = Plan {
                program: bundle.root().join("bin/jailer"),
                arguments: vec!["--api-sock".into(), "/api".into(), "--config-file".into(), "/monitor.json".into()],
                directory: directory.into(),
                lock: lease,
                parent: std::process::id(),
                read: Vec::new(),
                write: Vec::new(),
                role: Role::Jailer {
                    uid: owner.0,
                    gid: owner.1,
                    id: id.into(),
                    base,
                    monitor,
                    namespace: network.map(network::Network::namespace).transpose()?.flatten(),
                },
            };
            Ok(Launch { channel, configuration, plan, jail: Some(jail) })
        } else {
            // Relative paths, so a snapshot names this boot directory's link and socket, and a
            // restore (a fork's included) resolves them in its own sandbox's directory.
            let channel = directory.join(configuration::CHANNEL);
            link_disk(directory, disk)?;
            let configuration = configuration::configure(
                bundle,
                image,
                Path::new(configuration::DISK_LINK),
                Path::new(configuration::CHANNEL),
                configuration::Destination { output: &output, jail: None, tap: None, working: Some(directory) },
                (memory, cpus),
                deadline,
            )
            .await?;
            let plan =
                plan(bundle, image.layers.clone(), disk.into(), directory, lease, &directory.join("api"), &output);
            Ok(Launch { channel, configuration, plan, jail: None })
        }
    }
}

/// The boot directory's link to the sandbox's disk, which the monitor's configuration names.
pub(super) fn link_disk(directory: &Path, disk: &Path) -> Result<(), RuntimeError> {
    std::os::unix::fs::symlink(disk, directory.join(configuration::DISK_LINK)).map_err(error)
}

impl Live {
    pub(super) fn retire_jail(&mut self) -> Result<(), RuntimeError> {
        if let Some(jail) = &mut self.jail {
            jail.retire()?;
        }
        Ok(())
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_monitor_ids_cannot_be_selected_by_an_ordinary_caller() {
        let ids = |uid, gid| Some(monitor_ids::MonitorIds { uid, gid, count: 8 });
        assert!(owner(ids(rustix::process::Uid::ROOT.as_raw(), rustix::process::Gid::ROOT.as_raw())).is_err());
        assert!(owner(ids(u32::MAX, u32::MAX)).is_err());
        let root = rustix::process::geteuid().is_root();
        let uid = rustix::process::geteuid().as_raw().checked_add(1).unwrap();
        let gid = rustix::process::getegid().as_raw().checked_add(1).unwrap();
        assert_eq!(owner(ids(uid, gid)).is_ok(), root);
        assert_eq!(owner(None).is_ok(), !root);
    }
}
