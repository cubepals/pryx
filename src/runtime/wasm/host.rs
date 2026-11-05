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
    server: Mutex<Option<super::frontend::Frontend>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

pub(super) async fn start(
    vm: AgentOs,
    spec: &ContainerSpec,
    ingress: Arc<tokio::sync::Semaphore>,
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
    let port = |logical| {
        spec.ports
            .iter()
            .find(|p| p.container_port == logical)
            .map(|p| p.host_port)
            .ok_or_else(|| super::error("Missing wasm host port"))
    };
    let ports = pryx_agent::ports::Ports::bind_with_forward(
        boot.sandbox.clone(),
        boot.domain.clone(),
        port(boot.agent_port)?,
        boot.spare_ports.iter().map(|p| port(*p)).collect::<Result<_, _>>()?,
        Some(DataDir::open(&root).map_err(super::error)?),
        telemetry.clone(),
        Some(Arc::new(super::network::Network::new(vm.clone(), ingress))),
    )
    .await
    .map_err(super::error)?;
    ports.set_public_origin(boot.public_origin.clone());
    let environment = spec.env.iter().filter(|(name, _)| name != &boot.secret_env).cloned().collect();
    let status = Arc::new(std::sync::atomic::AtomicU8::new(super::VM_RUNNING));
    let guest = Arc::new(super::process::Guest::new(vm.clone(), environment, spec.mount_path.clone(), status.clone()));
    let agent = Arc::new(Agent {
        public_key: key.public_key(),
        key,
        sandbox: boot.sandbox.clone(),
        epoch: boot.epoch,
        processes: Processes::embedded(root.clone(), guest),
        data: DataDir::open(&root).map_err(super::error)?,
        ports,
        telemetry,
        logs,
    });
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port(boot.agent_port)?))
        .await
        .map_err(super::error)?;
    let server = super::frontend::Frontend::start(listener, pryx_agent::server::router(agent.clone()));
    let mut live =
        Live { vm, agent, entrypoint: None, status, server: Mutex::new(Some(server)), tasks: Mutex::new(Vec::new()) };
    if let Some(argv) = spec.entrypoint.as_ref().filter(|argv| !argv.is_empty()) {
        let process = live
            .agent
            .processes
            .start(
                Start {
                    command: Command::Argv(argv.clone()),
                    env: Default::default(),
                    cwd: Some(spec.mount_path.clone()),
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
    live.tasks.lock().unwrap().push(tokio::spawn(activity_task(live.agent.clone(), root)));
    Ok(live)
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

async fn activity_task(agent: Arc<Agent>, root: std::path::PathBuf) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        interval.tick().await;
        let written = DataDir::open(&root).and_then(|data| {
            pryx_agent::state::write(&data, pryx_agent::state::ACTIVITY_PATH, &agent.telemetry.snapshot(), false)
        });
        if written.is_err() {
            tracing::warn!("Cannot checkpoint wasm agent activity");
        }
    }
}

impl Live {
    pub(super) async fn shutdown(&self) -> Result<(), RuntimeError> {
        use std::sync::atomic::Ordering;
        self.status.store(super::VM_CLOSING, Ordering::Release);
        let server = self.server.lock().unwrap().take();
        if let Some(server) = server {
            server.shutdown().await?;
        }
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
        self.server.lock().unwrap().take();
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}
