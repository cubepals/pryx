// SPDX-License-Identifier: FSL-1.1-ALv2
//! A sandbox supervisor without an engine, in namespaces and a cgroup of its own where the host
//! gives them (`namespaces.rs`, `network.rs`). Private runtime journals
//! allow adoption after daemon restart; process identities include kernel start ticks, not PID
//! alone. The manager retains the existing epoch, snapshot and data-directory lifecycle.
//!
//! Parts (`process/`):
//! - `native.rs`: protected tool inputs and authenticated supervisor adoption.
//! - `journal.rs`: private specifications, UID allocation and process identity checks.
//! - `power.rs`: agent launch, tree signals and process observations.
//! - `exec.rs`: the legacy node exec transport through the same authenticated agent.
//! - `observe.rs`: read-only kernel process identity, ownership and tree observations.
//! - `logs.rs`: bounded, safe entrypoint log replay and following.
//! - `termination.rs`: stable tree freezing and orphan retirement.
//! - `namespaces.rs`: the namespaced mode's helper, its arguments and its probe.
//! - `network.rs`: each sandbox's cgroup, kernel network policy and hostname gateways.
//! - `adoption.rs`: turning that policy on at start and adopting what outlived the last daemon.
/// The host probe's name for the namespaced process mode.
pub const NAMESPACES: &str = "process-namespaces";
mod adoption;
mod exec;
mod journal;
mod logs;
pub mod namespaces;
mod native;
mod network;
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
    /// The kernel network policy, where this host lets the runtime attach it.
    network: Option<Arc<network::Network>>,
    network_capability: Option<crate::capability::Capability>,
}
pub use network::Served;
struct State {
    pub root: PathBuf,
    pub launch: tokio::sync::Mutex<()>,
    pub entries: Mutex<BTreeMap<String, Entry>>,
    pub children: Mutex<HashMap<String, std::process::Child>>,
    pub owners: Mutex<BTreeMap<String, (u32, u32)>>,
    /// Each agent executable's namespace helper, or why it can't be used: probed once.
    pub namespaces: Mutex<BTreeMap<PathBuf, Result<PathBuf, String>>>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_browser: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    network: Option<network::Record>,
    /// Set when the supervisor started in its own namespaces; absent in the plain mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespaced: Option<namespaces::Namespaced>,
}

use std::sync::Arc;

#[async_trait]
impl ContainerRuntime for ProcessRuntime {
    fn is_process(&self) -> bool {
        true
    }
    async fn native_browser(&self, binary: &std::path::Path) -> Result<(), RuntimeError> {
        self.browser_requirement(binary).await
    }
    async fn native_desktop(&self, binary: &std::path::Path) -> Result<(), RuntimeError> {
        self.desktop_requirement(binary).await
    }
    async fn process_namespaces(&self, binary: &std::path::Path) -> Result<(), RuntimeError> {
        let own = self.clone();
        let binary = binary.to_owned();
        tokio::task::spawn_blocking(move || own.namespace_helper(&binary).map(|_| ()))
            .await
            .map_err(|_| RuntimeError::Other("The namespace probe stopped".into()))?
            .map_err(RuntimeError::Other)
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
    fn supports_hostname_egress(&self) -> bool {
        self.network.is_some()
    }
    fn supports_egress_addresses(&self) -> bool {
        self.network.is_some()
    }
    fn kernel_network_policy(&self) -> Option<crate::capability::Capability> {
        let proved = self.network_capability.clone()?;
        let unpoliced = self.unpoliced();
        if proved.state != crate::capability::State::On || unpoliced == 0 {
            return Some(proved);
        }
        // Another sandbox's listeners trust loopback, which such a sandbox reaches unpoliced.
        Some(crate::capability::Capability::new(
            crate::capability::State::Off,
            format!(
                "{unpoliced} process sandbox(es) run without verified kernel network policy (started before this \
                 host had it, or their rules no longer fit its addresses) and could reach other sandboxes' \
                 listeners; {}",
                proved.evidence
            ),
            "It comes back once they stop or are started again, which attaches their programs",
        ))
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
                return self.release_network(&self.entry(name)?.spec.name);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.kill(name).await
    }
    async fn kill(&self, name: &str) -> Result<(), RuntimeError> {
        self.terminate_tree(name).await?;
        let key = self.entry(name)?.spec.name;
        for _ in 0..100 {
            if !self.running(name)? && !self.state.children.lock().unwrap().contains_key(&key) {
                return self.release_network(&key);
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
        let counts = self.network.as_ref().and_then(|network| network.counts(&entry.spec.name));
        Ok(observe::matching_pid(entry.pid, entry.ticks).map(|pid| RawStats {
            memory_usage: observe::resident_tree(pid),
            memory_limit: u64::try_from(entry.spec.memory_bytes).ok(),
            pids_limit: u64::try_from(entry.spec.pids_limit).ok(),
            rx_bytes: counts.map(|counts| counts.ingress[1]),
            tx_bytes: counts.map(|counts| counts.egress[1]),
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
