// SPDX-License-Identifier: FSL-1.1-ALv2
//! Durable transitions own each VM until creation or disposal is acknowledged. A cancelled caller
//! cannot clear pending ownership, and no replacement starts while a prior VM remains uncertain.

use super::*;
use futures_util::{StreamExt, stream};

impl WasmRuntime {
    pub(super) async fn launch(&self, name: &str) -> Result<(), RuntimeError> {
        let transition = self.transition(name)?;
        let _turn = transition.lock().await;
        let entry = self.entry(name)?;
        if self.state.live.lock().unwrap().contains_key(&entry.id) {
            return Ok(());
        }
        let mut options = boot::options(
            Some(&entry.spec),
            &self.state.distribution,
            &self.state.launcher,
            &self.state.pool,
            self.state.owner,
        )?;
        boot::packages(&mut options, &entry.spec.packages).await?;
        {
            let mut entries = self.state.entries.lock().unwrap();
            let current = entries.get_mut(&entry.id).ok_or_else(|| error("Wasm sandbox disappeared"))?;
            if current.pending {
                return Err(RuntimeError::Conflict("Wasm VM ownership is pending".into()));
            }
            current.pending = true;
        }
        self.save()?;
        let vm = match AgentOs::create(options).await {
            Ok(vm) => vm,
            Err(_) => {
                return Err(RuntimeError::Unavailable("agentOS VM creation failed; ownership remains pending".into()));
            }
        };
        let live = match host::start(vm.clone(), &entry.spec, self.state.ingress.clone(), self.state.owner).await {
            Ok(live) => Arc::new(live),
            Err(error) => {
                if vm.shutdown().await.is_err() {
                    tracing::error!("Failed wasm frontend retains uncertain VM ownership");
                } else {
                    self.state.entries.lock().unwrap().get_mut(&entry.id).expect("entry retained").pending = false;
                    self.save()?;
                }
                return Err(error);
            }
        };
        self.state.live.lock().unwrap().insert(entry.id.clone(), live);
        {
            let mut entries = self.state.entries.lock().unwrap();
            let current = entries.get_mut(&entry.id).expect("pending entry retained");
            current.pending = false;
            current.running = true;
            current.paused = false;
            current.code = 0;
            current.started = Some(format_time(OffsetDateTime::now_utc()));
            current.finished = None;
        }
        self.save()?;
        self.emit(&entry.id, "start");
        Ok(())
    }

    pub(super) async fn dispose(&self, name: &str, paused: bool) -> Result<(), RuntimeError> {
        self.finish(name, paused, false, None).await
    }

    pub(super) async fn finish(
        &self,
        name: &str,
        paused: bool,
        remove: bool,
        code: Option<i64>,
    ) -> Result<(), RuntimeError> {
        let own = self.clone();
        let name = name.to_owned();
        tokio::spawn(async move {
            let transition = own.transition(&name)?;
            let _turn = transition.lock().await;
            own.dispose_inner(&name, paused, remove, code).await
        })
        .await
        .map_err(error)?
    }

    async fn dispose_inner(
        &self,
        name: &str,
        paused: bool,
        remove: bool,
        code: Option<i64>,
    ) -> Result<(), RuntimeError> {
        let entry = self.entry(name)?;
        let live = self.state.live.lock().unwrap().get(&entry.id).cloned();
        if entry.pending {
            return Err(RuntimeError::Conflict("Wasm VM ownership is pending".into()));
        }
        if let Some(live) = live {
            self.state.entries.lock().unwrap().get_mut(&entry.id).expect("entry retained").pending = true;
            self.save()?;
            // Keep the live handle and pending journal on uncertain disposal. Never replace it.
            live.shutdown().await?;
            self.state.live.lock().unwrap().remove(&entry.id);
        }
        {
            let mut entries = self.state.entries.lock().unwrap();
            let current = entries.get_mut(&entry.id).expect("entry retained");
            current.pending = false;
            current.running = false;
            current.paused = paused;
            current.finished = Some(format_time(OffsetDateTime::now_utc()));
            if let Some(code) = code {
                current.code = code;
            }
            if remove {
                entries.remove(&entry.id);
                self.state.transitions.lock().unwrap().remove(&entry.id);
            }
        }
        self.save()?;
        self.emit(&entry.id, if paused { "pause" } else { DIED });
        Ok(())
    }

    pub(super) async fn inspect_entry(&self, name: &str) -> Result<Option<ContainerInfo>, RuntimeError> {
        let mut entry = match self.entry(name) {
            Ok(entry) => entry,
            Err(RuntimeError::NotFound(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        let live = self.state.live.lock().unwrap().get(&entry.id).cloned();
        if let Some(live) = live.filter(|_| !entry.pending) {
            match live.status.load(std::sync::atomic::Ordering::Acquire) {
                super::VM_UNCERTAIN => {
                    return Err(RuntimeError::Unavailable("Wasm VM termination remains uncertain".into()));
                }
                super::VM_DISPOSED => match self.finish(&entry.id, false, false, Some(exit_by(libc::SIGKILL))).await {
                    Ok(()) => {}
                    Err(RuntimeError::NotFound(_)) => return Ok(None),
                    Err(error) => return Err(error),
                },
                _ => {}
            }
            let keep_alive = entry.spec.agent.as_ref().is_some_and(|agent| agent.keep_alive);
            if !keep_alive && let Some(result) = live.entrypoint.as_ref().and_then(|p| p.view().result) {
                let code = result.code.map(i64::from).unwrap_or_else(|| exit_by(libc::SIGKILL));
                match self.finish(&entry.id, false, false, Some(code)).await {
                    Ok(()) => {}
                    Err(RuntimeError::NotFound(_)) => return Ok(None),
                    Err(error) => return Err(error),
                }
            }
        }
        entry = match self.entry(&entry.id) {
            Ok(entry) => entry,
            Err(RuntimeError::NotFound(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        let status = if entry.paused {
            ContainerStatus::Paused
        } else if entry.running {
            ContainerStatus::Running
        } else if entry.started.is_some() {
            ContainerStatus::Exited
        } else {
            ContainerStatus::Created
        };
        Ok(Some(ContainerInfo {
            id: entry.id,
            name: entry.spec.name,
            labels: entry.spec.labels,
            env: entry.spec.env.into_iter().collect(),
            status,
            exit_code: entry.code,
            oom_killed: false,
            started_at: parse_time(entry.started.as_deref()),
            finished_at: parse_time(entry.finished.as_deref()),
            restart_count: 0,
            health: None,
            cgroup_parent: None,
        }))
    }

    pub(super) async fn exec_stream_for(
        &self,
        name: &str,
        command: &[String],
        timeout: Duration,
        _: usize,
    ) -> Result<BoxStream<'static, Result<pryx_agent::process::Event, RuntimeError>>, RuntimeError> {
        let entry = self.entry(name)?;
        let live = self
            .state
            .live
            .lock()
            .unwrap()
            .get(&entry.id)
            .cloned()
            .ok_or_else(|| error("Wasm sandbox is not running"))?;
        let process = live
            .agent
            .processes
            .start(
                pryx_agent::process::Start {
                    command: pryx_agent::process::Command::Argv(command.into()),
                    env: Default::default(),
                    cwd: Some(entry.spec.mount_path),
                    stdin: false,
                    pty: false,
                    cols: None,
                    rows: None,
                    timeout: Some(u32::try_from(timeout.as_secs()).map_err(error)?),
                },
                &uuid::Uuid::new_v4().to_string(),
                None,
            )
            .map_err(error)?;
        let (replay, live) = process.subscribe().await;
        Ok(Box::pin(
            stream::iter(replay.into_iter().map(Ok)).chain(stream::unfold(live, |mut live| async move {
                live.recv().await.map(|event| (Ok(event), live))
            })),
        ))
    }

    pub(super) fn logs_for(
        &self,
        name: &str,
        options: LogOptions,
    ) -> BoxStream<'static, Result<LogChunk, RuntimeError>> {
        let result = (|| {
            let entry = self.entry(name)?;
            let live = self.state.live.lock().unwrap().get(&entry.id).cloned();
            let (replay, events) = match live {
                Some(live) => live.agent.logs.subscribe(),
                None => pryx_agent::logbook::Logbook::open(&entry.spec.data_dir.join(boot::CONTENT_DIRECTORY))
                    .map_err(error)?
                    .subscribe(),
            };
            Ok::<_, RuntimeError>((replay, events))
        })();
        let (replay, receiver) = match result {
            Ok(result) => result,
            Err(error) => return Box::pin(stream::once(async move { Err(error) })),
        };
        let mut chunks = replay
            .into_iter()
            .flat_map(|frame| match frame {
                pryx_agent::logbook::Frame::Replay { chunks } => chunks,
                pryx_agent::logbook::Frame::Chunk { chunk } => vec![chunk],
                _ => Vec::new(),
            })
            .filter(|chunk| options.since_unix.is_none_or(|since| chunk.at / 1000 >= since.max(0) as u64))
            .collect::<Vec<_>>();
        if let Some(tail) = options.tail {
            chunks = chunks.into_iter().rev().take(tail as usize).collect();
            chunks.reverse();
        }
        let live = stream::unfold(options.follow.then_some(receiver), |receiver| async move {
            let mut receiver = receiver?;
            loop {
                match receiver.recv().await {
                    Ok(pryx_agent::logbook::Frame::Chunk { chunk }) => return Some((log_chunk(chunk), Some(receiver))),
                    Ok(pryx_agent::logbook::Frame::Gap) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        return Some((Err(error("Wasm logs have a gap")), None));
                    }
                    Ok(_) => {}
                    Err(_) => return None,
                }
            }
        });
        Box::pin(stream::iter(chunks.into_iter().map(log_chunk)).chain(live))
    }
}

fn log_chunk(chunk: pryx_agent::logbook::Chunk) -> Result<LogChunk, RuntimeError> {
    use base64::Engine;
    let content = base64::engine::general_purpose::STANDARD.decode(chunk.bytes).map_err(error)?;
    let timestamp = OffsetDateTime::from_unix_timestamp_nanos(i128::from(chunk.at) * 1_000_000).map_err(error)?;
    let mut bytes = format!("{} ", format_time(timestamp)).into_bytes();
    bytes.extend(content);
    Ok(LogChunk {
        stream: if chunk.stream == "stderr" { LogStream::Stderr } else { LogStream::Stdout },
        bytes: bytes.into(),
    })
}
