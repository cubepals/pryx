// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private completion transport and failure cleanup for gVisor exec owners. The key never
//! enters argv, the container environment, child stdin or diagnostics. The sandbox is retired
//! once, on evidence that the in-sandbox supervisor did not finish: the exec ended without its
//! authenticated completion, or it still runs past its bound. A lost engine answer or output
//! stream is reported, never answered with a kill, since the supervisor keeps the deadline
//! whether or not this node hears it. A future dropped before its exec is judged still retires
//! it; the node's API runs each exec detached, so a caller that goes away drops none. Command
//! submission remains in exec.rs; bounded output collection, which both exec paths share, is
//! here beside the completion it carries.
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
    /// The key was offered to the supervisor: from then on it may run the command.
    offered: bool,
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
            offered: false,
        })
    }
    pub(super) async fn deliver(&mut self, input: &mut (impl AsyncWrite + Unpin)) -> std::io::Result<()> {
        // Even a failed write may have handed it over whole.
        self.offered = true;
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
    /// Nothing says the supervisor failed, so its sandbox is kept: the command never started, or
    /// this node could not hear how it ended.
    fn release(&mut self) {
        self.retirement = None;
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

/// Failed private transports retire only the exact start they served, using this same
/// serialized, single-submission engine authority as exec completion.
pub(super) async fn retire_container(docker: Docker, container: String, start: OffsetDateTime, starts: Starts) -> bool {
    retire(Target { docker, container, start, starts }).await
}

// Docker exposes exec inspection, but no exec wait endpoint. Avoid a 20 ms quantization after EOF.
pub(super) const EXIT_POLL: Duration = Duration::from_millis(4);
// While output is still open, a long-running exec needs only a few engine requests a second.
const RUNNING_POLL: Duration = Duration::from_millis(250);

pub(super) async fn collect_output<S>(
    output: &mut S,
    result: &mut ExecOutput,
    max_output: usize,
    live: &Option<tokio::sync::mpsc::Sender<LogChunk>>,
    completion: &mut Option<Session>,
) -> Result<(), RuntimeError>
where
    S: futures_util::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Unpin,
{
    while let Some(item) = output.next().await {
        let item = match item.map_err(map_err)? {
            LogOutput::StdErr { message } if completion.is_some() => {
                LogOutput::StdErr { message: completion.as_mut().unwrap().stderr(&message) }
            }
            other => other,
        };
        let (buffer, truncated, stream, bytes) = match item {
            LogOutput::StdOut { message } => {
                (&mut result.stdout, &mut result.stdout_truncated, LogStream::Stdout, message)
            }
            LogOutput::StdErr { message } => {
                (&mut result.stderr, &mut result.stderr_truncated, LogStream::Stderr, message)
            }
            _ => continue,
        };
        let room = max_output.saturating_sub(buffer.len());
        buffer.extend_from_slice(&bytes[..bytes.len().min(room)]);
        *truncated |= bytes.len() > room;
        if let Some(live) = &live {
            for chunk in bytes[..bytes.len().min(room)].chunks(crate::protocol::EXEC_CHUNK_BYTES) {
                if live.send(LogChunk { stream, bytes: bytes::Bytes::copy_from_slice(chunk) }).await.is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}

/// One authenticated exec, created but not yet started, and what is needed to see it to its
/// end: the start lock it was created under, and its bound (deadline and grace).
pub(super) struct Owned {
    pub(super) docker: Docker,
    pub(super) exec_id: String,
    pub(super) session: Session,
    pub(super) setup: tokio::sync::OwnedMutexGuard<()>,
    pub(super) limit: Duration,
    pub(super) max_output: usize,
    pub(super) live: Option<tokio::sync::mpsc::Sender<LogChunk>>,
}

/// How long the attached output may still flow once the engine says the exec has exited: Moby
/// reports the exit before it finishes copying the last output, and closes the stream two seconds
/// later even while something else holds it open.
const DRAIN: Duration = Duration::from_secs(5);
/// Between attempts at an engine call that failed.
const ENGINE_RETRY: Duration = Duration::from_millis(100);
/// How long a failing engine is asked again once the exec's bound has passed.
const LAST_ASK: Duration = Duration::from_secs(5);

impl Owned {
    pub(super) async fn run(self) -> Result<ExecOutput, RuntimeError> {
        let Self { docker, exec_id, mut session, setup, limit, max_output, live } = self;
        let bound = tokio::time::Instant::now() + limit;
        let options = StartExecOptions { detach: false, tty: false, output_capacity: None };
        // A start that fails delivers no key, and without one the supervisor runs nothing.
        let (mut output, mut input) = match quick("start exec", docker.start_exec(&exec_id, Some(options))).await {
            Ok(StartExecResults::Attached { output, input }) => (output, input),
            Ok(StartExecResults::Detached) => {
                session.release();
                return Err(RuntimeError::Other("exec started detached".into()));
            }
            Err(problem) => {
                session.release();
                return Err(problem);
            }
        };
        let established = session.establish(&mut output, &mut input).await;
        drop(input);
        drop(setup);
        if !established && !session.offered {
            session.release();
            return Err(RuntimeError::Other(
                "the exec's supervisor did not start, so it was never given a command to run".into(),
            ));
        }
        watch(&docker, &exec_id, output, session, bound, max_output, live).await
    }
}

/// From an established exec's output and the engine's word on it, its verified result, or the
/// sandbox retired when the supervisor did not finish.
async fn watch<S>(
    docker: &Docker,
    exec_id: &str,
    mut output: S,
    session: Session,
    bound: tokio::time::Instant,
    max_output: usize,
    live: Option<tokio::sync::mpsc::Sender<LogChunk>>,
) -> Result<ExecOutput, RuntimeError>
where
    S: futures_util::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Unpin,
{
    let mut result = ExecOutput::default();
    let mut completion = Some(session);
    let (live, forwarding) = relay(live);
    let (streamed, exit) = {
        let collect = collect_output(&mut output, &mut result, max_output, &live, &mut completion);
        let exit = exited(docker, exec_id, RUNNING_POLL);
        tokio::pin!(collect, exit);
        tokio::select! {
            streamed = &mut collect => {
                exit.set(exited(docker, exec_id, EXIT_POLL));
                (Some(streamed), tokio::time::timeout_at(bound, exit).await.ok())
            },
            code = &mut exit => match tokio::time::timeout(DRAIN, &mut collect).await {
                Ok(streamed) => (Some(streamed), Some(code)),
                Err(_) => {
                    // A client that reads slowly is let go of, so the output behind it, and its
                    // completion, are still read; what it missed arrives with the result.
                    if let Some(forwarding) = &forwarding {
                        forwarding.abort();
                    }
                    (tokio::time::timeout(DRAIN, collect).await.ok(), Some(code))
                }
            },
            () = tokio::time::sleep_until(bound) => (None, None),
        }
    };
    drop(output);
    let mut session = completion.take().expect("output collection keeps the session");
    let code = match exit {
        Some(code) => code,
        None => match state(docker, exec_id).await {
            Ok(Some(code)) => code,
            // Still running past its bound: the supervisor never finished retiring the command.
            Ok(None) => return Err(session.failed().await),
            Err(problem) => {
                session.release();
                return Err(unheard(&problem));
            }
        },
    };
    if let Some(Err(problem)) = streamed {
        session.release();
        return Err(unheard(&problem));
    }
    result.exit_code = code;
    if session.complete(&mut result, max_output, &live).await { Ok(result) } else { Err(session.failed().await) }
}

/// Live output through a forwarder of its own, which can be let go of when the client it waits on
/// stops reading; the client's stream then sends what it missed from the result.
fn relay(
    live: Option<tokio::sync::mpsc::Sender<LogChunk>>,
) -> (Option<tokio::sync::mpsc::Sender<LogChunk>>, Option<tokio::task::JoinHandle<()>>) {
    let Some(live) = live else { return (None, None) };
    let (relay, mut queued) = tokio::sync::mpsc::channel::<LogChunk>(1);
    let forwarding = tokio::spawn(async move {
        while let Some(chunk) = queued.recv().await {
            if live.send(chunk).await.is_err() {
                return;
            }
        }
    });
    (Some(relay), Some(forwarding))
}

/// The exec's exit code once the engine says it has exited, asking again after a failed answer.
async fn exited(docker: &Docker, exec_id: &str, poll: Duration) -> Option<i64> {
    loop {
        match quick("inspect exec", docker.inspect_exec(exec_id)).await {
            Ok(inspect) if inspect.running != Some(true) => return inspect.exit_code,
            Ok(_) => tokio::time::sleep(poll).await,
            Err(_) => tokio::time::sleep(ENGINE_RETRY).await,
        }
    }
}

/// Whether the exec has exited (and its code) or still runs, as the engine says now; an engine
/// that fails is asked again for a little while.
async fn state(docker: &Docker, exec_id: &str) -> Result<Option<Option<i64>>, RuntimeError> {
    let until = tokio::time::Instant::now() + LAST_ASK;
    loop {
        match quick("inspect exec", docker.inspect_exec(exec_id)).await {
            Ok(inspect) if inspect.running == Some(true) => return Ok(None),
            Ok(inspect) => return Ok(Some(inspect.exit_code)),
            Err(problem) if tokio::time::Instant::now() >= until => return Err(problem),
            Err(_) => tokio::time::sleep(ENGINE_RETRY).await,
        }
    }
}

/// An exec whose completion this node could not hear: unconfirmed, but nothing says it failed.
fn unheard(problem: &dyn std::fmt::Display) -> RuntimeError {
    RuntimeError::Other(format!(
        "exec completion was not heard ({problem}); the sandbox's own deadline still applies to the command"
    ))
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
    use axum::{Json, Router, extract::Request, response::IntoResponse, routing::any};
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
    /// One exec's engine: its inspection fails `failures` times, then says it runs while `running`
    /// holds and otherwise that it exited with `code`; its container runs until it is killed.
    struct Engine {
        kills: Arc<AtomicUsize>,
        inspections: Arc<AtomicUsize>,
        running: Arc<std::sync::atomic::AtomicBool>,
        docker: Docker,
        start: OffsetDateTime,
        server: tokio::task::JoinHandle<()>,
        _dir: tempfile::TempDir,
    }

    impl Engine {
        fn new(failures: usize, code: i64) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("engine.sock");
            let listener = tokio::net::UnixListener::bind(&path).unwrap();
            let (kills, running) = (Arc::new(AtomicUsize::new(0)), Arc::new(std::sync::atomic::AtomicBool::new(false)));
            let inspections = Arc::new(AtomicUsize::new(0));
            let (killed, runs, inspected) = (kills.clone(), running.clone(), inspections.clone());
            let start = OffsetDateTime::now_utc();
            let app = Router::new().fallback(any(move |request: Request| {
                let (killed, runs, inspected) = (killed.clone(), runs.clone(), inspected.clone());
                async move {
                    let path = request.uri().path().to_owned();
                    if path.ends_with("/kill") {
                        killed.fetch_add(1, Ordering::SeqCst);
                    }
                    if path.contains("/exec/") {
                        if inspected.fetch_add(1, Ordering::SeqCst) < failures {
                            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
                        }
                        let running = runs.load(Ordering::SeqCst);
                        let exec = bollard::models::ExecInspectResponse {
                            running: Some(running),
                            exit_code: (!running).then_some(code),
                            ..Default::default()
                        };
                        return Json(exec).into_response();
                    }
                    let status = if killed.load(Ordering::SeqCst) == 0 {
                        bollard::models::ContainerStateStatusEnum::RUNNING
                    } else {
                        bollard::models::ContainerStateStatusEnum::EXITED
                    };
                    let started = start.format(&time::format_description::well_known::Rfc3339).unwrap();
                    Json(bollard::models::ContainerInspectResponse {
                        state: Some(bollard::models::ContainerState {
                            status: Some(status),
                            started_at: Some(started),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .into_response()
                }
            }));
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let docker = Docker::connect_with_unix(path.to_str().unwrap(), 2, bollard::API_DEFAULT_VERSION).unwrap();
            Self { kills, inspections, running, docker, start, server, _dir: dir }
        }

        fn session(&self) -> Session {
            Session::new(self.docker.clone(), "owned", self.start, Starts::default()).unwrap()
        }

        /// An exec watched to its end, its output what `output` yields, its bound `limit` away.
        async fn watch(
            &self,
            session: Session,
            output: impl futures_util::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Unpin,
            limit: Duration,
        ) -> Result<ExecOutput, RuntimeError> {
            self.watch_live(session, output, limit, None).await
        }

        async fn watch_live(
            &self,
            session: Session,
            output: impl futures_util::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Unpin,
            limit: Duration,
            live: Option<tokio::sync::mpsc::Sender<LogChunk>>,
        ) -> Result<ExecOutput, RuntimeError> {
            let bound = tokio::time::Instant::now() + limit;
            watch(&self.docker, "exec", output, session, bound, 1024, live).await
        }
    }

    impl Drop for Engine {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    fn stderr(message: Vec<u8>) -> Result<LogOutput, bollard::errors::Error> {
        Ok(LogOutput::StdErr { message: message.into() })
    }

    #[tokio::test]
    async fn an_engine_answer_that_failed_is_asked_again_and_kills_nothing() {
        let engine = Engine::new(1, 0);
        let session = engine.session();
        let mut footer = b"done".to_vec();
        session.key.write(&mut footer, 0, false).unwrap();
        let result = engine.watch(session, futures_util::stream::iter([stderr(footer)]), Duration::from_secs(10));
        let result = result.await.unwrap();
        assert_eq!((result.exit_code, result.stderr.as_slice()), (Some(0), b"done".as_slice()));
        assert_eq!(engine.kills.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_exec_with_open_output_does_not_flood_the_engine() {
        let engine = Engine::new(0, 0);
        engine.running.store(true, Ordering::SeqCst);
        let session = engine.session();
        let mut footer = Vec::new();
        session.key.write(&mut footer, 0, false).unwrap();
        let (finish, output) = tokio::sync::oneshot::channel();
        let output = Box::pin(futures_util::stream::once(async { stderr(output.await.unwrap()) }));
        let (result, ()) = tokio::join!(engine.watch(session, output, Duration::from_secs(10)), async {
            tokio::time::timeout(Duration::from_secs(2), async {
                while engine.inspections.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("the running exec was inspected");
            tokio::time::sleep(Duration::from_millis(80)).await;
            assert_eq!(engine.inspections.load(Ordering::SeqCst), 1, "open output allows slow inspection");
            engine.running.store(false, Ordering::SeqCst);
            finish.send(footer).unwrap();
        });
        assert_eq!(result.unwrap().exit_code, Some(0));
        assert_eq!(engine.kills.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_client_that_stops_reading_never_stands_for_a_failed_completion() {
        let engine = Engine::new(0, 0);
        let session = engine.session();
        let mut footer = Vec::new();
        session.key.write(&mut footer, 0, false).unwrap();
        let mut output: Vec<_> = (0..4).map(|_| Ok(LogOutput::StdOut { message: b"line\n".to_vec().into() })).collect();
        output.push(stderr(footer));
        // The client takes nothing: its queue fills with the first chunk.
        let (live, _unread) = tokio::sync::mpsc::channel(1);
        let began = std::time::Instant::now();
        let result =
            engine.watch_live(session, futures_util::stream::iter(output), Duration::from_secs(60), Some(live));
        let result = result.await.unwrap();
        assert_eq!((result.exit_code, result.stdout.as_slice()), (Some(0), b"line\n".repeat(4).as_slice()));
        assert!(began.elapsed() < Duration::from_secs(30), "the slow client was let go of, not waited for");
        assert_eq!(engine.kills.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_full_live_queue_does_not_hold_a_verified_stderr_tail() {
        let engine = Engine::new(0, 0);
        let session = engine.session();
        let mut footer = b"final stderr\n".to_vec();
        session.key.write(&mut footer, 0, false).unwrap();
        // One chunk fills the client queue, one holds the forwarder, and one fills its queue.
        let lines = (0..3).map(|_| Ok(LogOutput::StdOut { message: b"line\n".to_vec().into() }));
        let mut drained = false;
        let output =
            futures_util::stream::iter(lines.chain([stderr(footer)])).chain(futures_util::stream::poll_fn(|_| {
                drained = true;
                std::task::Poll::Ready(None)
            }));
        let (live, _unread) = tokio::sync::mpsc::channel(1);
        let completed = tokio::time::timeout(
            Duration::from_secs(2),
            engine.watch_live(session, output, Duration::from_secs(60), Some(live)),
        )
        .await;
        assert!(drained, "the engine output and signed footer were drained");
        assert!(engine.inspections.load(Ordering::SeqCst) > 0, "the engine reported completion");
        assert_eq!(engine.kills.load(Ordering::SeqCst), 0);
        let result = completed.expect("verified completion must return with the client queue full").unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout, b"line\n".repeat(3));
        assert_eq!(result.stderr, b"final stderr\n");
    }

    #[tokio::test]
    async fn an_exec_that_ended_without_its_completion_retires_its_sandbox_once() {
        let engine = Engine::new(0, i64::from(128 + libc::SIGKILL));
        let output = futures_util::stream::iter([stderr(b"no completion".to_vec())]);
        let refused = engine.watch(engine.session(), output, Duration::from_secs(10)).await.unwrap_err();
        assert!(refused.to_string().contains("sandbox was retired"), "{refused}");
        assert_eq!(engine.kills.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_lost_output_stream_is_reported_and_kills_nothing() {
        let engine = Engine::new(0, 0);
        let lost = bollard::errors::Error::IOError { err: std::io::Error::other("attach stream reset") };
        let output = futures_util::stream::iter([stderr(b"partial".to_vec()), Err(lost)]);
        let refused = engine.watch(engine.session(), output, Duration::from_secs(10)).await.unwrap_err();
        assert!(refused.to_string().contains("not heard"), "{refused}");
        assert_eq!(engine.kills.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_exec_still_running_past_its_bound_retires_its_sandbox() {
        let engine = Engine::new(0, 0);
        engine.running.store(true, Ordering::SeqCst);
        let output = futures_util::stream::pending();
        let refused = engine.watch(engine.session(), output, Duration::from_millis(200)).await.unwrap_err();
        assert!(refused.to_string().contains("sandbox was retired"), "{refused}");
        assert_eq!(engine.kills.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_exited_exec_whose_output_stays_open_is_judged_as_it_exits() {
        // Its supervisor was killed; what it started still holds the output open.
        let engine = Engine::new(0, i64::from(128 + libc::SIGKILL));
        let began = std::time::Instant::now();
        let output = futures_util::stream::iter([stderr(b"started".to_vec())]).chain(futures_util::stream::pending());
        let refused = engine.watch(engine.session(), output, Duration::from_secs(60)).await.unwrap_err();
        assert!(refused.to_string().contains("sandbox was retired"), "{refused}");
        assert!(began.elapsed() < Duration::from_secs(30), "judged at its exit, not at its bound");
        assert_eq!(engine.kills.load(Ordering::SeqCst), 1);
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
