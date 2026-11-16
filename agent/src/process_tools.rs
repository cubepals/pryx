// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-supplied native tool inputs and a closed pre-policy acquisition worker. Ordinary guest
//! commands never select an artifact, cache grant or executable through this channel.
//!
//! Parts (`process_tools/`):
//! - `browser.rs`: fixed launch through maintained private namespaces.
//! - `browser_session.rs`: scoped Unix connections and bounded screenshots.
//! - `desktop.rs`: closed desktop launch with private temporary and proc mounts.
//! - `desktop_session.rs`: scoped native desktop calls and worker lifetime.
//! - `ssh.rs`: unchanged OpenSSH inetd behind private accounts and Unix sockets.
//! - `ssh_session.rs`: native SSH worker lifetime and lease-bound logical ports.
//! - `cache.rs`: pinned acquisition, safe extraction and complete cache verification.
//! - `cache_tests.rs`: real HTTP/archive integrity and cache-tampering regressions.
pub mod browser;
mod browser_session;
mod cache;
#[cfg(test)]
#[path = "process_tools/cache_tests.rs"]
mod cache_tests;
pub mod desktop;
mod desktop_session;
pub mod ssh;
mod ssh_session;

use crate::context::Failure;
pub use crate::process_tool_policy::{Artifact, Policy};
use axum::http::StatusCode;
pub use browser_session::Browser;
use browser_session::failed;
pub use desktop_session::Desktop;
pub use ssh_session::Ssh;
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock, RwLock},
};
use tokio::sync::{Mutex, mpsc, oneshot};

type Prepared = Result<PathBuf, String>;
enum Request {
    Prepare(oneshot::Sender<Prepared>),
    Browser(browser::Launch, oneshot::Sender<Result<std::process::Child, String>>),
    Desktop(desktop::Launch, oneshot::Sender<Result<std::process::Child, String>>),
    Ssh(ssh::Launch, oneshot::Sender<Result<std::process::Child, String>>),
    Stop,
}
pub struct Tools {
    pub policy: Policy,
    requests: mpsc::Sender<Request>,
    stopping: Arc<std::sync::atomic::AtomicBool>,
    worker: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    browser_transition: Mutex<()>,
    browser: RwLock<Option<Arc<Browser>>>,
    desktop_transition: Mutex<()>,
    desktop: RwLock<Option<Arc<Desktop>>>,
    ssh: RwLock<std::collections::BTreeMap<u16, std::sync::Weak<Ssh>>>,
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
    let stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker = if policy.unavailable.is_none() {
        let captured = policy.clone();
        let closed = stopping.clone();
        Some(std::thread::Builder::new().name("native-tools".into()).spawn(move || {
            let priority = rustix::process::setpriority_process(None, process.nice);
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
            while let Some(request) = receive.blocking_recv() {
                if matches!(request, Request::Stop) {
                    break;
                }
                if closed.load(std::sync::atomic::Ordering::Acquire) {
                    continue;
                }
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
                    Request::Desktop(launch, reply) => {
                        let result = if priority.is_ok() {
                            captured
                                .artifact
                                .as_ref()
                                .ok_or_else(|| "Missing native desktop artifact".to_string())
                                .and_then(|artifact| {
                                    let bundle = captured.cache.join(&artifact.sha256);
                                    std::env::current_exe()
                                        .and_then(|executable| {
                                            desktop::start_at(&process, launch, &bundle, &executable)
                                        })
                                        .map_err(|e| e.to_string())
                                })
                        } else {
                            Err("Native desktop worker is unavailable".into())
                        };
                        if let Err(Ok(mut child)) = reply.send(result) {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                    }
                    Request::Ssh(launch, reply) => {
                        let result = if priority.is_ok() {
                            captured
                                .artifact
                                .as_ref()
                                .ok_or_else(|| "Missing native SSH artifact".to_string())
                                .and_then(|artifact| {
                                    let bundle = captured.cache.join(&artifact.sha256);
                                    std::env::current_exe()
                                        .and_then(|executable| ssh::start_at(&process, launch, &bundle, &executable))
                                        .map_err(|error| error.to_string())
                                })
                        } else {
                            Err("Native SSH worker is unavailable".into())
                        };
                        if let Err(Ok(mut child)) = reply.send(result) {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                    }
                    Request::Stop => break,
                }
            }
        })?)
    } else {
        None
    };
    TOOLS
        .set(Arc::new(Tools {
            policy,
            requests,
            stopping,
            worker: std::sync::Mutex::new(worker),
            ssh: RwLock::default(),
            transition: Mutex::new(()),
            root: RwLock::new(None),
            browser_transition: Mutex::new(()),
            browser: RwLock::new(None),
            desktop_transition: Mutex::new(()),
            desktop: RwLock::new(None),
        }))
        .map_err(|_| std::io::Error::other("Native process tools are already configured"))
}

/// Acquire another host-owned runtime artifact with the same bounded, pinned archive format.
/// Call before any guest policy; the caller owns private cache/download directories and verifies
/// its component-specific manifest before executing a program. No tool worker is installed.
pub async fn acquire_artifact(
    cache: PathBuf,
    downloads: PathBuf,
    artifact: Artifact,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    cache::prepare(&Policy {
        cache,
        scratch: downloads.clone(),
        downloads,
        artifact: Some(artifact),
        unavailable: None,
        browser_on: false,
        desktop_on: None,
        desktop_unavailable: None,
    })
    .await
}

pub fn configured() -> Option<&'static Arc<Tools>> {
    TOOLS.get()
}

/// Stops every worker that can spawn a native tool. The caller drops its main runtime before
/// joining, so canceled requesters cannot queue more work while teardown drains the queue.
pub fn close_admission() {
    if let Some(tools) = configured() {
        tools.stopping.store(true, std::sync::atomic::Ordering::Release);
    }
}

pub fn shutdown() -> std::io::Result<()> {
    configured().map_or(Ok(()), |tools| tools.shutdown())
}

impl Tools {
    fn shutdown(&self) -> std::io::Result<()> {
        self.stopping.store(true, std::sync::atomic::Ordering::Release);
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = self.requests.blocking_send(Request::Stop);
            worker.join().map_err(|_| std::io::Error::other("Native tool worker did not stop cleanly"))?;
        }
        *self.browser.write().unwrap() = None;
        *self.desktop.write().unwrap() = None;
        self.ssh.write().unwrap().clear();
        Ok(())
    }
    pub fn root(&self) -> Option<PathBuf> {
        self.root.read().unwrap().clone()
    }

    pub fn supported(&self) -> Result<(), crate::context::Failure> {
        if self.stopping.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Failure(StatusCode::SERVICE_UNAVAILABLE, "stopping", "Native tools are stopping".into()));
        }
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

impl Tools {
    pub fn active_desktop(&self) -> Option<Arc<Desktop>> {
        self.desktop.read().unwrap().clone()
    }
    pub async fn desktop(&self, scratch: &std::path::Path) -> Result<Arc<Desktop>, Failure> {
        self.supported()?;
        if self.policy.desktop_on != Some(true) {
            return Err(Failure(axum::http::StatusCode::NOT_IMPLEMENTED,"unsupported",
                self.policy.desktop_unavailable.clone().unwrap_or_else(|| "Native desktop needs an installation providing its verified bundle, host-glibc and private-namespace policy".into())));
        }
        if std::env::var_os(crate::process_policy::NETWORK_ENV).is_some() {
            return Err(Failure(
                axum::http::StatusCode::NOT_IMPLEMENTED,
                "unsupported",
                "Process desktop is off for network-none; ordinary command egress remains denied".into(),
            ));
        }
        let root = self.prepare().await?;
        if !root.join("desktop/adapter/start.mjs").is_file() {
            return Err(failed("The installed native artifact has no verified desktop component"));
        }
        let _transition = self.desktop_transition.lock().await;
        if let Some(desktop) = self.active_desktop() {
            return Ok(desktop);
        }
        let launch = desktop::Launch {
            scratch: scratch.into(),
            address: format!("pryx-desktop-{}", uuid::Uuid::new_v4()),
            private_token: uuid::Uuid::new_v4().to_string(),
            application_url: None,
        };
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Desktop(launch.clone(), reply))
            .await
            .map_err(|_| failed("Desktop worker stopped"))?;
        let child = answer.await.map_err(|_| failed("Desktop worker stopped"))?.map_err(failed)?;
        let desktop = Arc::new(Desktop::new(launch, child));
        tokio::time::timeout(crate::contract::TOOL_STARTUP, async {
            loop {
                if desktop.exited()? {
                    return Err(std::io::Error::other("Native desktop startup failed; inspect its private diagnostic"));
                }
                if desktop.connect().await.is_ok() {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| failed("Desktop startup deadline elapsed"))?
        .map_err(|e| failed(e.to_string()))?;
        *self.desktop.write().unwrap() = Some(desktop.clone());
        Ok(desktop)
    }
}

impl Tools {
    pub fn active_ssh(&self, port: u16) -> Option<Arc<Ssh>> {
        self.ssh.read().unwrap().get(&port).and_then(std::sync::Weak::upgrade)
    }
    pub async fn ssh(&self, launch: ssh::Launch, held: tokio::net::TcpListener) -> Result<Arc<Ssh>, Failure> {
        self.supported()?;
        if self.policy.desktop_on != Some(true) {
            return Err(Failure(
                StatusCode::NOT_IMPLEMENTED,
                "unsupported",
                "Native SSH requires the verified desktop/OpenSSH component, host glibc and private user namespaces"
                    .into(),
            ));
        }
        if std::env::var_os(crate::process_policy::NETWORK_ENV).is_some() {
            return Err(Failure(
                StatusCode::NOT_IMPLEMENTED,
                "unsupported",
                "Process SSH is off for network-none; ordinary command egress remains denied".into(),
            ));
        }
        let root = self.prepare().await?;
        if !root.join("desktop/bin/sshd").is_file() {
            return Err(failed("The installed native artifact has no verified OpenSSH component"));
        }
        let (reply, answer) = oneshot::channel();
        self.requests.send(Request::Ssh(launch.clone(), reply)).await.map_err(|_| failed("SSH worker stopped"))?;
        let child = answer.await.map_err(|_| failed("SSH worker stopped"))?.map_err(failed)?;
        let port = held.local_addr()?.port();
        let session = Arc::new(Ssh::new(launch, child, held));
        tokio::time::timeout(crate::contract::TOOL_STARTUP, async {
            loop {
                if !session.running() {
                    return Err(std::io::Error::other("Native SSH worker exited before readiness"));
                }
                if session.ready() && session.connect().await.is_ok() {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| failed("Native SSH startup deadline elapsed"))?
        .map_err(|error| failed(error.to_string()))?;
        let mut routes = self.ssh.write().unwrap();
        routes.retain(|_, value| value.strong_count() > 0);
        routes.insert(port, Arc::downgrade(&session));
        Ok(session)
    }
}

#[cfg(test)]
mod retirement_tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn shutdown_waits_for_an_in_progress_spawn_and_joins_its_worker() {
        let root = tempfile::tempdir().unwrap();
        let (requests, mut receive) = mpsc::channel(1);
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, proceed) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _request = receive.blocking_recv().unwrap();
            entered.send(()).unwrap();
            proceed.recv().unwrap();
            // An already-admitted tool launch can finish after close. Shutdown must join this
            // worker, including its dropped-requester cleanup, before final retirement proof.
            let mut child = std::process::Command::new("sleep").arg("60").spawn().unwrap();
            assert!(matches!(receive.blocking_recv(), Some(Request::Stop)));
            child.kill().unwrap();
            assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        });
        let tools = Arc::new(Tools {
            policy: Policy {
                cache: root.path().into(),
                downloads: root.path().into(),
                scratch: root.path().into(),
                artifact: None,
                unavailable: None,
                browser_on: false,
                desktop_on: None,
                desktop_unavailable: None,
            },
            requests,
            stopping: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            worker: std::sync::Mutex::new(Some(worker)),
            browser_transition: Mutex::new(()),
            browser: RwLock::default(),
            desktop_transition: Mutex::new(()),
            desktop: RwLock::default(),
            ssh: RwLock::default(),
            transition: Mutex::new(()),
            root: RwLock::default(),
        });
        let (reply, _answer) = oneshot::channel();
        tools.requests.blocking_send(Request::Prepare(reply)).unwrap();
        ready.recv().unwrap();
        let (done, completed) = std::sync::mpsc::channel();
        let captured = tools.clone();
        let stopping = std::thread::spawn(move || {
            let _ = done.send(captured.shutdown().is_ok());
        });
        assert!(
            completed.recv_timeout(std::time::Duration::from_millis(50)).is_err(),
            "the in-flight worker was joined"
        );
        release.send(()).unwrap();
        assert!(completed.recv_timeout(std::time::Duration::from_secs(5)).unwrap());
        stopping.join().unwrap();
        assert!(tools.supported().is_err(), "admission stays closed after shutdown");
    }
}
