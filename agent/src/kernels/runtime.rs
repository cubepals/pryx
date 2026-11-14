// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Managed kernel processes and maintained Jupyter channels. User code is sent once; only the
//! side-effect-free kernel-info startup handshake may repeat while subscriptions become ready.
//!
//! Parts (`runtime/`):
//! - `start.rs`: supervised process launch and maintained channel readiness.
mod start;

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
