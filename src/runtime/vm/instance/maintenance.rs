// SPDX-License-Identifier: FSL-1.1-ALv2
//! A stopped disk is served by a trusted file-only guest with fresh in-memory credentials.
//! The transition lock spans its lifetime; owned cleanup requests one signed shutdown and
//! keeps the monitor lease authoritative until actual exit, including caller cancellation.
use super::*;
use crate::runtime::PortBinding;
use hyper::Method;
use pryx_agent::{
    auth::{Claims, Scope, SigningKey, TOKEN_LIFETIME_SECONDS},
    vm_shutdown,
};
use std::net::Ipv4Addr;
use tokio::{sync::OwnedMutexGuard, time::Instant};

pub struct FileGuest {
    inner: Option<Inner>,
}
struct Inner {
    entry: Entry,
    live: Live,
    bundle: Arc<Bundle>,
    _transition: OwnedMutexGuard<()>,
}

pub(super) fn specification(original: &ContainerSpec) -> Result<ContainerSpec, RuntimeError> {
    let mut spec = original.clone();
    let boot = spec.agent.as_mut().ok_or_else(|| error("VM file service requires agent identity"))?;
    let (_, encoded) = SigningKey::generate().map_err(error)?;
    spec.env = vec![(boot.secret_env.clone(), encoded)];
    boot.spare_ports.clear();
    boot.keep_alive = true;
    boot.public_origin = None;
    spec.entrypoint = Some(Vec::new());
    spec.workdir = Some("/".into());
    spec.egress = None;
    spec.internal_network = true;
    spec.read_only_rootfs = true;
    spec.packages.clear();
    spec.ports = vec![PortBinding {
        container_port: boot.agent_port,
        host_port: 0,
        host_ips: vec![Ipv4Addr::LOCALHOST.into()],
        protocol: crate::protocol::Proto::Tcp,
    }];
    Ok(spec)
}
impl VmRuntime {
    /// No workload is started, and no live guest may share this disk.
    pub async fn open_files(&self, name: &str) -> Result<FileGuest, RuntimeError> {
        let (own, name) = (self.clone(), name.to_owned());
        tokio::spawn(async move { own.file_guest(&name).await }).await.map_err(error)?
    }
    async fn file_guest(&self, name: &str) -> Result<FileGuest, RuntimeError> {
        let entry = self.entry(name)?;
        let transition = self.transition(&entry.id).lock_owned().await;
        let mut entry = self.entry(&entry.id)?;
        if self.state.live.lock().unwrap().contains_key(&entry.id) {
            return Err(RuntimeError::Conflict("A running VM must serve files through its existing agent".into()));
        }
        entry.spec = specification(&entry.spec)?;
        let (memory, cpus, uid, gid) = launch::resources(&entry.spec)?;
        let bundle = self.bundle().await?;
        let lease = self.lease(&entry)?;
        let disk = super::super::disk::prepare(
            bundle.filesystems(),
            &entry.spec.data_dir,
            &lease,
            entry.spec.storage_bytes,
            launch::BOOT_TIMEOUT,
            self.state.settings.free_floor_bytes,
        )
        .await?;
        let directory = boot_directory(&self.state.root.join("boots"))?;
        let channel = directory.path().join("vsock");
        let serial = directory.path().join("serial");
        let config_path = directory.path().join("monitor.json");
        let api = directory.path().join("api");
        let deadline = Instant::now() + launch::BOOT_TIMEOUT;
        let image = Prepared { digest: String::new(), layers: Vec::new(), configuration: Default::default() };
        let configuration =
            configuration::configure(&bundle, &image, &disk.path, &channel, &config_path, (memory, cpus), deadline)
                .await?;
        let mut boot = guest::boot(&entry.spec, &entry.id, &image, &configuration, (uid, gid))?;
        boot.arguments.push(format!("--{}", pryx_agent::bootstrap_capabilities::VM_FILES_ARGUMENT));
        let nonce = boot.nonce.clone();
        let plan = launch::plan(&bundle, Vec::new(), disk.path, directory.path(), lease, &api, &config_path);
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
        let forwarder = Forwarder::start(
            &bundle.root().join("bin/vm-transport"),
            &channel,
            &[super::super::forward::Binding::service(
                Ipv4Addr::LOCALHOST.into(),
                0,
                entry.spec.ports[0].container_port,
            )?],
            self.state.settings.ingress_connections,
            deadline,
        )
        .await?;
        entry.spec.ports[0].host_port = forwarder.addresses()[0].port();
        let readiness = async {
            monitor.delivered(deadline).await?;
            launch::ready(&mut monitor, &entry.spec, deadline).await?;
            monitor.bootstrapped(deadline).await
        }
        .await;
        #[cfg(test)]
        if readiness.is_err() {
            diagnostics::failed(&mut monitor, &serial, &entry.spec).await;
        }
        readiness?;
        #[cfg(test)]
        let diagnostic_secrets = entry.spec.env.iter().map(|(_, value)| value.clone()).collect();
        Ok(FileGuest {
            inner: Some(Inner {
                entry,
                bundle,
                _transition: transition,
                live: Live {
                    monitor,
                    forwarder,
                    _directory: directory,
                    serial,
                    channel,
                    nonce,
                    shutdown_requested: false,
                    killing: false,
                    #[cfg(test)]
                    diagnostic_secrets,
                },
            }),
        })
    }
}
impl FileGuest {
    pub fn into_service(self) -> Result<crate::runtime::files::Service, RuntimeError> {
        let inner = self.inner.as_ref().ok_or_else(|| error("VM file service is closed"))?;
        let boot = inner.entry.spec.agent.as_ref().ok_or_else(|| error("VM file service identity is missing"))?;
        crate::runtime::files::Service::new(
            inner.live.forwarder.addresses()[0],
            agent::key(&inner.entry.spec)?,
            (boot.sandbox.clone(), boot.epoch),
            launch::BOOT_TIMEOUT,
            self.close(),
        )
    }
    /// Each file mutation is submitted once. The caller owns stream limits and admission.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: crate::http_client::Body,
    ) -> Result<hyper::Response<hyper::body::Incoming>, RuntimeError> {
        let inner = self.inner.as_ref().ok_or_else(|| error("VM file service is closed"))?;
        let spec = &inner.entry.spec;
        let boot = spec.agent.as_ref().ok_or_else(|| error("VM file service identity is missing"))?;
        let now = crate::clock::now().unix_timestamp();
        let token = agent::key(spec)?
            .sign(&Claims {
                sandbox: boot.sandbox.clone(),
                epoch: boot.epoch,
                issued_at: now,
                expires_at: now + TOKEN_LIFETIME_SECONDS,
                scope: Scope::Sandbox,
                max_ports: 0,
                max_exec_seconds: None,
                file: None,
            })
            .map_err(error)?;
        let endpoint = inner.live.forwarder.addresses()[0];
        crate::http_client::send(
            method,
            &format!("http://{endpoint}{path}"),
            &[("authorization", format!("Bearer {token}"))],
            body,
            None,
            launch::BOOT_TIMEOUT,
        )
        .await
        .map_err(error)
    }
    pub async fn close(mut self) -> Result<(), RuntimeError> {
        let inner = self.inner.take().ok_or_else(|| error("VM file service is closed"))?;
        tokio::spawn(inner.close()).await.map_err(error)?
    }
}
impl Inner {
    async fn close(mut self) -> Result<(), RuntimeError> {
        let deadline = Instant::now() + launch::BOOT_TIMEOUT;
        let boot = self.entry.spec.agent.as_ref().ok_or_else(|| error("VM file service identity is missing"))?;
        let request = vm_shutdown::request(&agent::key(&self.entry.spec)?, &self.live.nonce, &boot.sandbox, boot.epoch);
        let _ = super::super::transport::request(
            &self.bundle.root().join("bin/vm-transport"),
            &serde_json::json!({"socket":self.live.channel,"port":vm_shutdown::PORT,"shutdown":request}),
            deadline.saturating_duration_since(Instant::now()),
            pryx_agent::vm_boot::MAX_BYTES,
        )
        .await;
        let exited = tokio::time::timeout_at(deadline, self.live.monitor.wait()).await;
        let result = match exited {
            Ok(status) => status.map(|_| ()).map_err(error),
            Err(_) => {
                self.live.monitor.kill().await.map_err(error)?;
                Err(RuntimeError::Timeout("VM file service did not stop cleanly".into()))
            }
        };
        self.live.forwarder.stop().await.map_err(error)?;
        tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, self.live.monitor.finish_serial())
            .await
            .map_err(|_| RuntimeError::Timeout("VM file service diagnostics did not finish".into()))?
            .map_err(error)?;
        result?;
        runtime::verified_exit(&self.entry, &self.live).map(|_| ())
    }
}
impl Drop for FileGuest {
    fn drop(&mut self) {
        // Without a runtime, dropping Inner still kills its owned child. The kernel
        // lease fences subsequent boots until actual exit; next access checks ext4.
        if let Some(inner) = self.inner.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                if let Err(problem) = inner.close().await {
                    tracing::warn!(%problem, "VM file service cleanup failed; next access must verify its disk");
                }
            });
        }
    }
}
