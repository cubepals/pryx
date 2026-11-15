// SPDX-FileCopyrightText: 2026 The Cubepals Authors
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
use crate::http_client;
use async_trait::async_trait;
use hyper::{Method, Response, body::Incoming};
use pryx_agent::auth::SigningKey;
use std::{future::Future, net::SocketAddr, time::Duration};

/// Host-owned data identity and resource bounds; no workload command, image or secret enters it.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
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

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
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

#[async_trait]
/// Backend-owned file session transport. Authentication and streaming stay with its owner.
pub trait Transport: Send + Sync {
    async fn request(
        &self,
        operation: Operation,
        path: &str,
        body: http_client::Body,
    ) -> Result<Response<Incoming>, RuntimeError>;
}

pub struct Service {
    transport: Box<dyn Transport>,
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
        Ok(Self::with_transport(agent::Agent::new(endpoint, key, identity, timeout)?, cleanup))
    }

    pub fn with_transport(
        transport: impl Transport + 'static,
        cleanup: impl Future<Output = Result<(), RuntimeError>> + Send + 'static,
    ) -> Self {
        let (release, released) = tokio::sync::oneshot::channel();
        let cleanup = tokio::spawn(async move {
            let _ = released.await;
            let result = cleanup.await;
            if let Err(problem) = &result {
                tracing::warn!(%problem, "File-service cleanup failed");
            }
            result
        });
        Self { transport: Box::new(transport), release: Some(release), cleanup: Some(cleanup) }
    }

    pub async fn request(
        &self,
        operation: Operation,
        path: &str,
        body: http_client::Body,
    ) -> Result<Response<Incoming>, RuntimeError> {
        self.transport.request(operation, path, body).await
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
