// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-supplied native tool inputs and a closed pre-policy acquisition worker. Ordinary guest
//! commands never select an artifact, cache grant or executable through this channel.
//!
//! Parts (`process_tools/`):
//! - `cache.rs`: pinned acquisition, safe extraction and complete cache verification.
//! - `cache_tests.rs`: real HTTP/archive integrity and cache-tampering regressions.
mod cache;
#[cfg(test)]
#[path = "process_tools/cache_tests.rs"]
mod cache_tests;

pub use crate::process_tool_policy::{Artifact, Policy};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock, RwLock},
};
use tokio::sync::{Mutex, mpsc, oneshot};

type Prepared = Result<PathBuf, String>;
pub struct Tools {
    pub policy: Policy,
    requests: mpsc::Sender<oneshot::Sender<Prepared>>,
    transition: Mutex<()>,
    root: RwLock<Option<PathBuf>>,
}
static TOOLS: OnceLock<Arc<Tools>> = OnceLock::new();

/// Called only by the trusted bootstrap after private scoping and before its filesystem filter.
pub fn install(policy: Policy, nice: i32) -> std::io::Result<()> {
    if policy.unavailable.is_none() && !crate::process_policy::ipc_scoped() {
        return Err(std::io::Error::other("Native process tools require enforced private IPC"));
    }
    let (requests, mut receive) = mpsc::channel::<oneshot::Sender<Prepared>>(1);
    if policy.unavailable.is_none() {
        let captured = policy.clone();
        std::thread::Builder::new().name("native-tools".into()).spawn(move || {
            let priority = rustix::process::setpriority_process(None, nice);
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
            while let Some(reply) = receive.blocking_recv() {
                let result = match (&priority, &runtime) {
                    (Ok(()), Ok(runtime)) => {
                        runtime.block_on(cache::prepare(&captured)).map_err(|error| error.to_string())
                    }
                    _ => Err("Native tool acquisition worker is unavailable".into()),
                };
                let _ = reply.send(result);
            }
        })?;
    }
    TOOLS
        .set(Arc::new(Tools { policy, requests, transition: Mutex::new(()), root: RwLock::new(None) }))
        .map_err(|_| std::io::Error::other("Native process tools are already configured"))
}

pub fn configured() -> Option<&'static Arc<Tools>> {
    TOOLS.get()
}

impl Tools {
    pub fn root(&self) -> Option<PathBuf> {
        self.root.read().unwrap().clone()
    }

    pub fn supported(&self) -> Result<(), crate::context::Failure> {
        if let Some(reason) = &self.policy.unavailable {
            return Err(crate::context::Failure(
                axum::http::StatusCode::NOT_IMPLEMENTED,
                "unsupported",
                reason.clone(),
            ));
        }
        Ok(())
    }

    pub async fn prepare(&self) -> Result<PathBuf, crate::context::Failure> {
        use axum::http::StatusCode;
        let refused = |message: String| crate::context::Failure(StatusCode::NOT_IMPLEMENTED, "unsupported", message);
        self.supported()?;
        let _transition = self.transition.lock().await;
        if let Some(root) = self.root.read().unwrap().clone() {
            return Ok(root);
        }
        let (reply, answer) = oneshot::channel();
        self.requests.send(reply).await.map_err(|_| refused("Native tool acquisition worker stopped".into()))?;
        let root =
            answer.await.map_err(|_| refused("Native tool acquisition worker stopped".into()))?.map_err(refused)?;
        *self.root.write().unwrap() = Some(root.clone());
        Ok(root)
    }
}
