// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! The main daemon's VM backend when a separate worker holds the host's VM authority. Each
//! runtime verb crosses the worker's kernel-authenticated socket once, answered with the
//! runtime owner's own types and errors; an uncertain answer is never replayed. The frontend
//! holds the worker's lifetime while it serves and reads the worker's fixed capabilities once.
//! It boots no compute, opens no device and widens nothing the worker's policy refused.
use super::{
    authority::Owner,
    commands::{Capabilities, Command},
    data_client, http, streams,
};
use crate::runtime::{
    ContainerInfo, ContainerRuntime, ContainerSpec, ExecOutput, LogChunk, LogOptions, NetworkReport, RawStats,
    RuntimeError, RuntimeEvent, RuntimeInfo, files, limits, vm::instance::ISOLATION,
};
use async_trait::async_trait;
use futures_util::stream::BoxStream;
use serde::de::DeserializeOwned;
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::time::Instant;

/// Bounds a worker that never answers. A verb's own timeout, where it has one, is added.
const OPERATION: Duration = Duration::from_mins(10);
/// Covers a successor worker's start after the previous pryxd retired its predecessor.
const START: Duration = Duration::from_secs(30);
/// The installed service that runs `pryxd vm-worker` as root.
pub const WORKER_UNIT: &str = "pryxd-vm-worker.service";

fn refused(problem: io::Error) -> RuntimeError {
    RuntimeError::Unavailable(format!("VM worker lifetime refused: {problem}"))
}

pub struct Frontend {
    socket: PathBuf,
    uid: u32,
    capabilities: Capabilities,
    data: data_client::Client,
    _lifetime: Owner,
}

fn deadline(own: Duration) -> Instant {
    Instant::now() + OPERATION + own
}

impl Frontend {
    /// Root's installed worker beside this node's state. A missing or refused socket is waited
    /// for, briefly, while a successor worker starts; an accepted lifetime is never retried.
    pub async fn installed(state: &Path) -> Result<Self, RuntimeError> {
        Self::installed_as(state, rustix::process::Uid::ROOT.as_raw()).await
    }

    pub(super) async fn installed_as(state: &Path, uid: u32) -> Result<Self, RuntimeError> {
        let (lifetime, requests) = super::service::paths(state);
        let deadline = Instant::now() + START;
        let owner = loop {
            match tokio::time::timeout_at(deadline, Owner::connect(&lifetime, uid)).await {
                Ok(Ok(owner)) => break owner,
                Ok(Err(problem))
                    if matches!(problem.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused)
                        && Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(100)).await
                }
                Ok(Err(problem)) => return Err(refused(problem)),
                Err(_) => return Err(RuntimeError::Timeout("The root VM worker did not acknowledge pryxd".into())),
            }
        };
        Self::serving(owner, &requests, uid).await
    }

    /// Whether root's worker is serving, without taking its one lifetime.
    pub fn available(state: &Path) -> Result<(), RuntimeError> {
        Self::available_as(state, rustix::process::Uid::ROOT.as_raw())
    }

    pub(super) fn available_as(state: &Path, uid: u32) -> Result<(), RuntimeError> {
        super::transport::socket(&super::service::paths(state).0, uid).map_err(|problem| {
            RuntimeError::Unavailable(format!(
                "The root VM worker is not serving ({problem}); it runs as {WORKER_UNIT} on a host with KVM"
            ))
        })
    }

    /// The worker's UID is fixed host policy; the kernel proves each socket's peer.
    pub async fn connect(lifetime: &Path, socket: &Path, uid: u32) -> Result<Self, RuntimeError> {
        Self::serving(Owner::connect(lifetime, uid).await.map_err(refused)?, socket, uid).await
    }

    async fn serving(lifetime: Owner, socket: &Path, uid: u32) -> Result<Self, RuntimeError> {
        let answer: Result<Capabilities, RuntimeError> =
            http::control(socket, uid, &Command::Capabilities, deadline(Duration::ZERO)).await?;
        Ok(Self {
            socket: socket.into(),
            uid,
            capabilities: answer?,
            data: data_client::Client { socket: socket.into(), uid, timeout: OPERATION },
            _lifetime: lifetime,
        })
    }

    async fn call<T: DeserializeOwned>(&self, command: Command, own: Duration) -> Result<T, RuntimeError> {
        let answer: Result<T, RuntimeError> = http::control(&self.socket, self.uid, &command, deadline(own)).await?;
        answer
    }
}

#[async_trait]
impl ContainerRuntime for Frontend {
    fn isolation(&self) -> &'static str {
        ISOLATION
    }
    fn uses_host_ports(&self) -> bool {
        self.capabilities.uses_host_ports
    }
    fn uses_remote_data(&self) -> bool {
        self.capabilities.uses_remote_data
    }
    async fn data_service(&self, request: files::Request) -> Result<files::Service, RuntimeError> {
        self.data.files(request).await
    }
    fn supports_data_exclude(&self) -> bool {
        self.capabilities.supports_data_exclude
    }
    fn requires_stopped_data_copy(&self) -> bool {
        self.capabilities.requires_stopped_data_copy
    }
    async fn data_copy_lease(&self, sandbox: &str, data: &Path) -> Result<Box<dyn Send>, RuntimeError> {
        self.data.copy(sandbox, data).await
    }
    fn supports_agent(&self) -> bool {
        self.capabilities.supports_agent
    }
    fn supports_network_policy(&self) -> bool {
        self.capabilities.supports_network_policy
    }
    fn supports_hostname_egress(&self) -> bool {
        self.capabilities.supports_hostname_egress
    }
    fn data_owner(&self, _: &str) -> Result<Option<(u32, u32)>, RuntimeError> {
        Ok(self.capabilities.data_owner)
    }
    async fn info(&self) -> Result<RuntimeInfo, RuntimeError> {
        self.call(Command::Info, Duration::ZERO).await
    }
    async fn resource_limits(&self) -> Result<Option<limits::ResourceLimits>, RuntimeError> {
        self.call(Command::Limits, Duration::ZERO).await
    }
    async fn ensure_network(
        &self,
        name: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<NetworkReport, RuntimeError> {
        self.call(Command::EnsureNetwork { name: name.into(), labels: labels.clone() }, Duration::ZERO).await
    }
    async fn ensure_internal_network(&self, name: &str, labels: &BTreeMap<String, String>) -> Result<(), RuntimeError> {
        self.call(Command::EnsureInternalNetwork { name: name.into(), labels: labels.clone() }, Duration::ZERO).await
    }
    async fn network_isolated(&self, name: &str) -> Result<Option<bool>, RuntimeError> {
        self.call(Command::NetworkIsolated { name: name.into() }, Duration::ZERO).await
    }
    async fn ensure_image(&self, image: &str, timeout: Duration) -> Result<bool, RuntimeError> {
        self.call(Command::EnsureImage { image: image.into(), timeout }, timeout).await
    }
    async fn create(&self, spec: &ContainerSpec) -> Result<String, RuntimeError> {
        self.call(Command::Create { spec: Box::new(spec.clone()) }, Duration::ZERO).await
    }
    async fn start(&self, name: &str) -> Result<(), RuntimeError> {
        self.call(Command::Start { name: name.into() }, Duration::ZERO).await
    }
    async fn stop(&self, name: &str, signal: &str, grace: Duration) -> Result<(), RuntimeError> {
        self.call(Command::Stop { name: name.into(), signal: signal.into(), grace }, grace).await
    }
    async fn kill(&self, name: &str) -> Result<(), RuntimeError> {
        self.call(Command::Kill { name: name.into() }, Duration::ZERO).await
    }
    async fn pause(&self, name: &str) -> Result<(), RuntimeError> {
        self.call(Command::Pause { name: name.into() }, Duration::ZERO).await
    }
    async fn unpause(&self, name: &str) -> Result<(), RuntimeError> {
        self.call(Command::Unpause { name: name.into() }, Duration::ZERO).await
    }
    async fn disable_restart(&self, name: &str) -> Result<(), RuntimeError> {
        self.call(Command::DisableRestart { name: name.into() }, Duration::ZERO).await
    }
    async fn remove(&self, name: &str) -> Result<(), RuntimeError> {
        self.call(Command::Remove { name: name.into() }, Duration::ZERO).await
    }
    async fn inspect(&self, name: &str) -> Result<Option<ContainerInfo>, RuntimeError> {
        self.call(Command::Inspect { name: name.into() }, Duration::ZERO).await
    }
    async fn list(&self, filters: &[String]) -> Result<Vec<ContainerInfo>, RuntimeError> {
        self.call(Command::List { filters: filters.to_vec() }, Duration::ZERO).await
    }
    async fn exec(
        &self,
        name: &str,
        command: &[String],
        timeout: Duration,
        max_output: usize,
    ) -> Result<ExecOutput, RuntimeError> {
        self.call(Command::Exec { name: name.into(), command: command.to_vec(), timeout, max_output }, timeout).await
    }
    fn logs(&self, name: &str, options: LogOptions) -> BoxStream<'static, Result<LogChunk, RuntimeError>> {
        streams::logs(&self.socket, self.uid, name.into(), options, deadline(Duration::ZERO))
    }
    async fn stats(&self, name: &str) -> Result<Option<RawStats>, RuntimeError> {
        self.call(Command::Stats { name: name.into() }, Duration::ZERO).await
    }
    fn events(&self, filters: &[String]) -> BoxStream<'static, Result<RuntimeEvent, RuntimeError>> {
        streams::events(&self.socket, self.uid, filters.to_vec(), deadline(Duration::ZERO))
    }
}
