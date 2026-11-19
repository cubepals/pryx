// SPDX-License-Identifier: FSL-1.1-ALv2
//! Build private monitor configuration from generated owner models, then boot a confined
//! guest and expose only admitted TCP services. Spec secrets remain in the one-shot bootstrap.
use super::super::image::{Failure, outcome};
use super::*;
use crate::runtime::PortBinding;
use serde_json::Value;
use tokio::time::Instant;

pub(super) const BOOT_TIMEOUT: Duration = Duration::from_secs(30);

/// The manifest digest an image reference names after `@`, as the OCI distribution
/// specification writes it (`algorithm:encoded`); a tag names none.
fn named_digest(image: &str) -> Option<&str> {
    image.rsplit_once('@').map(|(_, digest)| digest).filter(|digest| digest.starts_with("sha256:"))
}

/// A layer file's device, inode, size, and modification and change times.
pub(super) type Identity = (u64, u64, u64, (i64, i64), (i64, i64));

fn identities_of(layers: &[PathBuf]) -> Option<Vec<Identity>> {
    layers
        .iter()
        .map(|layer| {
            let info = fs::symlink_metadata(layer).ok()?;
            info.is_file().then(|| {
                (
                    info.dev(),
                    info.ino(),
                    info.len(),
                    (info.mtime(), info.mtime_nsec()),
                    (info.ctime(), info.ctime_nsec()),
                )
            })
        })
        .collect()
}

/// Milliseconds each stage of one start took, in order, for the node log and tests.
#[derive(Clone, Debug)]
pub(super) struct Stages {
    since: Instant,
    pub(super) taken: Vec<(&'static str, u128)>,
}
impl Default for Stages {
    fn default() -> Self {
        Self { since: Instant::now(), taken: Vec::new() }
    }
}
impl Stages {
    pub(super) fn mark(&mut self, stage: &'static str) {
        let now = Instant::now();
        self.taken.push((stage, now.duration_since(self.since).as_millis()));
        self.since = now;
    }
}
impl std::fmt::Display for Stages {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, (stage, milliseconds)) in self.taken.iter().enumerate() {
            write!(formatter, "{}{stage}={milliseconds}ms", if index == 0 { "" } else { " " })?;
        }
        Ok(())
    }
}
impl VmRuntime {
    pub(super) async fn forward(
        &self,
        bundle: &Bundle,
        channel: &Path,
        entry: &Entry,
        deadline: Instant,
    ) -> Result<Forwarder, RuntimeError> {
        let agent = entry.spec.agent.as_ref().map(|agent| agent.agent_port);
        let bindings = entry
            .spec
            .ports
            .iter()
            .flat_map(|port: &PortBinding| {
                let own = Some(port.container_port) == agent && port.protocol == crate::protocol::Proto::Tcp;
                port.host_ips.iter().map(move |ip| {
                    if own {
                        super::super::forward::Binding::agent(*ip, port.host_port, port.container_port)
                    } else {
                        super::super::forward::Binding::service(*ip, port.host_port, port.container_port)
                    }
                })
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

    pub(super) async fn network(
        &self,
        bundle: &Bundle,
        directory: &Path,
        lease: &Path,
        entry: &Entry,
        deadline: Instant,
    ) -> Result<Option<network::Network>, RuntimeError> {
        if !entry.spec.internal_network
            && let Some(owner) = self.monitor_owner(&entry.id)?
        {
            return network::Network::root(
                &self.state.settings,
                bundle,
                &self.state.root.join("boots"),
                lease,
                owner,
                entry.spec.egress.clone(),
                deadline,
            )
            .await
            .map(Some);
        }
        let socket = directory.join(format!("vsock_{}", pryx_agent::vm_boot::PACKET_PORT));
        let network_lease = lease.with_extension("network.lock");
        let network = if !entry.spec.internal_network {
            Some(network::Network::User(Box::new(
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
            )))
        } else {
            None
        };
        Ok(network)
    }

    /// A preparation that stopped at the host's free-space floor is asked once more, after an
    /// eviction of what no VM here needs.
    pub(super) async fn prepared(
        &self,
        image: &str,
        pinned: Option<&str>,
        timeout: Duration,
    ) -> Result<Prepared, RuntimeError> {
        match self.helper(image, pinned, timeout, false).await? {
            Err(Failure::Room(_)) => {
                self.evict_images(timeout).await?;
                outcome(self.helper(image, pinned, timeout, false).await?)
            }
            answer => outcome(answer),
        }
    }
    /// Removes every converted layer no VM of this runtime needs. No launch registers its image
    /// meanwhile, so what is kept is read whole, and a launch registered after it prepares again.
    pub(super) async fn evict_images(&self, timeout: Duration) -> Result<(), RuntimeError> {
        let _evicting = self.state.evicting.write().await;
        outcome(self.helper("", None, timeout, true).await?).map(drop)
    }
    async fn helper(
        &self,
        image: &str,
        pinned: Option<&str>,
        timeout: Duration,
        evict: bool,
    ) -> Result<super::super::image::Response, RuntimeError> {
        let bundle = self.bundle().await?;
        super::super::image::exchange(
            &bundle.root().join("bin/pryx-vm-image"),
            &super::super::image::Request {
                component: bundle.root().join("filesystem"),
                manifest_sha256: bundle.filesystems().identity().into(),
                launcher: bundle.filesystems().launcher().into(),
                cache: self.state.root.join("images"),
                image: image.into(),
                pinned: pinned.map(String::from),
                timeout_seconds: timeout.as_secs(),
                free_floor_bytes: self.state.settings.free_floor_bytes,
                keep: self.kept_images(),
                evict,
            },
        )
        .await
    }
    /// What some VM of this runtime needs: each one's pinned manifest, a digest its image names,
    /// and each launch's image, by pin or reference, from before its preparation.
    fn kept_images(&self) -> Vec<String> {
        let mut kept: std::collections::BTreeSet<String> = self
            .state
            .entries
            .lock()
            .unwrap()
            .values()
            .flat_map(|entry| {
                entry.image_digest.iter().cloned().chain(named_digest(&entry.spec.image).map(String::from))
            })
            .collect();
        kept.extend(self.state.launching.lock().unwrap().keys().cloned());
        kept.into_iter().collect()
    }
    /// Holds `image` (a pin or a reference) from before its preparation until the launch that
    /// takes it ends; never while an eviction reads what is kept.
    async fn launching(&self, image: &str) -> Launching<'_> {
        let _evicting = self.state.evicting.read().await;
        *self.state.launching.lock().unwrap().entry(image.into()).or_default() += 1;
        Launching { runtime: self, digest: image.into() }
    }
    /// A pinned image this runtime already prepared needs no helper while its layer files are
    /// the ones the helper verified; anything else goes through the helper.
    pub(super) async fn pinned(
        &self,
        image: &str,
        pinned: Option<&str>,
        timeout: Duration,
    ) -> Result<Prepared, RuntimeError> {
        // A reference naming its digest is immutable too; it is kept under the whole reference,
        // since that digest may be an index the helper resolves to one platform's manifest.
        let key = pinned.map(String::from).or_else(|| named_digest(image).map(|_| image.to_owned()));
        if let Some(key) = &key {
            let known = self.state.prepared.lock().unwrap().get(key).cloned();
            if let Some((prepared, identities)) = known
                && identities_of(&prepared.layers).is_some_and(|current| current == identities)
            {
                return Ok(prepared);
            }
        }
        let prepared = self.prepared(image, pinned, timeout).await?;
        if let Some(identities) = identities_of(&prepared.layers) {
            let mut known = self.state.prepared.lock().unwrap();
            for name in std::iter::once(prepared.digest.clone()).chain(key) {
                known.insert(name, (prepared.clone(), identities.clone()));
            }
        }
        Ok(prepared)
    }
    /// True while the previous boot still runs; otherwise its helpers and jail are retired.
    pub(super) async fn running_or_retired(&self, id: &str) -> Result<bool, RuntimeError> {
        let previous = self.state.live.lock().unwrap().get(id).cloned();
        if let Some(previous) = previous {
            let mut previous = previous.lock().await;
            if previous.monitor.exited().map_err(error)?.is_none() {
                return Ok(true);
            }
            previous.forwarder.stop().await.map_err(error)?;
            if let Some(network) = &mut previous.network {
                network.stop().await.map_err(error)?;
            }
            previous.retire_jail()?;
            drop(previous);
            self.state.live.lock().unwrap().remove(id);
        }
        Ok(false)
    }
    pub(super) async fn launch_entry(&self, name: &str) -> Result<(), RuntimeError> {
        let entry = self.entry(name)?;
        let transition = self.transition(&entry.id);
        let _transition = transition.lock().await;
        if self.running_or_retired(&entry.id).await? {
            return Ok(());
        }
        let mut entry = self.entry(name)?;
        // A cold boot changes the disk, so memory taken with it is discarded first.
        self.discard_memory(&mut entry)?;
        let mut stages = Stages::default();
        let (_, _, uid, gid) = monitor::resources(&entry.spec)?;
        // The first start pins the manifest; a wake never takes a moved tag's lower layers
        // under its retained disk, and a cached pin needs no registry.
        let _launching = self.launching(entry.image_digest.as_deref().unwrap_or(&entry.spec.image)).await;
        let image =
            self.pinned(&entry.spec.image, entry.image_digest.as_deref(), self.state.settings.image_timeout).await?;
        entry.image_digest = Some(image.digest.clone());
        stages.mark("image");
        let bundle = self.bundle().await?;
        let lease = self.lease(&entry)?;
        self.recover_disk(&entry.spec.data_dir)?;
        let disk = super::super::disk::prepare_trusted(
            bundle.filesystems(),
            &entry.spec.data_dir,
            &lease,
            entry.spec.storage_bytes,
            BOOT_TIMEOUT,
            self.state.settings.free_floor_bytes,
            entry.clean_disk.take(),
        )
        .await?;
        stages.mark("disk");
        let directory = super::memory::sandbox_directory(&self.state.root.join("boots"), &lease)?;
        let serial = directory.path().join("serial");
        let deadline = Instant::now() + BOOT_TIMEOUT;
        let network = self.network(&bundle, directory.path(), &lease, &entry, deadline).await?;
        stages.mark("network");
        let monitor::Launch { channel, configuration, plan, jail } = self
            .prepare_monitor(
                &bundle,
                &entry,
                &image,
                monitor::Inputs { directory: directory.path(), disk: &disk.path, network: network.as_ref() },
                deadline,
            )
            .await?;
        let mut boot = super::guest::boot(&entry.spec, &entry.id, &image, &configuration, (uid, gid))?;
        boot.network = network.as_ref().map(network::Network::guest).transpose()?;
        let nonce = boot.nonce.clone();
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
        stages.mark("monitor");
        let readiness: Result<Forwarder, RuntimeError> = async {
            let forwarder = self.forward(&bundle, &channel, &entry, deadline).await?;
            stages.mark("forwarder");
            monitor.delivered(deadline).await?;
            stages.mark("bootstrap");
            ready(&mut monitor, &entry.spec, deadline).await?;
            stages.mark("agent");
            Ok(forwarder)
        }
        .await;
        #[cfg(test)]
        if readiness.is_err() {
            diagnostics::failed(&mut monitor, &serial, &entry.spec).await;
        }
        let forwarder = readiness?;
        tracing::info!(sandbox = %entry.id, stages = %stages, "VM boot stages");
        self.state.live.lock().unwrap().insert(
            entry.id.clone(),
            Arc::new(tokio::sync::Mutex::new(Live {
                monitor,
                jail,
                forwarder,
                network,
                _directory: directory,
                serial,
                channel,
                nonce,
                shutdown_requested: false,
                shutdown_exit: None,
                killing: false,
                stages,
                #[cfg(test)]
                diagnostic_secrets: entry.spec.env.iter().map(|(_, value)| value.clone()).collect(),
            })),
        );
        Ok(())
    }
}

pub(super) struct Launching<'a> {
    runtime: &'a VmRuntime,
    /// The image held: a pin or a reference.
    digest: String,
}
impl Drop for Launching<'_> {
    fn drop(&mut self) {
        let mut launching = self.runtime.state.launching.lock().unwrap();
        if let Some(count) = launching.get_mut(&self.digest) {
            *count -= 1;
            if *count == 0 {
                launching.remove(&self.digest);
            }
        }
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

#[cfg(test)]
mod tests {
    #[test]
    fn only_a_digest_reference_pins_itself() {
        let digest = "sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf";
        assert_eq!(super::named_digest(&format!("docker.io/library/alpine@{digest}")), Some(digest));
        assert_eq!(super::named_digest("docker.io/library/alpine:3.24.4"), None);
        assert_eq!(super::named_digest("registry.example:5000/team/app"), None);
    }
}
