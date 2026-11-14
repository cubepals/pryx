// SPDX-License-Identifier: FSL-1.1-ALv2
//! Build private monitor configuration from generated owner models, then boot a confined
//! guest and expose only admitted TCP services. Spec secrets remain in the one-shot bootstrap.
use super::*;
use crate::runtime::PortBinding;
use serde_json::Value;
use tokio::time::Instant;

pub(super) const BOOT_TIMEOUT: Duration = Duration::from_secs(30);
impl VmRuntime {
    async fn forward(
        &self,
        bundle: &Bundle,
        channel: &Path,
        entry: &Entry,
        deadline: Instant,
    ) -> Result<Forwarder, RuntimeError> {
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
        Forwarder::start(
            &bundle.root().join("bin/vm-transport"),
            channel,
            &bindings,
            self.state.settings.ingress_connections,
            deadline,
        )
        .await
    }

    async fn network(
        &self,
        bundle: &Bundle,
        directory: &Path,
        lease: &Path,
        entry: &Entry,
        deadline: Instant,
    ) -> Result<Option<super::super::network::Network>, RuntimeError> {
        let socket = directory.join(format!("vsock_{}", pryx_agent::vm_boot::PACKET_PORT));
        let network_lease = lease.with_extension("network.lock");
        let network = if !entry.spec.internal_network {
            Some(
                super::super::network::Network::start(
                    super::super::network::Paths {
                        launcher: &bundle.root().join("bin/vm-launcher"),
                        program: &bundle.root().join("bin/network-proxy"),
                        boots: &self.state.root.join("boots"),
                        lease: &network_lease,
                        socket: Some(&socket),
                    },
                    entry.spec.egress.clone(),
                    self.state.settings.ingress_connections,
                    deadline,
                )
                .await?,
            )
        } else {
            None
        };
        Ok(network)
    }

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
            if let Some(network) = &mut previous.network {
                network.stop().await.map_err(error)?;
            }
            drop(previous);
            self.state.live.lock().unwrap().remove(&entry.id);
        }
        let mut entry = self.entry(name)?;
        let (memory, cpus, uid, gid) = monitor::resources(&entry.spec)?;
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
        let network = self.network(&bundle, directory.path(), &lease, &entry, deadline).await?;
        let configuration = super::configuration::configure(
            &bundle,
            &image,
            &disk.path,
            &channel,
            super::configuration::Destination { output: &config_path, jail: None },
            (memory, cpus),
            deadline,
        )
        .await?;
        let mut boot = super::guest::boot(&entry.spec, &entry.id, &image, &configuration, (uid, gid))?;
        boot.network = network.as_ref().map(super::super::network::Network::guest).transpose()?;
        let nonce = boot.nonce.clone();
        let plan = monitor::plan(&bundle, image.layers, disk.path, directory.path(), lease, &api, &config_path);
        entry.started = Some(crate::clock::now());
        entry.finished = None;
        entry.code = UNKNOWN_EXIT_CODE;
        self.save_entry(entry.clone())?;
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
        let forwarder = self.forward(&bundle, &channel, &entry, deadline).await?;
        monitor.delivered(deadline).await?;
        ready(&mut monitor, &entry.spec, deadline).await?;

        self.state.live.lock().unwrap().insert(
            entry.id.clone(),
            Arc::new(tokio::sync::Mutex::new(Live {
                monitor,
                forwarder,
                network,
                _directory: directory,
                serial,
                channel,
                nonce,
                shutdown_requested: false,
                killing: false,
                #[cfg(test)]
                diagnostic_secrets: entry.spec.env.iter().map(|(_, value)| value.clone()).collect(),
            })),
        );
        Ok(())
    }
}

pub(super) async fn ready(monitor: &mut Machine, spec: &ContainerSpec, deadline: Instant) -> Result<(), RuntimeError> {
    // Only authenticated health reads retry. No exec, bootstrap, file write or shutdown does.
    loop {
        if monitor.exited().map_err(error)?.is_some() {
            break;
        }
        if agent::raw(spec, hyper::Method::GET, "/_pryx/health", Value::Null, crate::runtime::ENGINE_PROBE_TIMEOUT)
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
    Ok(())
}
