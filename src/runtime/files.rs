// SPDX-License-Identifier: FSL-1.1-ALv2
//! Authenticated streaming file sessions keep backend cleanup owned after caller cancellation.
//! Only four fixed data operations are available. This transport never selects a workload,
//! boots compute, retries a mutation or grants a caller a different backend policy.
//!
//! Parts (`files/`):
//! - `agent.rs`: authenticated fixed operations over the owned private endpoint.
//! - `tests.rs`: real streaming and cancellation-owned cleanup acceptance.
mod agent;
#[cfg(test)]
mod tests;
use super::RuntimeError;
use hyper::Method;
use pryx_agent::auth::SigningKey;
use std::{future::Future, net::SocketAddr, time::Duration};

/// Host-owned data identity and resource bounds; no workload command, image or secret enters it.
#[derive(Clone)]
pub struct Request {
    pub name: String,
    pub sandbox: String,
    pub epoch: u64,
    pub data_dir: std::path::PathBuf,
    pub mount_path: String,
    pub user: String,
    pub memory_bytes: i64,
    pub storage_bytes: u64,
    pub pids_limit: i64,
    pub tmp_size_mb: u32,
    pub log_max_size_mb: u32,
}

pub enum Operation {
    Read,
    Write,
    List,
    Stat,
}
impl Operation {
    fn route(&self) -> (&'static str, Method) {
        match self {
            Self::Read => ("content", Method::GET),
            Self::Write => ("content", Method::PUT),
            Self::List => ("list", Method::GET),
            Self::Stat => ("stat", Method::GET),
        }
    }
}
pub struct Service {
    endpoint: SocketAddr,
    key: SigningKey,
    sandbox: String,
    epoch: u64,
    timeout: Duration,
    release: Option<tokio::sync::oneshot::Sender<()>>,
    cleanup: Option<tokio::task::JoinHandle<Result<(), RuntimeError>>>,
}
impl Service {
    pub fn new(
        endpoint: SocketAddr,
        key: SigningKey,
        identity: (String, u64),
        timeout: Duration,
        cleanup: impl Future<Output = Result<(), RuntimeError>> + Send + 'static,
    ) -> Result<Self, RuntimeError> {
        let private = match endpoint.ip() {
            std::net::IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
            std::net::IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
        };
        if !private || endpoint.port() == 0 || timeout.is_zero() {
            return Err(RuntimeError::Other("File service requires a private endpoint and positive deadline".into()));
        }
        let (release, released) = tokio::sync::oneshot::channel();
        let cleanup = tokio::spawn(async move {
            let _ = released.await;
            let result = cleanup.await;
            if let Err(problem) = &result {
                tracing::warn!(%problem, "File-service cleanup failed");
            }
            result
        });
        Ok(Self {
            endpoint,
            key,
            sandbox: identity.0,
            epoch: identity.1,
            timeout,
            release: Some(release),
            cleanup: Some(cleanup),
        })
    }
    /// Cleanup stays in its own task if waiting for it is cancelled.
    pub async fn close(mut self) -> Result<(), RuntimeError> {
        self.release.take();
        self.cleanup
            .take()
            .ok_or_else(|| RuntimeError::Other("File service is closed".into()))?
            .await
            .map_err(|problem| RuntimeError::Other(problem.to_string()))?
    }
}
