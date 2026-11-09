// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Managed kernel processes and maintained Jupyter channels. User code is sent once; only the
//! side-effect-free kernel-info startup handshake may repeat while subscriptions become ready.

use super::events::{self, KernelEvent, Language};
use crate::{
    context::Failure,
    process::{Command, Process, Processes, Start},
};
use axum::http::StatusCode;
use jupyter_protocol::{
    ConnectionInfo, ExecuteRequest, ExecutionState, JupyterMessage, JupyterMessageContent, KernelInfoRequest,
};
use jupyter_zmq_client::{ClientIoPubConnection, ClientShellConnection};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

pub(crate) const STARTUP: Duration = Duration::from_secs(30);
pub(crate) const STOP: Duration = Duration::from_secs(5);
pub(crate) const DEFAULT_RUN: Duration = Duration::from_secs(30);
const BACKPRESSURE: Duration = Duration::from_secs(5);
// Jupyter messaging 5.5 defines shell, control, stdin, heartbeat and IOPub channels.
const CHANNELS: usize = 5;
// Jupyter connection files specify this HMAC scheme (jupyter_client.connect).
const SIGNATURE: &str = "hmac-sha256";

pub(crate) fn failure(code: &'static str, message: &str) -> Failure {
    Failure(StatusCode::CONFLICT, code, message.into())
}

pub(crate) struct Channels {
    pub(crate) shell: ClientShellConnection,
    pub(crate) iopub: ClientIoPubConnection,
}

pub(crate) struct Session {
    pub(crate) process: Arc<Process>,
    pub(crate) cancel: CancellationToken,
    pub(crate) busy: AtomicBool,
    pub(crate) jobs: Arc<Semaphore>,
    pub(crate) channels: Mutex<Channels>,
    _directory: tempfile::TempDir,
}

struct Starting(Option<Arc<Process>>);
impl Drop for Starting {
    fn drop(&mut self) {
        if let Some(process) = &self.0 {
            let _ = process.signal(libc::SIGKILL);
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancel.cancel();
        if self.process.view().running {
            let _ = self.process.signal(libc::SIGKILL);
        }
    }
}
struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Session {
    pub(crate) async fn start(owner: &Processes, language: Language) -> Result<Arc<Self>, Failure> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let directory = tempfile::Builder::new().prefix("pryx-kernel-").tempdir()?;
        let path = directory.path().join("connection.json");
        let ip = std::net::Ipv4Addr::LOCALHOST;
        let (ports, reservations) = jupyter_zmq_client::peek_ports_with_listeners(ip.into(), CHANNELS)
            .await
            .map_err(|_| failure("kernel_start_failed", "Cannot reserve kernel channels"))?;
        let info = ConnectionInfo {
            transport: jupyter_protocol::connection_info::Transport::TCP,
            ip: ip.to_string(),
            stdin_port: ports[0],
            control_port: ports[1],
            hb_port: ports[2],
            shell_port: ports[3],
            iopub_port: ports[4],
            key: uuid::Uuid::new_v4().to_string(),
            signature_scheme: SIGNATURE.into(),
            kernel_name: None,
        };
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
        file.write_all(serde_json::to_string(&info).expect("ConnectionInfo serializes").as_bytes())?;
        drop(file);
        let config = path.to_string_lossy().into_owned();
        let command = match language {
            Language::Python => {
                let mut command = vec!["python3".into(), "-m".into(), "ipykernel_launcher".into(), "-f".into(), config];
                // Read image configuration explicitly; runtime history/state belongs in the
                // private writable directory, never the image's root-owned IPython profile.
                if let Some(root) = std::env::var_os("IPYTHONDIR") {
                    command.extend([
                        "--config".into(),
                        std::path::Path::new(&root)
                            .join("profile_default/ipython_config.py")
                            .to_string_lossy()
                            .into_owned(),
                    ]);
                }
                command
            }
            Language::Javascript => vec![
                "node".into(),
                "--require".into(),
                "/opt/pryx/interrupt.cjs".into(),
                "/opt/pryx/kernel.cjs".into(),
                config,
            ],
        };
        let mut env = BTreeMap::new();
        env.insert("MPLCONFIGDIR".into(), directory.path().join("matplotlib").to_string_lossy().into_owned());
        env.insert("JUPYTER_RUNTIME_DIR".into(), directory.path().to_string_lossy().into_owned());
        env.insert("IPYTHONDIR".into(), directory.path().join("ipython").to_string_lossy().into_owned());
        env.insert("PYTHONUNBUFFERED".into(), "1".into());
        let process = owner.start(
            Start {
                command: Command::Argv(command),
                env,
                cwd: None,
                stdin: false,
                pty: false,
                cols: None,
                rows: None,
                timeout: None,
            },
            &uuid::Uuid::new_v4().to_string(),
            None,
        )?;
        let mut starting = Starting(Some(process.clone()));
        drop(reservations);
        let connected = async {
            let session = uuid::Uuid::new_v4().to_string();
            let iopub = jupyter_zmq_client::create_client_iopub_connection(&info, "", &session).await?;
            let identity = jupyter_zmq_client::peer_identity_for_session(&session)?;
            let shell =
                jupyter_zmq_client::create_client_shell_connection_with_identity(&info, &session, identity).await?;
            Ok::<_, jupyter_zmq_client::RuntimeError>(Channels { shell, iopub })
        };
        let mut channels = tokio::select! {
            result = tokio::time::timeout(STARTUP, connected) => result
                .map_err(|_| failure("kernel_start_timeout", "Kernel did not bind its channels"))?
                .map_err(|_| failure("kernel_start_failed", "Cannot connect kernel channels"))?,
            _ = process.wait() => return Err(failure("kernel_died", "Kernel exited during startup")),
        };
        let ready = async {
            loop {
                let request: JupyterMessage = KernelInfoRequest {}.into();
                let id = request.header.msg_id.clone();
                channels
                    .shell
                    .send(request)
                    .await
                    .map_err(|_| failure("kernel_protocol", "Kernel handshake failed"))?;
                let round = async {
                    let (shell, iopub) = (&mut channels.shell, &mut channels.iopub);
                    let (mut replied, mut idle) = (false, false);
                    while !replied || !idle {
                        tokio::select! {
                            result = shell.read() => {
                                let message = result.map_err(|_| failure("kernel_protocol", "Invalid kernel reply"))?;
                                if parent(&message, &id) && matches!(message.content, JupyterMessageContent::KernelInfoReply(_)) { replied = true; }
                            },
                            result = iopub.read() => {
                                let message = result.map_err(|_| failure("kernel_protocol", "Invalid kernel publication"))?;
                                if parent(&message, &id) && matches!(message.content, JupyterMessageContent::Status(ref s) if s.execution_state == ExecutionState::Idle) { idle = true; }
                            },
                            _ = process.wait() => return Err(failure("kernel_died", "Kernel exited during startup")),
                        }
                    }
                    Ok::<_, Failure>(())
                };
                if let Ok(result) = tokio::time::timeout(Duration::from_millis(500), round).await {
                    break result;
                }
            }
        };
        tokio::time::timeout(STARTUP, ready)
            .await
            .map_err(|_| failure("kernel_start_timeout", "Kernel did not become ready"))??;
        starting.0.take();
        Ok(Arc::new(Self {
            process,
            cancel: CancellationToken::new(),
            busy: AtomicBool::new(false),
            jobs: Arc::new(Semaphore::new(16)),
            channels: Mutex::new(channels),
            _directory: directory,
        }))
    }

    pub(crate) fn interrupt(&self) -> Result<(), Failure> {
        if !self.process.view().running {
            return Err(failure("kernel_died", "Kernel has exited; restart it explicitly"));
        }
        self.process.signal(libc::SIGINT)?;
        Ok(())
    }

    pub(crate) async fn stop(&self) -> Result<(), Failure> {
        self.cancel.cancel();
        if self.process.view().running {
            let _ = self.process.signal(libc::SIGKILL);
        }
        tokio::time::timeout(STOP, self.process.wait())
            .await
            .map_err(|_| failure("kernel_termination_unconfirmed", "Kernel termination is not yet confirmed"))?;
        Ok(())
    }

    pub(crate) async fn execute(&self, code: String, limit: Duration, sender: mpsc::Sender<KernelEvent>) {
        let began = Instant::now();
        let locked = tokio::time::timeout(limit, self.channels.lock()).await;
        let Ok(mut channels) = locked else {
            terminal(&sender, "KernelQueueTimeout", "Execution did not start before its deadline", "timeout").await;
            return;
        };
        if self.cancel.is_cancelled() || !self.process.view().running {
            terminal(&sender, "KernelDied", "Kernel is not running; restart it explicitly", "dead").await;
            return;
        }
        self.busy.store(true, Ordering::Release);
        let _busy = Busy(&self.busy);
        let mut request = ExecuteRequest::new(code);
        request.allow_stdin = false;
        let request: JupyterMessage = request.into();
        let id = request.header.msg_id.clone();
        let work = async {
            channels
                .shell
                .send(request)
                .await
                .map_err(|_| failure("kernel_protocol", "Cannot submit kernel execution"))?;
            let Channels { shell, iopub } = &mut *channels;
            let (mut reply, mut idle, mut bytes, mut saw_error) = (None, false, 0usize, false);
            while reply.is_none() || !idle {
                tokio::select! {
                    result = shell.read() => {
                        let message = result.map_err(|_| failure("kernel_protocol", "Invalid kernel reply"))?;
                        if parent(&message, &id) && let JupyterMessageContent::ExecuteReply(value) = message.content {
                            reply = Some(value);
                        }
                    },
                    result = iopub.read() => {
                        let message = result.map_err(|_| failure("kernel_protocol", "Invalid kernel publication"))?;
                        if !parent(&message, &id) { continue; }
                        if let JupyterMessageContent::Status(ref s) = message.content
                            && s.execution_state == ExecutionState::Idle { idle = true; }
                        if let Some(event) = events::project(message.content).map_err(|_| failure("kernel_protocol", "Invalid kernel MIME bundle"))? {
                            saw_error |= matches!(event, KernelEvent::Error { .. });
                            bytes = bytes.saturating_add(serde_json::to_vec(&event).expect("KernelEvent serializes").len());
                            if bytes > crate::contract::CAPTURE_BYTES { return Err(failure("output_overflow", "Kernel output exceeded capture budget")); }
                            publish(&sender, event).await?;
                        }
                    },
                    _ = self.process.wait() => return Err(failure("kernel_died", "Kernel exited during execution")),
                    _ = self.cancel.cancelled() => return Err(failure("kernel_closed", "Kernel was closed or restarted")),
                }
            }
            if let Some(reply) = reply {
                if let Some(error) = &reply.error
                    && !saw_error
                {
                    publish(
                        &sender,
                        KernelEvent::Error {
                            name: error.ename.clone(),
                            value: error.evalue.clone(),
                            traceback: error.traceback.clone(),
                        },
                    )
                    .await?;
                }
                if let Some(done) =
                    events::project(JupyterMessageContent::ExecuteReply(reply)).expect("Reply serializes")
                {
                    publish(&sender, done).await?;
                }
            }
            Ok::<_, Failure>(())
        };
        match tokio::time::timeout(limit.saturating_sub(began.elapsed()), work).await {
            Ok(Ok(())) => {}
            Ok(Err(Failure(_, code, message))) => {
                let _ = self.stop().await;
                terminal(&sender, code, &message, "dead").await;
            }
            Err(_) => {
                let _ = self.stop().await;
                terminal(&sender, "KernelTimeout", "Execution timed out; restart the kernel explicitly", "timeout")
                    .await;
            }
        }
    }

    pub(crate) async fn complete(&self, code: String) -> Result<serde_json::Value, Failure> {
        let work = async {
            let mut channels = self.channels.lock().await;
            let request: JupyterMessage = jupyter_protocol::IsCompleteRequest { code }.into();
            let id = request.header.msg_id.clone();
            channels
                .shell
                .send(request)
                .await
                .map_err(|_| failure("kernel_protocol", "Cannot submit completeness query"))?;
            loop {
                tokio::select! {
                    result = channels.shell.read() => {
                        let message = result.map_err(|_| failure("kernel_protocol", "Invalid completeness reply"))?;
                        if parent(&message, &id) && let JupyterMessageContent::IsCompleteReply(value) = message.content {
                            return serde_json::to_value(value).map_err(|_| failure("kernel_protocol", "Invalid completeness reply"));
                        }
                    },
                    _ = self.process.wait() => return Err(failure("kernel_died", "Kernel has exited")),
                    _ = self.cancel.cancelled() => return Err(failure("kernel_closed", "Kernel was closed")),
                }
            }
        };
        tokio::time::timeout(STOP, work)
            .await
            .map_err(|_| failure("kernel_busy", "Kernel is busy; completeness query did not execute"))?
    }
}

fn parent(message: &JupyterMessage, id: &str) -> bool {
    message.parent_header.as_ref().is_some_and(|header| header.msg_id == id)
}

async fn publish(sender: &mpsc::Sender<KernelEvent>, event: KernelEvent) -> Result<(), Failure> {
    // A disconnected caller does not replay or cancel code: continue draining it within its limit.
    let _ = tokio::time::timeout(BACKPRESSURE, sender.send(event))
        .await
        .map_err(|_| failure("output_overflow", "Kernel reader exceeded the backpressure deadline"))?;
    Ok(())
}

async fn terminal(sender: &mpsc::Sender<KernelEvent>, name: &str, message: &str, status: &str) {
    let _ = publish(sender, KernelEvent::failure(name, message)).await;
    let _ = publish(sender, KernelEvent::Done { execution_count: None, status: status.into() }).await;
}
