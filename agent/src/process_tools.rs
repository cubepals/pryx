// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-supplied native tool inputs and a closed pre-policy acquisition worker. Ordinary guest
//! commands never select an artifact, cache grant or executable through this channel.
//!
//! Parts (`process_tools/`):
//! - `browser.rs`: fixed launch through maintained private namespaces.
//! - `browser_session.rs`: scoped Unix connections and bounded screenshots.
//! - `cache.rs`: pinned acquisition, safe extraction and complete cache verification.
//! - `cache_tests.rs`: real HTTP/archive integrity and cache-tampering regressions.
pub mod browser;
mod browser_session;
mod cache;
#[cfg(test)]
#[path = "process_tools/cache_tests.rs"]
mod cache_tests;

use crate::context::Failure;
pub use crate::process_tool_policy::{Artifact, Policy};
use axum::http::StatusCode;
pub use browser_session::Browser;
use browser_session::failed;
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock, RwLock},
};
use tokio::sync::{Mutex, mpsc, oneshot};

type Prepared = Result<PathBuf, String>;
enum Request {
    Prepare(oneshot::Sender<Prepared>),
    Browser(browser::Launch, oneshot::Sender<Result<std::process::Child, String>>),
}
pub struct Tools {
    pub policy: Policy,
    requests: mpsc::Sender<Request>,
    browser_transition: Mutex<()>,
    browser: RwLock<Option<Arc<Browser>>>,
    transition: Mutex<()>,
    root: RwLock<Option<PathBuf>>,
}
static TOOLS: OnceLock<Arc<Tools>> = OnceLock::new();

/// Called only by the trusted bootstrap after private scoping and before its filesystem filter.
pub fn install(policy: Policy, process: crate::process_policy::Policy) -> std::io::Result<()> {
    if policy.unavailable.is_none() && !crate::process_policy::ipc_scoped() {
        return Err(std::io::Error::other("Native process tools require enforced private IPC"));
    }
    let (requests, mut receive) = mpsc::channel::<Request>(1);
    if policy.unavailable.is_none() {
        let captured = policy.clone();
        std::thread::Builder::new().name("native-tools".into()).spawn(move || {
            let priority = rustix::process::setpriority_process(None, process.nice);
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
            while let Some(request) = receive.blocking_recv() {
                match request {
                    Request::Prepare(reply) => {
                        let result = match (&priority, &runtime) {
                            (Ok(()), Ok(runtime)) => {
                                runtime.block_on(cache::prepare(&captured)).map_err(|error| error.to_string())
                            }
                            _ => Err("Native tool acquisition worker is unavailable".into()),
                        };
                        let _ = reply.send(result);
                    }
                    Request::Browser(launch, reply) => {
                        let result = if priority.is_ok() {
                            browser::start(&process, &captured, launch).map_err(|error| error.to_string())
                        } else {
                            Err("Native browser worker is unavailable".into())
                        };
                        if let Err(Ok(mut child)) = reply.send(result) {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                    }
                }
            }
        })?;
    }
    TOOLS
        .set(Arc::new(Tools {
            policy,
            requests,
            transition: Mutex::new(()),
            root: RwLock::new(None),
            browser_transition: Mutex::new(()),
            browser: RwLock::new(None),
        }))
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
        self.requests
            .send(Request::Prepare(reply))
            .await
            .map_err(|_| refused("Native tool acquisition worker stopped".into()))?;
        let root =
            answer.await.map_err(|_| refused("Native tool acquisition worker stopped".into()))?.map_err(refused)?;
        *self.root.write().unwrap() = Some(root.clone());
        Ok(root)
    }
}

impl Tools {
    pub fn active_browser(&self) -> Option<Arc<Browser>> {
        self.browser.read().unwrap().clone()
    }
    pub async fn browser(&self, scratch: &std::path::Path) -> Result<Arc<Browser>, Failure> {
        self.supported()?;
        if !self.policy.browser_on {
            return Err(Failure(
                StatusCode::NOT_IMPLEMENTED,
                "unsupported",
                "Process browser requires usable unprivileged user namespaces; allow them in the host security policy"
                    .into(),
            ));
        }
        if std::env::var_os(crate::process_policy::NETWORK_ENV).is_some() {
            return Err(Failure(
                StatusCode::NOT_IMPLEMENTED,
                "unsupported",
                "Process browser is off for network-none; ordinary command egress remains denied".into(),
            ));
        }
        self.prepare().await?;
        let _transition = self.browser_transition.lock().await;
        if let Some(browser) = self.active_browser() {
            return Ok(browser);
        }
        let launch = browser::Launch {
            scratch: scratch.into(),
            address: format!("pryx-browser-{}", uuid::Uuid::new_v4()),
            private_token: uuid::Uuid::new_v4().to_string(),
        };
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Browser(launch.clone(), reply))
            .await
            .map_err(|_| failed("Browser worker stopped"))?;
        let child = answer.await.map_err(|_| failed("Browser worker stopped"))?.map_err(failed)?;
        let browser = Arc::new(Browser::new(launch, child));
        tokio::time::timeout(crate::contract::TOOL_STARTUP, async {
            loop {
                if browser.exited()? {
                    return Err(std::io::Error::other(
                        "Mandatory process browser sandbox or pipe startup failed; inspect its private diagnostic",
                    ));
                }
                if browser.connect().await.is_ok() {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| failed("Browser startup deadline elapsed"))?
        .map_err(|problem| failed(problem.to_string()))?;
        *self.browser.write().unwrap() = Some(browser.clone());
        Ok(browser)
    }
}
