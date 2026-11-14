// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Authenticated entrypoint log streams use the guest's existing bounded replay/gap protocol.
//! No serial diagnostics, bootstrap inputs or commands become application output. Stopped-disk
//! replay belongs to the manager's admitted data session, not an unaccounted guest here.
use super::*;
use crate::runtime::{ContainerStatus, LogChunk, LogOptions, logs::records};
use futures_util::{StreamExt, stream};
use http_body_util::BodyExt;
use pryx_agent::logbook::Frame;
use tokio::io::{AsyncBufReadExt, AsyncReadExt};

#[derive(Debug)]
enum Failure {
    Disconnected,
    Invalid(RuntimeError),
}
impl From<RuntimeError> for Failure {
    fn from(problem: RuntimeError) -> Self {
        Self::Invalid(problem)
    }
}
async fn relay(
    spec: &ContainerSpec,
    options: LogOptions,
    tx: &tokio::sync::mpsc::Sender<Result<LogChunk, RuntimeError>>,
) -> Result<(), Failure> {
    let response = agent::log_response(spec, options.follow).await.map_err(|problem| {
        if problem.is_unavailable() || matches!(problem, RuntimeError::Timeout(_)) {
            Failure::Disconnected
        } else {
            Failure::Invalid(problem)
        }
    })?;
    if !response.status().is_success() {
        return Err(error(format!("VM agent refused log read: {}", response.status())).into());
    }
    let body = response.into_body().into_data_stream().map(|part| part.map_err(std::io::Error::other));
    let mut reader = tokio_util::io::StreamReader::new(body);
    let mut first = true;
    loop {
        let mut line = Vec::new();
        const MAX_FRAME_BYTES: u64 = (pryx_agent::contract::CAPTURE_BYTES * 4) as u64;
        let mut bounded = (&mut reader).take(MAX_FRAME_BYTES + 1);
        let result = tokio::select! {
            () = tx.closed() => return Ok(()),
            result = bounded.read_until(b'\n', &mut line) => result,
        };
        let count = match result {
            Ok(count) => count,
            Err(_) if line.is_empty() => return Err(Failure::Disconnected),
            Err(problem) => return Err(error(problem).into()),
        };
        if count == 0 {
            return if options.follow { Err(Failure::Disconnected) } else { Ok(()) };
        }
        if line.len() as u64 > MAX_FRAME_BYTES || line.last() != Some(&b'\n') {
            return Err(error("VM log frame is oversized or incomplete; output gap").into());
        }
        let frame: Frame = serde_json::from_slice(&line).map_err(error)?;
        let chunks = match frame {
            Frame::Replay { chunks } if first => chunks,
            Frame::Chunk { chunk } => vec![chunk],
            Frame::Gap | Frame::Replay { .. } => return Err(error("VM logs have a gap").into()),
        };
        let mut bytes = Vec::new();
        for chunk in chunks {
            bytes.extend(serde_json::to_vec(&chunk).map_err(error)?);
            bytes.push(b'\n');
        }
        let selection = LogOptions { tail: if first { options.tail } else { None }, ..options.clone() };
        for chunk in records(&bytes, &selection)? {
            if tx.send(Ok(chunk)).await.is_err() {
                return Ok(());
            }
        }
        first = false;
    }
}

impl VmRuntime {
    pub(super) fn entrypoint_logs(
        &self,
        name: &str,
        options: LogOptions,
    ) -> futures_util::stream::BoxStream<'static, Result<LogChunk, RuntimeError>> {
        let (own, name) = (self.clone(), name.to_owned());
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        tokio::spawn(async move {
            let result = match own.entry(&name) {
                Ok(entry) => relay(&entry.spec, options, &tx).await,
                Err(problem) => Err(Failure::Invalid(problem)),
            };
            let problem = match result {
                Ok(()) => return,
                Err(Failure::Disconnected) => match own.observe(&name).await {
                    Ok(Some(info)) if info.status != ContainerStatus::Running => return,
                    Ok(None) => return,
                    _ => error("VM log transport disconnected; output gap"),
                },
                Err(Failure::Invalid(problem)) => problem,
            };
            let _ = tx.send(Err(problem)).await;
        });
        stream::poll_fn(move |cx| rx.poll_recv(cx)).boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use pryx_agent::{paths::DataDir, ports::Ports, process::Processes, server::Agent};

    #[tokio::test]
    async fn actual_agent_replay_follow_authentication_and_idle_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = super::super::tests::spec(root.path(), "unused");
        let boot = spec.agent.as_ref().unwrap();
        let signing = agent::key(&spec).unwrap();
        let agent = Arc::new(Agent {
            vm_resources: false,
            sandbox: boot.sandbox.clone(),
            epoch: boot.epoch,
            public_key: signing.public_key(),
            key: signing,
            file_tasks: Default::default(),
            shells: Default::default(),
            telemetry: Arc::default(),
            logs: Arc::new(pryx_agent::logbook::Logbook::open(root.path()).unwrap()),
            data: DataDir::open(root.path()).unwrap(),
            processes: Processes::new(root.path().into(), boot.secret_env.clone()),
            ports: Ports::bind_ports(boot.sandbox.clone(), "invalid".into(), boot.agent_port, Vec::new())
                .await
                .unwrap(),
        });
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        spec.ports[0].host_port = listener.local_addr().unwrap().port();
        let router = pryx_agent::server::router(agent.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        agent.logs.record("stdout", &STANDARD.encode(b"one\n")).unwrap();
        agent.logs.record("stderr", &STANDARD.encode(b"two\n")).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        relay(&spec, LogOptions { tail: Some(1), ..Default::default() }, &tx).await.unwrap();
        let chunk = rx.recv().await.unwrap().unwrap();
        assert_eq!(chunk.stream, crate::runtime::LogStream::Stderr);
        assert!(chunk.bytes.ends_with(b" two\n"));
        assert!(rx.try_recv().is_err());
        let following_spec = spec.clone();
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let following = tokio::spawn(async move {
            relay(&following_spec, LogOptions { follow: true, tail: Some(1), ..Default::default() }, &tx).await
        });
        assert!(rx.recv().await.unwrap().unwrap().bytes.ends_with(b" two\n"));
        // A live stream outlasts connection setup; an idle subscriber can still cancel it.
        tokio::time::sleep(crate::runtime::ENGINE_PROBE_TIMEOUT + Duration::from_millis(50)).await;
        agent.logs.record("stdout", &STANDARD.encode(b"three\n")).unwrap();
        assert!(rx.recv().await.unwrap().unwrap().bytes.ends_with(b" three\n"));
        drop(rx);
        tokio::time::timeout(Duration::from_secs(1), following).await.unwrap().unwrap().unwrap();
        spec.agent.as_mut().unwrap().epoch += 1;
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        assert!(matches!(relay(&spec, LogOptions::default(), &tx).await, Err(Failure::Invalid(_))));
        server.abort();
    }

    #[tokio::test]
    async fn explicit_gaps_and_oversized_or_incomplete_frames_are_errors() {
        for (bytes, follow) in [
            (format!("{}\n", serde_json::to_string(&Frame::Gap).unwrap()).into_bytes(), false),
            (vec![b'x'; pryx_agent::contract::CAPTURE_BYTES * 4 + 1], false),
            (serde_json::to_vec(&Frame::Replay { chunks: Vec::new() }).unwrap(), false),
            (format!("{}\n", serde_json::to_string(&Frame::Replay { chunks: Vec::new() }).unwrap()).into_bytes(), true),
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut spec = super::super::tests::spec(root.path(), "unused");
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            spec.ports[0].host_port = listener.local_addr().unwrap().port();
            let router = axum::Router::new().route(
                "/_pryx/logs",
                axum::routing::get(move || {
                    let bytes = bytes.clone();
                    async move { bytes }
                }),
            );
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let (tx, _rx) = tokio::sync::mpsc::channel(32);
            let result = relay(&spec, LogOptions { follow, ..Default::default() }, &tx).await;
            if follow {
                assert!(matches!(result, Err(Failure::Disconnected)));
            } else {
                assert!(matches!(result, Err(Failure::Invalid(_))));
            }
            server.abort();
        }
    }
}
