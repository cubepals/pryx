// SPDX-License-Identifier: FSL-1.1-ALv2
//! The existing agent router fronts one virtual VM. Signing keys and runtime metadata remain
//! outside its guest environment; ingress, output and activity retain the shared agent contract.

use super::{ContainerSpec, RuntimeError};
use agentos_client::AgentOs;
use pryx_agent::{
    auth::SigningKey,
    context::Agent,
    paths::DataDir,
    process::{Command, Event, Process, Processes, Start},
};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

pub(super) struct Live {
    pub vm: AgentOs,
    pub agent: Arc<Agent>,
    pub entrypoint: Option<Arc<Process>>,
    pub status: Arc<std::sync::atomic::AtomicU8>,
    servers: Mutex<Vec<super::frontend::Frontend>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

pub(super) async fn start(
    vm: AgentOs,
    spec: &ContainerSpec,
    ingress: Arc<tokio::sync::Semaphore>,
    owner: (u32, u32),
) -> Result<Live, RuntimeError> {
    let boot = spec.agent.as_ref().ok_or_else(|| super::error("Wasm requires the agent"))?;
    let secret = spec
        .env
        .iter()
        .find(|(name, _)| name == &boot.secret_env)
        .ok_or_else(|| super::error("Missing wasm signing material"))?;
    let key = SigningKey::from_encoded(&secret.1).map_err(super::error)?;
    let root = spec.data_dir.join(super::boot::CONTENT_DIRECTORY);
    let telemetry = Arc::new(pryx_agent::telemetry::Telemetry::default());
    let logs = Arc::new(pryx_agent::logbook::Logbook::open(&root).map_err(super::error)?);
    let network = Arc::new(super::network::Network::new(vm.clone(), ingress));
    let mut spares = Vec::new();
    for logical in &boot.spare_ports {
        let binding = binding(spec, *logical)?;
        spares.push((binding.host_port, listeners(binding).await?));
    }
    let agent_binding = binding(spec, boot.agent_port)?;
    let ports = pryx_agent::ports::Ports::bind_listeners(
        boot.sandbox.clone(),
        boot.domain.clone(),
        agent_binding.host_port,
        spares,
        Some(DataDir::open_owned(&root, owner).map_err(super::error)?),
        telemetry.clone(),
        Some(network.clone()),
    )
    .await
    .map_err(super::error)?;
    ports.set_public_origin(boot.public_origin.clone());
    let environment = spec.env.iter().filter(|(name, _)| name != &boot.secret_env).cloned().collect();
    let status = Arc::new(std::sync::atomic::AtomicU8::new(super::VM_RUNNING));
    let guest = Arc::new(super::process::Guest::new(vm.clone(), environment, spec.mount_path.clone(), status.clone()));
    let agent = Arc::new(Agent {
        vm_resources: false,
        file_tasks: Default::default(),
        shells: Default::default(),
        public_key: key.public_key(),
        key,
        sandbox: boot.sandbox.clone(),
        epoch: boot.epoch,
        processes: Processes::embedded(root.clone(), guest),
        data: DataDir::open_owned(&root, owner).map_err(super::error)?,
        ports,
        telemetry,
        logs,
    });
    let mut servers = Vec::new();
    for listener in listeners(agent_binding).await? {
        servers.push(super::frontend::Frontend::start(listener, pryx_agent::server::router(agent.clone())));
    }
    for binding in &spec.ports {
        if binding.container_port == boot.agent_port || boot.spare_ports.contains(&binding.container_port) {
            continue;
        }
        for listener in listeners(binding).await? {
            servers.push(super::frontend::Frontend::forward(listener, network.clone(), binding.container_port));
        }
    }
    let mut live =
        Live { vm, agent, entrypoint: None, status, servers: Mutex::new(servers), tasks: Mutex::new(Vec::new()) };
    if !boot.keep_alive
        && let Some(argv) = spec.entrypoint.as_ref().filter(|argv| !argv.is_empty())
    {
        let process = live
            .agent
            .processes
            .start(
                Start {
                    command: Command::Argv(argv.clone()),
                    env: Default::default(),
                    cwd: Some(spec.workdir.clone().unwrap_or_else(|| spec.mount_path.clone())),
                    stdin: false,
                    pty: false,
                    cols: None,
                    rows: None,
                    timeout: None,
                },
                "entrypoint",
                None,
            )
            .map_err(super::error)?;
        live.tasks.lock().unwrap().push(tokio::spawn(logs_task(process.clone(), live.agent.logs.clone())));
        live.entrypoint = Some(process);
    }
    live.tasks.lock().unwrap().push(tokio::spawn(activity_task(live.agent.clone())));
    Ok(live)
}

fn binding(spec: &ContainerSpec, logical: u16) -> Result<&super::PortBinding, RuntimeError> {
    spec.ports.iter().find(|p| p.container_port == logical).ok_or_else(|| super::error("Missing wasm host port"))
}

async fn listeners(binding: &super::PortBinding) -> Result<Vec<tokio::net::TcpListener>, RuntimeError> {
    if binding.host_ips.is_empty() {
        return Err(super::error("Missing wasm host binding address"));
    }
    let mut listeners = Vec::new();
    for ip in &binding.host_ips {
        listeners.push(tokio::net::TcpListener::bind((*ip, binding.host_port)).await.map_err(super::error)?);
    }
    Ok(listeners)
}

async fn logs_task(process: Arc<Process>, logs: Arc<pryx_agent::logbook::Logbook>) {
    let (replay, mut receiver) = process.subscribe().await;
    let mut replay = std::collections::VecDeque::from(replay);
    while let Some(event) = match replay.pop_front() {
        Some(event) => Some(event),
        None => receiver.recv().await,
    } {
        if let Event::Output { stream, bytes } = event
            && logs.record(stream, &bytes).is_err()
        {
            tracing::error!("Cannot retain wasm entrypoint output");
        }
    }
}

async fn activity_task(agent: Arc<Agent>) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        interval.tick().await;
        let written =
            pryx_agent::state::write(&agent.data, pryx_agent::state::ACTIVITY_PATH, &agent.telemetry.snapshot(), false);
        if written.is_err() {
            tracing::warn!("Cannot checkpoint wasm agent activity");
        }
    }
}

impl Live {
    pub(super) async fn shutdown(&self) -> Result<(), RuntimeError> {
        use std::sync::atomic::Ordering;
        self.status.store(super::VM_CLOSING, Ordering::Release);
        let servers = std::mem::take(&mut *self.servers.lock().unwrap());
        for server in servers {
            server.shutdown().await?;
        }
        self.agent.file_tasks.close();
        self.agent.file_tasks.wait().await;
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap());
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            if let Err(error) = task.await
                && !error.is_cancelled()
            {
                tracing::warn!("Wasm agent background task failed");
            }
        }
        self.agent.ports.shutdown().await;
        match self.vm.shutdown().await {
            Ok(()) => {
                self.status.store(super::VM_DISPOSED, Ordering::Release);
                Ok(())
            }
            Err(error) => {
                self.status.store(super::VM_UNCERTAIN, Ordering::Release);
                Err(super::error(error))
            }
        }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.servers.lock().unwrap().clear();
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn listeners_use_only_the_host_selected_addresses() {
        let ip = std::net::Ipv4Addr::new(127, 0, 0, 2);
        let mut binding = super::super::PortBinding {
            container_port: 3000,
            host_port: 0,
            host_ips: vec![ip.into()],
            protocol: crate::protocol::Proto::Tcp,
        };
        let bound = listeners(&binding).await.unwrap();
        assert_eq!(bound.len(), 1);
        let address = bound[0].local_addr().unwrap();
        assert_eq!(address.ip(), ip);
        assert!(tokio::net::TcpStream::connect(address).await.is_ok());
        assert!(tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, address.port())).await.is_err());
        binding.host_ips.clear();
        assert!(listeners(&binding).await.is_err());
    }
}
