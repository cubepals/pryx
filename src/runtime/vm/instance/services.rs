// SPDX-License-Identifier: FSL-1.1-ALv2
//! File access reuses a running agent or boots a trusted maintenance guest for stopped or
//! retained data. It never starts workload compute, pulls an image or adds a runtime journal.
use super::*;
use crate::runtime::{
    AgentBoot, PortBinding,
    files::{Request, Service},
};
use std::net::SocketAddr;

pub(super) fn specification(request: Request, binary: PathBuf) -> ContainerSpec {
    ContainerSpec {
        name: request.name,
        agent: Some(AgentBoot {
            binary,
            sandbox: request.sandbox,
            epoch: request.epoch,
            secret_env: format!("{}{}", pryx_agent::contract::SECRET_ENV_PREFIX, uuid::Uuid::new_v4().simple()),
            domain: "invalid".into(),
            agent_port: pryx_agent::contract::AGENT_PORT,
            public_origin: None,
            spare_ports: Vec::new(),
            keep_alive: true,
        }),
        env: Vec::new(),
        labels: BTreeMap::new(),
        cgroup_parent: None,
        image: String::new(),
        packages: Vec::new(),
        entrypoint: Some(Vec::new()),
        workdir: Some("/".into()),
        egress: None,
        user: request.user,
        memory_bytes: request.memory_bytes,
        storage_bytes: request.storage_bytes,
        cpu_weight_requested: false,
        allow_unenforced_memory: None,
        require_cpu_limit: true,
        require_pids_limit: true,
        require_browser_sandbox: None,
        nano_cpus: None,
        cpu_shares: 0,
        pids_limit: request.pids_limit,
        read_only_rootfs: true,
        tmp_size_mb: request.tmp_size_mb,
        data_dir: request.data_dir,
        mount_path: request.mount_path,
        ports: Vec::new(),
        stop_signal: crate::protocol::StopSignal::SIGTERM.as_str().into(),
        stop_timeout_secs: 0,
        network: String::new(),
        internal_network: true,
        log_max_size_mb: request.log_max_size_mb,
        log_max_files: 1,
        oom_score_adj: 0,
    }
}
fn same_data(entry: &Entry, request: &Request) -> bool {
    entry.spec.agent.as_ref().is_some_and(|agent| agent.sandbox == request.sandbox && agent.epoch == request.epoch)
        && entry.spec.data_dir == request.data_dir
        && entry.spec.mount_path == request.mount_path
}
fn endpoint(spec: &ContainerSpec) -> Result<SocketAddr, RuntimeError> {
    let agent = spec.agent.as_ref().ok_or_else(|| error("VM file service identity is missing"))?;
    let port = spec
        .ports
        .iter()
        .find(|port: &&PortBinding| {
            port.container_port == agent.agent_port && port.protocol == crate::protocol::Proto::Tcp
        })
        .ok_or_else(|| error("VM agent binding is missing"))?;
    Ok(SocketAddr::new(*port.host_ips.first().ok_or_else(|| error("VM agent address is missing"))?, port.host_port))
}
impl VmRuntime {
    pub(super) async fn serve_files(&self, request: Request) -> Result<Service, RuntimeError> {
        // Retire a spontaneously dead monitor before choosing its existing agent or disk.
        let _ = self.observe(&request.name).await?;
        let existing = match self.entry(&request.name) {
            Ok(entry) => Some(entry),
            Err(RuntimeError::NotFound(_)) => None,
            Err(problem) => return Err(problem),
        };
        let id = existing
            .as_ref()
            .map(|entry| entry.id.clone())
            .unwrap_or_else(|| format!("retained-files:{}", request.sandbox));
        let transition = self.transition(&id).lock_owned().await;
        let existing = match existing {
            Some(entry) => Some(self.entry(&entry.id)?),
            None if self.entry(&request.name).is_ok() => {
                return Err(RuntimeError::Conflict("VM appeared while file access waited".into()));
            }
            None => None,
        };
        if let Some(entry) = &existing {
            if !same_data(entry, &request) {
                return Err(RuntimeError::Conflict("VM data identity differs from the node record".into()));
            }
            let live = self.state.live.lock().unwrap().get(&entry.id).cloned();
            if let Some(live) = live {
                let mut live = live.lock().await;
                if live.monitor.exited().map_err(error)?.is_some() || live.forwarder.exited().map_err(error)?.is_some()
                {
                    return Err(RuntimeError::Unavailable(
                        "VM file forwarding exited during access preparation".into(),
                    ));
                }
                drop(live);
                let agent = entry.spec.agent.as_ref().ok_or_else(|| error("VM file identity is missing"))?;
                return Service::new(
                    endpoint(&entry.spec)?,
                    agent::key(&entry.spec)?,
                    (agent.sandbox.clone(), agent.epoch),
                    launch::BOOT_TIMEOUT,
                    async move {
                        drop(transition);
                        Ok(())
                    },
                );
            }
        }
        let bundle = self.bundle().await?;
        let entry = Entry {
            id: existing.map(|entry| entry.id).unwrap_or_else(|| format!("vm-{}", uuid::Uuid::new_v4().simple())),
            spec: specification(request, bundle.root().join("bin/vm-launcher")),
            started: None,
            finished: None,
            code: 0,
            agent_state: None,
        };
        self.launch_files(entry, transition).await?.into_service()
    }
}
