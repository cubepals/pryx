// SPDX-License-Identifier: FSL-1.1-ALv2
//! A sandbox supervisor without an engine, cgroup or new namespace. Private runtime journals
//! allow adoption after daemon restart; process identities include kernel start ticks, not PID
//! alone. The manager retains the existing epoch, snapshot and data-directory lifecycle.
//!
//! Parts (`process/`):
//! - `journal.rs`: private specifications, UID allocation and process identity checks.
//! - `power.rs`: agent launch, tree signals and process observations.
//! - `exec.rs`: the legacy node exec transport through the same authenticated agent.
//! - `observe.rs`: read-only kernel process identity, ownership and tree observations.
//! - `logs.rs`: bounded, safe entrypoint log replay and following.
//! - `termination.rs`: stable tree freezing and orphan retirement.
mod exec;
mod journal;
mod logs;
mod observe;
mod power;
mod termination;
use super::*;
use crate::config::ProcessConfig;
use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Mutex};
#[derive(Clone)]
pub struct ProcessRuntime {
    state: Arc<State>,
    policy: ProcessConfig,
}
struct State {
    pub root: PathBuf,
    pub entries: Mutex<BTreeMap<String, Entry>>,
    pub children: Mutex<HashMap<String, std::process::Child>>,
    pub owners: Mutex<BTreeMap<String, (u32, u32)>>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    pub id: String,
    pub spec: ContainerSpec,
    pub pid: Option<u32>,
    pub ticks: Option<u64>,
    pub code: i64,
    pub started: Option<String>,
    pub finished: Option<String>,
    #[serde(default)]
    pub cleaned: bool,
}

use std::sync::Arc;

#[async_trait]
impl ContainerRuntime for ProcessRuntime {
    fn is_process(&self) -> bool {
        true
    }
    fn data_owner(&self, id: &str) -> Result<Option<(u32, u32)>, RuntimeError> {
        self.owner(id).map(Some)
    }
    fn data_deleted(&self, id: &str) -> Result<(), RuntimeError> {
        self.release_owner(id)
    }
    fn supports_agent(&self) -> bool {
        true
    }
    fn supports_network_policy(&self) -> bool {
        true
    }
    async fn info(&self) -> Result<RuntimeInfo, RuntimeError> {
        if !pryx_agent::process_capabilities::probe().filesystem {
            return Err(RuntimeError::Unavailable(pryx_agent::process_capabilities::FILESYSTEM_UNAVAILABLE.into()));
        }
        Ok(RuntimeInfo {
            version: Some(env!("CARGO_PKG_VERSION").into()),
            storage_driver: Some("process".into()),
            security_options: vec![
                "landlock".into(),
                "seccomp".into(),
                if self.shared_uid() { "shared-uid" } else { "uid-pool" }.into(),
                "sampled-rss".into(),
                "network-seccomp".into(),
            ],
            ..Default::default()
        })
    }
    async fn ensure_network(&self, _: &str, _: &BTreeMap<String, String>) -> Result<NetworkReport, RuntimeError> {
        Ok(NetworkReport { isolated: false })
    }
    async fn network_isolated(&self, _: &str) -> Result<Option<bool>, RuntimeError> {
        Ok(Some(false))
    }
    async fn ensure_internal_network(&self, _: &str, _: &BTreeMap<String, String>) -> Result<(), RuntimeError> {
        Ok(())
    }
    async fn ensure_image(&self, image: &str, _: Duration) -> Result<bool, RuntimeError> {
        if image.is_empty() {
            Ok(false)
        } else {
            Err(RuntimeError::Other(crate::protocol::PROCESS_IMAGE_PROBLEM.into()))
        }
    }
    async fn create(&self, spec: &ContainerSpec) -> Result<String, RuntimeError> {
        self.create_entry(spec)
    }
    async fn start(&self, name: &str) -> Result<(), RuntimeError> {
        self.launch(name).await
    }
    async fn stop(&self, name: &str, signal: &str, grace: Duration) -> Result<(), RuntimeError> {
        self.signal_tree(name, rustix::process::Signal::CONT)?;
        self.signal_tree(
            name,
            if signal == "SIGINT" { rustix::process::Signal::INT } else { rustix::process::Signal::TERM },
        )?;
        let until = tokio::time::Instant::now() + grace;
        while tokio::time::Instant::now() < until {
            if !self.running(name)? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.kill(name).await
    }
    async fn kill(&self, name: &str) -> Result<(), RuntimeError> {
        self.terminate_tree(name).await?;
        for _ in 0..100 {
            if !self.running(name)? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Err(RuntimeError::Timeout("Process tree did not stop".into()))
    }
    async fn pause(&self, name: &str) -> Result<(), RuntimeError> {
        self.freeze_tree(name).await.map(|_| ())
    }
    async fn unpause(&self, name: &str) -> Result<(), RuntimeError> {
        self.signal_tree(name, rustix::process::Signal::CONT)
    }
    async fn disable_restart(&self, _: &str) -> Result<(), RuntimeError> {
        Ok(())
    }
    async fn remove(&self, name: &str) -> Result<(), RuntimeError> {
        if self.entry(name).is_err() {
            return Ok(());
        }
        self.kill(name).await?;
        self.remove_entry(name)
    }
    async fn inspect(&self, name: &str) -> Result<Option<ContainerInfo>, RuntimeError> {
        self.observe_entry(name)
    }
    async fn list(&self, filters: &[String]) -> Result<Vec<ContainerInfo>, RuntimeError> {
        let mut found = Vec::new();
        for name in self.names() {
            if let Some(info) = self.observe_entry(&name)?
                && filters.iter().all(|filter| {
                    filter.split_once('=').is_some_and(|(key, value)| info.labels.get(key).is_some_and(|v| v == value))
                })
            {
                found.push(info);
            }
        }
        Ok(found)
    }
    async fn exec(
        &self,
        name: &str,
        command: &[String],
        timeout: Duration,
        max_output: usize,
    ) -> Result<ExecOutput, RuntimeError> {
        self.agent_exec(name, command, timeout, max_output).await
    }
    fn logs(&self, name: &str, options: LogOptions) -> BoxStream<'static, Result<LogChunk, RuntimeError>> {
        self.entrypoint_logs(name, options)
    }
    async fn stats(&self, name: &str) -> Result<Option<RawStats>, RuntimeError> {
        let entry = self.entry(name)?;
        Ok(entry.pid.filter(|pid| observe::identity(*pid) == entry.ticks).map(|pid| RawStats {
            memory_usage: observe::resident_tree(pid),
            memory_limit: u64::try_from(entry.spec.memory_bytes).ok(),
            pids_limit: u64::try_from(entry.spec.pids_limit).ok(),
            ..Default::default()
        }))
    }
    fn events(&self, _: &[String]) -> BoxStream<'static, Result<RuntimeEvent, RuntimeError>> {
        let own = self.clone();
        stream::unfold((own, std::collections::HashSet::new()), |(own, mut dead)| async move {
            loop {
                for name in own.names() {
                    if let Ok(Some(info)) = own.observe_entry(&name) {
                        if matches!(info.status, ContainerStatus::Running | ContainerStatus::Paused) {
                            dead.remove(&info.id);
                        } else if info.status == ContainerStatus::Exited && dead.insert(info.id.clone()) {
                            return Some((
                                Ok(RuntimeEvent {
                                    container_id: info.id,
                                    action: DIED.into(),
                                    exit_code: Some(info.exit_code),
                                    at: crate::clock::now(),
                                }),
                                (own, dead),
                            ));
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .boxed()
    }
}
