// SPDX-License-Identifier: FSL-1.1-ALv2
//! Opt-in external VM test support. Give the existing production launch::ready
//! the current guest's real private forwarding address and in-memory signing key.
use super::super::{error, forward::Binding};
use super::{ContainerSpec, Forwarder, Machine, RuntimeError, launch};
use crate::protocol::Proto;
use crate::runtime::{AgentBoot, PortBinding};
use pryx_agent::contract::AGENT_PORT;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use tokio::time::Instant;

pub const RETRY: std::time::Duration = launch::HEALTH_RETRY;

pub struct Plan(ContainerSpec);

impl Plan {
    /// The external fixture launches the monitor itself. Only the guest identity
    /// and private binding are consumed by launch::ready and its agent::raw.
    pub fn new(sandbox: &str, epoch: u64, secret: &str, encoded: &str) -> Self {
        let agent = AgentBoot {
            public_origin: None,
            binary: PathBuf::new(),
            sandbox: sandbox.into(),
            epoch,
            secret_env: secret.into(),
            domain: "vm.pryx.invalid".into(),
            agent_port: AGENT_PORT,
            spare_ports: Vec::new(),
            keep_alive: true,
        };
        Self(ContainerSpec {
            agent: Some(agent),
            name: sandbox.into(),
            cgroup_parent: None,
            image: String::new(),
            packages: Vec::new(),
            entrypoint: None,
            workdir: None,
            egress: None,
            egress_addresses: None,
            env: vec![(secret.into(), encoded.into())],
            labels: Default::default(),
            user: String::new(),
            memory_bytes: 0,
            storage_bytes: 0,
            cpu_weight_requested: false,
            allow_unenforced_memory: None,
            require_cpu_limit: false,
            require_pids_limit: false,
            require_browser_sandbox: None,
            nano_cpus: None,
            cpu_shares: 0,
            pids_limit: 0,
            read_only_rootfs: true,
            tmp_size_mb: 0,
            data_dir: PathBuf::new(),
            mount_path: "/data".into(),
            ports: vec![PortBinding {
                container_port: AGENT_PORT,
                protocol: Proto::Tcp,
                host_ips: vec![Ipv4Addr::LOCALHOST.into()],
                host_port: 0,
            }],
            stop_signal: String::new(),
            stop_timeout_secs: 0,
            network: String::new(),
            internal_network: true,
            log_max_size_mb: 0,
            log_max_files: 0,
            oom_score_adj: 0,
        })
    }

    pub async fn forward(self, binary: &Path, channel: &Path, deadline: Instant) -> Result<Health, RuntimeError> {
        let binding = Binding::agent(Ipv4Addr::LOCALHOST.into(), 0, AGENT_PORT)?;
        let forwarder = Forwarder::start(binary, channel, &[binding], super::INGRESS_CONNECTIONS, deadline).await?;
        let address =
            forwarder.addresses().first().copied().ok_or_else(|| error("Missing lab agent forwarding address"))?;
        let mut spec = self.0;
        spec.ports[0].host_port = address.port();
        Ok(Health { forwarder, spec, address })
    }
}

pub struct Health {
    forwarder: Forwarder,
    spec: ContainerSpec,
    address: SocketAddr,
}

impl Health {
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub async fn ready(&self, monitor: &mut Machine, deadline: Instant) -> Result<(), RuntimeError> {
        launch::ready(monitor, &self.spec, deadline).await
    }
    pub async fn stop(mut self) -> Result<(), RuntimeError> {
        self.forwarder.stop().await.map_err(error)
    }
}
