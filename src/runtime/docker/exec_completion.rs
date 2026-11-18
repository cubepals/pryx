// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private completion transport and failure cleanup for gVisor exec owners. The key never
//! enters argv, the container environment, child stdin or diagnostics. Unconfirmed completion
//! retires the sandbox once, including when the API caller drops its future.
//! Command submission and output collection remain in exec.rs.
use super::*;
use crate::runtime::parse_time;
use pryx_agent::exec_completion::{Completion, Key, TAIL_BYTES};
use tokio::io::{AsyncWrite, AsyncWriteExt};

/// Starts and failed-exec retirement serialize only their mutations, never an exec's lifetime.
#[derive(Clone, Default)]
pub(super) struct Starts(std::sync::Arc<std::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>);
impl Starts {
    pub(super) async fn lock(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut held = self.0.lock().unwrap();
            held.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = held.get(id).and_then(std::sync::Weak::upgrade) {
                lock
            } else {
                let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
                held.insert(id.into(), std::sync::Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }
}
struct Target {
    docker: Docker,
    container: String,
    start: OffsetDateTime,
    starts: Starts,
}

pub(super) struct Session {
    key: Key,
    input: [u8; pryx_agent::exec_completion::KEY_BYTES],
    tail: Vec<u8>,
    retirement: Option<Target>,
}
impl Session {
    pub(super) fn new(
        docker: Docker,
        container: &str,
        start: OffsetDateTime,
        starts: Starts,
    ) -> Result<Self, RuntimeError> {
        let (key, input) =
            Key::fresh().map_err(|_| RuntimeError::Other("exec completion key generation failed".into()))?;
        Ok(Self {
            key,
            input,
            tail: Vec::new(),
            retirement: Some(Target { docker, container: container.into(), start, starts }),
        })
    }
    pub(super) async fn deliver(&mut self, input: &mut (impl AsyncWrite + Unpin)) -> std::io::Result<()> {
        input.write_all(&self.input).await?;
        self.input.fill(0);
        input.shutdown().await
    }
    pub(super) async fn establish<S>(&mut self, output: &mut S, input: &mut (impl AsyncWrite + Unpin)) -> bool
    where
        S: futures_util::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Unpin,
    {
        // The execute-only image is protected by the kernel before visibility. READY only
        // establishes the actual exec inside the fenced generation, never channel protection.
        let established = tokio::time::timeout(EXEC_PID_WAIT, async {
            let expected = pryx_agent::exec_completion::READY;
            let mut ready = Vec::new();
            while ready.len() < expected.len() {
                let Some(Ok(LogOutput::StdErr { message })) = output.next().await else {
                    return false;
                };
                ready.extend_from_slice(&message);
                if !expected.starts_with(&ready) {
                    return false;
                }
            }
            true
        })
        .await;
        matches!(established, Ok(true)) && matches!(tokio::time::timeout(QUICK, self.deliver(input)).await, Ok(Ok(())))
    }
    pub(super) fn stderr(&mut self, bytes: &[u8]) -> bytes::Bytes {
        self.tail.extend_from_slice(bytes);
        let ready = self.tail.len().saturating_sub(TAIL_BYTES);
        self.tail.drain(..ready).collect::<Vec<_>>().into()
    }
    pub(super) fn finish(&mut self, exit: Option<i64>) -> Option<(Completion, bytes::Bytes)> {
        let completion = self.key.verify(&mut self.tail, exit)?;
        self.retirement = None;
        Some((completion, std::mem::take(&mut self.tail).into()))
    }
    pub(super) async fn complete(
        &mut self,
        result: &mut ExecOutput,
        max_output: usize,
        live: &Option<tokio::sync::mpsc::Sender<LogChunk>>,
    ) -> bool {
        let Some((confirmed, stderr)) = self.finish(result.exit_code) else {
            return false;
        };
        result.timed_out = confirmed.timed_out;
        result.killed = confirmed.timed_out;
        let room = max_output.saturating_sub(result.stderr.len());
        result.stderr.extend_from_slice(&stderr[..stderr.len().min(room)]);
        result.stderr_truncated |= stderr.len() > room;
        let visible = stderr.slice(..stderr.len().min(room));
        if let Some(live) = live
            && !visible.is_empty()
        {
            let _ = live.send(LogChunk { stream: LogStream::Stderr, bytes: visible }).await;
        }
        true
    }
    pub(super) async fn failed(&mut self) -> RuntimeError {
        let Some(target) = self.retirement.take() else {
            return RuntimeError::Other("exec-timeout cleanup was not authenticated".into());
        };
        let retired = tokio::spawn(retire(target)).await.unwrap_or(false);
        RuntimeError::Other(
            if retired {
                "exec-timeout cleanup was not authenticated; the gvisor sandbox was retired"
            } else {
                "exec-timeout cleanup was not authenticated and sandbox retirement could not be confirmed"
            }
            .into(),
        )
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if let Some(target) = self.retirement.take() {
            tokio::spawn(async move {
                if !retire(target).await {
                    tracing::warn!("unconfirmed gvisor exec cleanup could not retire its sandbox");
                }
            });
        }
    }
}

/// One kill submission, then read-only confirmation. A lost response never repeats the kill.
async fn retire(target: Target) -> bool {
    let _generation = target.starts.lock(&target.container).await;
    tokio::time::timeout(Duration::from_secs(5), async move {
        let current = match target.docker.inspect_container(&target.container, None).await {
            Ok(info) => info.state,
            Err(error) => return matches!(map_err(error), RuntimeError::NotFound(_)),
        };
        let Some(current) = current else {
            return false;
        };
        if current.started_at.as_deref().and_then(|start| parse_time(Some(start))) != Some(target.start) {
            // A replacement start already retired the original gVisor kernel and descendants.
            return current.started_at.as_deref().and_then(|start| parse_time(Some(start))).is_some();
        }
        if matches!(
            current.status,
            Some(bollard::models::ContainerStateStatusEnum::EXITED | bollard::models::ContainerStateStatusEnum::DEAD)
        ) {
            return true;
        }
        let options = KillContainerOptionsBuilder::new().signal("SIGKILL").build();
        let _ = target.docker.kill_container(&target.container, Some(options)).await;
        loop {
            match target.docker.inspect_container(&target.container, None).await {
                Ok(info)
                    if info.state.as_ref().is_some_and(|state| {
                        matches!(
                            state.status,
                            Some(
                                bollard::models::ContainerStateStatusEnum::EXITED
                                    | bollard::models::ContainerStateStatusEnum::DEAD
                            )
                        )
                    }) =>
                {
                    return true;
                }
                Err(error) => return matches!(map_err(error), RuntimeError::NotFound(_)),
                Ok(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::Request, routing::any};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn forged_completion_retires_the_sandbox_once() {
        failure_after_start(false).await;
    }
    #[tokio::test]
    async fn old_cleanup_waits_for_a_start_and_never_kills_its_replacement() {
        failure_after_start(true).await;
    }
    #[tokio::test]
    async fn a_verified_stderr_tail_obeys_the_existing_live_output_bound() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let docker = Docker::connect_with_unix(path.to_str().unwrap(), 2, bollard::API_DEFAULT_VERSION).unwrap();
        let mut session = Session::new(docker, "owned", OffsetDateTime::now_utc(), Starts::default()).unwrap();
        let mut tail = b"visible tail".to_vec();
        session.key.write(&mut tail, 0, false).unwrap();
        assert!(session.stderr(&tail).is_empty());
        let mut result = ExecOutput { stderr: b"prior".to_vec(), exit_code: Some(0), ..Default::default() };
        let (live, mut chunks) = tokio::sync::mpsc::channel(2);
        assert!(session.complete(&mut result, 6, &Some(live)).await);
        assert_eq!(result.stderr, b"priorv");
        assert!(result.stderr_truncated);
        assert_eq!(chunks.try_recv().unwrap().bytes.as_ref(), b"v");
        assert!(chunks.try_recv().is_err(), "no retained bytes may exceed the stream's room");
    }
    async fn failure_after_start(replaced: bool) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let kills = Arc::new(AtomicUsize::new(0));
        let observed = kills.clone();
        let initial = OffsetDateTime::now_utc();
        let stamp = Arc::new(std::sync::Mutex::new(initial));
        let current = stamp.clone();
        let app = Router::new().fallback(any(move |request: Request| {
            let observed = observed.clone();
            let start = *current.lock().unwrap();
            async move {
                if request.uri().path().ends_with("/kill") {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
                Json(bollard::models::ContainerInspectResponse {
                    state: Some(bollard::models::ContainerState {
                        status: Some(if observed.load(Ordering::SeqCst) == 0 {
                            bollard::models::ContainerStateStatusEnum::RUNNING
                        } else {
                            bollard::models::ContainerStateStatusEnum::EXITED
                        }),
                        started_at: Some(start.format(&time::format_description::well_known::Rfc3339).unwrap()),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let docker = Docker::connect_with_unix(path.to_str().unwrap(), 2, bollard::API_DEFAULT_VERSION).unwrap();
        let starts = Starts::default();
        let generation = starts.lock("owned").await;
        let mut session = Session::new(docker, "owned", initial, starts).unwrap();
        let code = 128 + libc::SIGKILL;
        let mut forged = Vec::new();
        Key::fresh().unwrap().0.write(&mut forged, code, true).unwrap();
        session.stderr(&forged);
        assert!(session.finish(Some(i64::from(code))).is_none());
        let pending = tokio::spawn(async move { session.failed().await });
        tokio::task::yield_now().await;
        assert_eq!(kills.load(Ordering::SeqCst), 0, "cleanup serializes with an admitted start");
        if replaced {
            *stamp.lock().unwrap() = initial + time::Duration::seconds(1);
        }
        drop(generation);
        assert!(pending.await.unwrap().to_string().contains("sandbox was retired"));
        assert_eq!(
            kills.load(Ordering::SeqCst),
            usize::from(!replaced),
            "never kill a replacement or repeat a mutation"
        );
        server.abort();
    }
}
