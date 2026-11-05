// SPDX-License-Identifier: FSL-1.1-ALv2
//! One agentOS VM per sandbox through the pinned client and sidecar. Existing manager epochs,
//! admission, snapshots and transfer remain authoritative. Virtual PIDs never reach host signals.
//!
//! Parts (`wasm/`):
//! - `boot.rs`: verified distribution and host-owned VM configuration.
//! - `launcher.rs`: inherited lifetime lock across sidecar exec.
//! - `power.rs`: durable ownership transitions and agent frontend lifecycle.
//! - `frontend.rs`: owned agent connections and cancellation on disposal.
//! - `host.rs`: the shared authenticated agent router, logs and activity.
//! - `process.rs`: virtual processes, streams, stdin, signals and PTYs.
//! - `network.rs`: bounded streaming ingress to virtual application listeners.

mod boot;
mod frontend;
mod host;
mod launcher;
mod network;
mod power;
mod process;

const VM_RUNNING: u8 = 0;
const VM_CLOSING: u8 = 3;
const VM_DISPOSED: u8 = 1;
const VM_UNCERTAIN: u8 = 2;

use super::*;
use agentos_client::AgentOs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct WasmRuntime {
    state: Arc<State>,
}

struct State {
    root: PathBuf,
    distribution: PathBuf,
    launcher: PathBuf,
    pool: String,
    entries: Mutex<BTreeMap<String, Entry>>,
    live: Mutex<HashMap<String, Arc<host::Live>>>,
    events: tokio::sync::broadcast::Sender<RuntimeEvent>,
    anchor: AgentOs,
    ingress: Arc<tokio::sync::Semaphore>,
    owner: (u32, u32),
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    id: String,
    spec: ContainerSpec,
    pending: bool,
    running: bool,
    paused: bool,
    code: i64,
    started: Option<String>,
    finished: Option<String>,
}

fn error(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Other(error.to_string())
}

/// Entrypoint dispatch for the node-private sidecar launcher symlink.
pub fn dispatch_sidecar() -> Option<std::io::Error> {
    if !launcher::invoked() {
        return None;
    }
    match launcher::execute() {
        Err(error) => Some(error),
        Ok(never) => match never {},
    }
}

impl WasmRuntime {
    pub async fn new(state: &std::path::Path, config: &crate::config::WasmConfig) -> Result<Self, RuntimeError> {
        let distribution = boot::distribution()?;
        boot::verify(&boot::binary(&config.distribution), &distribution.binary_sha256)?;
        for package in &distribution.default_packages {
            boot::verify(&boot::package_path(&config.distribution, &package.name), &package.sha256)?;
        }
        let owner = if rustix::process::geteuid().is_root() {
            (config.worker_uid, config.worker_gid)
        } else {
            (rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw())
        };
        launcher::traverse(state, state, owner).map_err(error)?;
        let root = state.join("wasm-runtime");
        let launcher = launcher::prepare(
            &root,
            &boot::binary(&config.distribution),
            config.launcher_executable.as_deref(),
            owner,
        )?;
        let pool = format!("pryx:{}", root.display());
        // A no-network manager VM keeps the shared sidecar alive between sandbox creations.
        // Its fixed memory cost is included separately from incremental per-VM measurements.
        let anchor = AgentOs::create(boot::options(None, &config.distribution, &launcher, &pool, owner)?)
            .await
            .map_err(error)?;
        let path = root.join("workloads.json");
        let mut entries: BTreeMap<String, Entry> = if path.exists() {
            use std::os::unix::fs::OpenOptionsExt;
            let file =
                std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(&path).map_err(error)?;
            serde_json::from_reader(file).map_err(error)?
        } else {
            BTreeMap::new()
        };
        // The predecessor's lifetime lock must be gone before the anchor can start. Its VMs
        // cannot execute, and the manager's secret-free workload records still own the data.
        // Require a fresh ensure from control; runtime metadata is not a secret environment store.
        entries.clear();
        let runtime = Self {
            state: Arc::new(State {
                root,
                distribution: config.distribution.clone(),
                launcher,
                pool,
                entries: Mutex::new(entries),
                live: Mutex::new(HashMap::new()),
                events: tokio::sync::broadcast::channel(256).0,
                anchor,
                ingress: Arc::new(tokio::sync::Semaphore::new(config.max_ingress_requests)),
                owner,
            }),
        };
        runtime.save()?;
        Ok(runtime)
    }

    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        let ids = self.state.live.lock().unwrap().keys().cloned().collect::<Vec<_>>();
        for id in ids {
            self.dispose(&id, false).await?;
        }
        self.state.anchor.shutdown().await.map_err(error)
    }

    fn save(&self) -> Result<(), RuntimeError> {
        let entries = self.state.entries.lock().unwrap();
        let metadata: BTreeMap<_, _> = entries
            .iter()
            .map(|(id, entry)| {
                let mut metadata = entry.clone();
                metadata.spec.env.clear();
                (id.clone(), metadata)
            })
            .collect();
        crate::durable::write_atomic(
            &self.state.root.join("workloads.json"),
            &serde_json::to_vec(&metadata).map_err(error)?,
            0o600,
        )
        .map_err(error)
    }

    fn entry(&self, name: &str) -> Result<Entry, RuntimeError> {
        self.state
            .entries
            .lock()
            .unwrap()
            .values()
            .find(|entry| entry.id == name || entry.spec.name == name)
            .cloned()
            .ok_or_else(|| RuntimeError::NotFound("Wasm sandbox".into()))
    }

    fn emit(&self, id: &str, action: &str) {
        // No subscribers is normal before reconcile starts; events are hints, never state.
        let _ = self.state.events.send(RuntimeEvent {
            container_id: id.into(),
            action: action.into(),
            exit_code: None,
            at: OffsetDateTime::now_utc(),
        });
    }
}

#[async_trait]
impl ContainerRuntime for WasmRuntime {
    fn isolation(&self) -> &'static str {
        "wasm"
    }
    fn is_wasm(&self) -> bool {
        true
    }
    fn data_path(&self, root: &std::path::Path) -> PathBuf {
        root.join(boot::CONTENT_DIRECTORY)
    }
    fn data_owner(&self, _: &str) -> Result<Option<(u32, u32)>, RuntimeError> {
        Ok(Some(self.state.owner))
    }
    fn supports_agent(&self) -> bool {
        true
    }
    fn supports_network_policy(&self) -> bool {
        true
    }
    async fn info(&self) -> Result<RuntimeInfo, RuntimeError> {
        self.state.anchor.stat("/").await.map_err(|_| RuntimeError::Unavailable("agentOS sidecar".into()))?;
        Ok(RuntimeInfo {
            version: Some(boot::distribution()?.version),
            storage_driver: Some("wasm".into()),
            security_options: vec!["wasm".into(), "v8".into(), "virtual-kernel".into()],
            ..Default::default()
        })
    }
    async fn ensure_network(&self, _: &str, _: &BTreeMap<String, String>) -> Result<NetworkReport, RuntimeError> {
        Ok(NetworkReport { isolated: true })
    }
    async fn ensure_internal_network(&self, _: &str, _: &BTreeMap<String, String>) -> Result<(), RuntimeError> {
        Ok(())
    }
    async fn network_isolated(&self, _: &str) -> Result<Option<bool>, RuntimeError> {
        Ok(Some(true))
    }
    async fn ensure_image(&self, image: &str, _: Duration) -> Result<bool, RuntimeError> {
        if image.is_empty() { Ok(false) } else { Err(error(crate::protocol::WASM_IMAGE_PROBLEM)) }
    }
    async fn create(&self, spec: &ContainerSpec) -> Result<String, RuntimeError> {
        let id = format!("wasm-{}", uuid::Uuid::new_v4());
        launcher::traverse(&spec.data_dir, self.state.root.parent().expect("state directory"), self.state.owner)
            .map_err(error)?;
        let content = spec.data_dir.join(boot::CONTENT_DIRECTORY);
        launcher::worker_directory(&content, self.state.owner).map_err(error)?;
        {
            let mut entries = self.state.entries.lock().unwrap();
            if entries.values().any(|entry| entry.spec.name == spec.name) {
                return Err(RuntimeError::Conflict("Wasm sandbox already exists".into()));
            }
            entries.insert(
                id.clone(),
                Entry {
                    id: id.clone(),
                    spec: spec.clone(),
                    pending: false,
                    running: false,
                    paused: false,
                    code: 0,
                    started: None,
                    finished: None,
                },
            );
        }
        self.save()?;
        Ok(id)
    }
    async fn start(&self, name: &str) -> Result<(), RuntimeError> {
        let own = self.clone();
        let name = name.to_owned();
        // Own launch until completion even when the manager's request deadline expires.
        tokio::spawn(async move { own.launch(&name).await }).await.map_err(error)?
    }
    async fn stop(&self, name: &str, _: &str, _: Duration) -> Result<(), RuntimeError> {
        self.dispose(name, false).await
    }
    async fn kill(&self, name: &str) -> Result<(), RuntimeError> {
        self.dispose(name, false).await
    }
    async fn pause(&self, name: &str) -> Result<(), RuntimeError> {
        self.dispose(name, true).await
    }
    async fn unpause(&self, name: &str) -> Result<(), RuntimeError> {
        self.start(name).await
    }
    async fn disable_restart(&self, _: &str) -> Result<(), RuntimeError> {
        Ok(())
    }
    async fn remove(&self, name: &str) -> Result<(), RuntimeError> {
        let entry = match self.entry(name) {
            Ok(entry) => entry,
            Err(RuntimeError::NotFound(_)) => return Ok(()),
            Err(error) => return Err(error),
        };
        self.dispose(&entry.id, false).await?;
        self.state.entries.lock().unwrap().remove(&entry.id);
        self.save()
    }
    async fn inspect(&self, name: &str) -> Result<Option<ContainerInfo>, RuntimeError> {
        self.inspect_entry(name).await
    }
    async fn list(&self, filters: &[String]) -> Result<Vec<ContainerInfo>, RuntimeError> {
        let ids = self.state.entries.lock().unwrap().keys().cloned().collect::<Vec<_>>();
        let mut result = Vec::new();
        for id in ids {
            if let Some(info) = self.inspect_entry(&id).await?
                && filters.iter().all(|filter| {
                    filter.split_once('=').is_some_and(|(key, value)| info.labels.get(key).is_some_and(|v| v == value))
                })
            {
                result.push(info);
            }
        }
        Ok(result)
    }
    async fn exec(
        &self,
        id: &str,
        command: &[String],
        timeout: Duration,
        max_output: usize,
    ) -> Result<ExecOutput, RuntimeError> {
        let mut stream = self.exec_stream_for(id, command, timeout, max_output).await?;
        let mut result = ExecOutput::default();
        while let Some(event) = futures_util::StreamExt::next(&mut stream).await {
            match event? {
                pryx_agent::process::Event::Output { stream, bytes } => {
                    use base64::Engine;
                    let bytes = base64::engine::general_purpose::STANDARD.decode(bytes).map_err(error)?;
                    let target = if stream == "stderr" { &mut result.stderr } else { &mut result.stdout };
                    target.extend_from_slice(&bytes[..bytes.len().min(max_output.saturating_sub(target.len()))]);
                }
                pryx_agent::process::Event::Truncated => {
                    result.stdout_truncated = true;
                    result.stderr_truncated = true;
                }
                pryx_agent::process::Event::Exit { result: exit } => {
                    result.exit_code = exit.code.map(i64::from);
                    result.timed_out = exit.timed_out;
                    result.killed = exit.killed;
                }
            }
        }
        Ok(result)
    }
    async fn exec_stream(
        &self,
        id: &str,
        command: &[String],
        timeout: Duration,
        max_output: usize,
        output: tokio::sync::mpsc::Sender<LogChunk>,
    ) -> Result<ExecOutput, RuntimeError> {
        let mut stream = self.exec_stream_for(id, command, timeout, max_output).await?;
        let mut result = ExecOutput::default();
        while let Some(event) = futures_util::StreamExt::next(&mut stream).await {
            match event? {
                pryx_agent::process::Event::Output { stream, bytes } => {
                    use base64::Engine;
                    let bytes = base64::engine::general_purpose::STANDARD.decode(bytes).map_err(error)?;
                    if output
                        .send(LogChunk {
                            stream: if stream == "stderr" { LogStream::Stderr } else { LogStream::Stdout },
                            bytes: bytes.into(),
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                pryx_agent::process::Event::Truncated => {
                    result.stdout_truncated = true;
                    result.stderr_truncated = true;
                }
                pryx_agent::process::Event::Exit { result: exit } => {
                    result.exit_code = exit.code.map(i64::from);
                    result.timed_out = exit.timed_out;
                    result.killed = exit.killed;
                }
            }
        }
        Ok(result)
    }
    fn logs(&self, id: &str, options: LogOptions) -> BoxStream<'static, Result<LogChunk, RuntimeError>> {
        self.logs_for(id, options)
    }
    async fn stats(&self, id: &str) -> Result<Option<RawStats>, RuntimeError> {
        let entry = self.entry(id)?;
        let live = self.state.live.lock().unwrap().get(&entry.id).cloned();
        match live {
            Some(live) => Ok(Some(RawStats {
                pids: Some(live.vm.all_processes().await.map_err(error)?.len() as u64),
                ..Default::default()
            })),
            None => Ok(None),
        }
    }
    fn events(&self, _: &[String]) -> BoxStream<'static, Result<RuntimeEvent, RuntimeError>> {
        Box::pin(futures_util::stream::unfold(self.state.events.subscribe(), |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => return Some((Ok(event), receiver)),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        tracing::warn!("Wasm runtime event consumer lagged")
                    }
                }
            }
        }))
    }
}
