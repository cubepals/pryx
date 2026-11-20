// SPDX-License-Identifier: FSL-1.1-ALv2
//! Rootless internal ingress through the engine's private exec attachment. The immutable
//! container and declared service are fixed before accepting bytes. HTTP/2 owns half-close
//! and cancellation; a dropped connection closes the helper's stdin and all its sockets.
//!
//! Parts (`relay_exec/`):
//! - `acceptance.rs`: real-engine private transport and owner-adoption acceptance.
use super::*;
use crate::runtime::PortBinding;
use bollard::models::ContainerInspectResponse;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::Empty;
use hyper_util::rt::{TokioExecutor, TokioIo};
use pryx_agent::exec_ingress::{Protocol, packet_codec};
use std::{io, os::unix::fs::MetadataExt, path::PathBuf, sync::Arc};
use tokio::net::UdpSocket;
use tokio::sync::OwnedSemaphorePermit;
use tokio_util::{codec::Framed, io::StreamReader, task::AbortOnDropHandle};

#[derive(Clone, Copy, PartialEq)]
struct Stamp {
    device: u64,
    inode: u64,
    size: u64,
    changed: (i64, i64),
}
impl Stamp {
    fn of(path: &Path) -> Option<Self> {
        let file = std::fs::symlink_metadata(path).ok()?;
        Some(Self {
            device: file.dev(),
            inode: file.ino(),
            size: file.size(),
            changed: (file.ctime(), file.ctime_nsec()),
        })
    }
}

pub(super) struct Support {
    binary: PathBuf,
    cached: std::sync::Mutex<Option<(Stamp, bool)>>,
}
impl Support {
    pub(super) fn new(binary: PathBuf) -> Self {
        Self { binary, cached: std::sync::Mutex::new(None) }
    }
    pub(super) async fn available(&self) -> bool {
        if crate::runtime::protected_agent(&self.binary, false).is_none() {
            return false;
        }
        let Some(stamp) = Stamp::of(&self.binary) else { return false };
        let cached = self.cached.lock().unwrap().filter(|(previous, _)| *previous == stamp).map(|(_, on)| on);
        if let Some(on) = cached {
            return on;
        }
        let available = super::exec_agent::static_binary(&self.binary)
            && crate::runtime::agent_metadata::query(&self.binary)
                .await
                .is_some_and(|caps| caps.internal_ingress_version >= pryx_agent::exec_ingress::VERSION);
        if Stamp::of(&self.binary) != Some(stamp) {
            return false;
        }
        *self.cached.lock().unwrap() = Some((stamp, available));
        available
    }
}

pub(super) struct Target {
    docker: Docker,
    container: String,
    start: OffsetDateTime,
    starts: super::exec_completion::Starts,
    protocol: Protocol,
    port: u16,
}

pub(super) struct Connection {
    pub(super) stream: TokioIo<hyper::upgrade::Upgraded>,
    _driver: AbortOnDropHandle<()>,
    _cleanup: Cleanup,
}

struct Cleanup {
    docker: Docker,
    exec: String,
    container: String,
    start: OffsetDateTime,
    starts: super::exec_completion::Starts,
    started: bool,
    capacity: Option<OwnedSemaphorePermit>,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let docker = self.docker.clone();
        let exec = self.exec.clone();
        let container = self.container.clone();
        let start = self.start;
        let starts = self.starts.clone();
        let started = self.started;
        let capacity = self.capacity.take();
        tokio::spawn(async move {
            // A lost start response is ambiguous: the server may still start it. No retry
            // or early stopped observation can establish cleanup in that case.
            if (!started || !finished(&docker, &exec).await)
                && !super::exec_completion::retire_container(docker, container, start, starts).await
            {
                tracing::warn!("unconfirmed private ingress cleanup could not retire its sandbox");
                // Unknown live owners still consume the node's finite ingress budget. A
                // replacement node must inspect/retire them before constructing new relays.
                if let Some(capacity) = capacity {
                    capacity.forget();
                }
            }
        });
    }
}
async fn finished(docker: &Docker, exec: &str) -> bool {
    tokio::time::timeout(QUICK, async {
        loop {
            match docker.inspect_exec(exec).await {
                Ok(info) if info.running == Some(false) => return true,
                Err(problem) => return matches!(map_err(problem), RuntimeError::NotFound(_)),
                Ok(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .unwrap_or(false)
}

/// A replacement node cannot adopt a private pipe whose previous owner disappeared. Normal
/// EOF retires helpers; an unresponsive or stopped helper fences only its own container start.
pub(super) async fn retire_orphans(
    runtime: &DockerRuntime,
    inspect: &ContainerInspectResponse,
) -> Result<(), RuntimeError> {
    let docker = runtime.docker()?;
    let id = inspect.id.as_ref().ok_or_else(unavailable)?;
    let start = inspect
        .state
        .as_ref()
        .and_then(|state| crate::runtime::parse_time(state.started_at.as_deref()))
        .ok_or_else(unavailable)?;
    for exec in inspect.exec_ids.as_ref().into_iter().flatten() {
        let info = quick("inspect private ingress orphan", docker.inspect_exec(exec)).await?;
        let owned = info.container_id.as_deref() == Some(id)
            && info.process_config.as_ref().is_some_and(|process| {
                process.entrypoint.as_deref() == Some(pryx_agent::contract::BINARY_PATH)
                    && process.arguments.as_ref().and_then(|arguments| arguments.first()).map(String::as_str)
                        == Some(pryx_agent::exec_ingress::MODE)
            });
        if owned && info.running == Some(true) && !finished(&docker, exec).await {
            let _ = super::exec_completion::retire_container(docker.clone(), id.clone(), start, runtime.starts.clone())
                .await;
            return Err(RuntimeError::Unavailable(
                "A private ingress owner was lost; its old container start was retired".into(),
            ));
        }
    }
    Ok(())
}

fn unavailable() -> RuntimeError {
    RuntimeError::Other("network-policy is unavailable: rootless internal ingress needs a current protected static agent; recreate this sandbox with the installed agent".into())
}
impl Target {
    pub(super) async fn new(
        runtime: &DockerRuntime,
        info: &ContainerInspectResponse,
        binding: &PortBinding,
    ) -> Result<Arc<Self>, RuntimeError> {
        let protocol = match binding.protocol {
            crate::protocol::Proto::Tcp => Protocol::Tcp,
            crate::protocol::Proto::Udp => Protocol::Udp,
        };
        let id = info.id.as_ref().ok_or_else(unavailable)?;
        let start = info
            .state
            .as_ref()
            .and_then(|state| crate::runtime::parse_time(state.started_at.as_deref()))
            .ok_or_else(unavailable)?;
        check(info, binding.container_port, binding.protocol).await?;
        Ok(Arc::new(Self {
            docker: runtime.docker()?,
            container: id.clone(),
            start,
            starts: runtime.starts.clone(),
            protocol,
            port: binding.container_port,
        }))
    }
    pub(super) async fn connect(&self, capacity: Option<OwnedSemaphorePermit>) -> io::Result<Connection> {
        tokio::time::timeout(QUICK, self.connected(capacity))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Internal ingress setup timed out"))?
            .map_err(|_| io::Error::other("Internal ingress unavailable for this container generation"))
    }
    async fn connected(&self, capacity: Option<OwnedSemaphorePermit>) -> Result<Connection, RuntimeError> {
        let _start = self.starts.lock(&self.container).await;
        let inspect = quick("inspect private ingress", self.docker.inspect_container(&self.container, None)).await?;
        if inspect.id.as_deref() != Some(&self.container)
            || inspect.state.as_ref().is_none_or(|state| {
                state.running != Some(true)
                    || crate::runtime::parse_time(state.started_at.as_deref()) != Some(self.start)
            })
        {
            return Err(unavailable());
        }
        let protocol = match self.protocol {
            Protocol::Tcp => crate::protocol::Proto::Tcp,
            Protocol::Udp => crate::protocol::Proto::Udp,
        };
        check(&inspect, self.port, protocol).await?;
        // No environment value is needed by this fixed-target static helper. Overrides apply
        // before execve, so even the bootstrap-to-first-syscall interval contains no key.
        let environment = inspect
            .config
            .as_ref()
            .and_then(|config| config.env.as_ref())
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.split_once('=').map(|(name, _)| format!("{name}=")))
            .collect();
        let options = CreateExecOptions::<String> {
            cmd: Some(vec![
                pryx_agent::contract::BINARY_PATH.into(),
                pryx_agent::exec_ingress::MODE.into(),
                self.protocol.as_str().into(),
                self.port.to_string(),
            ]),
            env: Some(environment),
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            working_dir: Some("/".into()),
            tty: Some(false),
            privileged: Some(false),
            ..Default::default()
        };
        let made = quick("create private ingress", self.docker.create_exec(&self.container, options)).await?;
        let mut cleanup = Cleanup {
            docker: self.docker.clone(),
            exec: made.id.clone(),
            container: self.container.clone(),
            start: self.start,
            starts: self.starts.clone(),
            started: false,
            capacity,
        };
        let attached = quick(
            "start private ingress",
            self.docker
                .start_exec(&made.id, Some(StartExecOptions { detach: false, tty: false, output_capacity: None })),
        )
        .await?;
        let StartExecResults::Attached { output, input } = attached else { return Err(unavailable()) };
        cleanup.started = true;
        let output = output.map(|chunk| match chunk {
            Ok(LogOutput::StdOut { message }) => Ok(message),
            _ => Err(io::Error::other("Private ingress pipe ended")),
        });
        let socket = tokio::io::join(StreamReader::new(output), input);
        let (mut sender, connection) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(socket))
                .await
                .map_err(|_| unavailable())?;
        let driver = AbortOnDropHandle::new(tokio::spawn(async move {
            let _ = connection.await;
        }));
        let request = hyper::Request::builder()
            .method(hyper::Method::CONNECT)
            .uri(format!("sandbox:{}", self.port))
            .body(Empty::<Bytes>::new())
            .map_err(|_| unavailable())?;
        let response = sender.send_request(request).await.map_err(|_| unavailable())?;
        if response.status() != StatusCode::OK {
            return Err(unavailable());
        }
        let stream = hyper::upgrade::on(response).await.map_err(|_| unavailable())?;
        Ok(Connection { stream: TokioIo::new(stream), _driver: driver, _cleanup: cleanup })
    }
}

async fn check(
    info: &ContainerInspectResponse,
    port: u16,
    protocol: crate::protocol::Proto,
) -> Result<(), RuntimeError> {
    let labels = info.config.as_ref().and_then(|config| config.labels.as_ref()).ok_or_else(unavailable)?;
    if labels
        .get(crate::labels::LABEL_INTERNAL_INGRESS)
        .and_then(|value| value.parse::<u16>().ok())
        .is_none_or(|version| version < pryx_agent::exec_ingress::VERSION)
    {
        return Err(unavailable());
    }
    let bindings: Vec<PortBinding> =
        serde_json::from_str(labels.get(crate::labels::LABEL_INTERNAL_BINDINGS).ok_or_else(unavailable)?)
            .map_err(|_| unavailable())?;
    if !bindings.iter().any(|binding| binding.container_port == port && binding.protocol == protocol) {
        return Err(unavailable());
    }
    let source = info
        .mounts
        .as_ref()
        .into_iter()
        .flatten()
        .find(|mount| {
            mount.destination.as_deref() == Some(pryx_agent::contract::BINARY_PATH) && mount.rw == Some(false)
        })
        .and_then(|mount| mount.source.as_deref())
        .ok_or_else(unavailable)?;
    if !super::exec_agent::protected(Path::new(source))
        || !crate::runtime::agent_metadata::query(Path::new(source))
            .await
            .is_some_and(|caps| caps.internal_ingress_version >= pryx_agent::exec_ingress::VERSION)
    {
        return Err(unavailable());
    }
    Ok(())
}

pub(super) async fn datagrams(
    target: &Target,
    front: Arc<UdpSocket>,
    peer: std::net::SocketAddr,
    mut input: tokio::sync::mpsc::Receiver<Vec<u8>>,
    idle: Duration,
    capacity: OwnedSemaphorePermit,
) -> io::Result<()> {
    let connection = target.connect(Some(capacity)).await?;
    let (mut sink, mut source) = Framed::new(connection.stream, packet_codec()).split();
    // Keep the HTTP/2 connection owner until the peer closes or its idle bound expires.
    let _driver = connection._driver;
    let _cleanup = connection._cleanup;
    loop {
        tokio::select! {
            () = tokio::time::sleep(idle) => return Ok(()),
            packet = input.recv() => {
                let Some(packet) = packet else { return Ok(()) };
                sink.send(Bytes::from(packet)).await?;
            }
            packet = source.next() => {
                let Some(packet) = packet else { return Ok(()) };
                front.send_to(&packet?, peer).await?;
            }
        }
    }
}

#[cfg(test)]
#[path = "relay_exec/acceptance.rs"]
mod acceptance;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::ready,
        sync::atomic::{AtomicBool, Ordering},
    };
    use tokio::sync::Notify;

    #[tokio::test]
    async fn confirmed_retirement_restores_a_reserved_relay_slot() {
        const ID: &str = "58148883975ef5e77d09e44ccb5a98973361de320d7c0f6d289f82bad03013306";
        let timed_out = Arc::new(Notify::new());
        let gone = Arc::new(AtomicBool::new(false));
        let observed_timeout = timed_out.clone();
        let observed_gone = gone.clone();
        let docker = Docker::connect_with_custom_transport(
            move |request: bollard::BollardRequest| {
                assert_eq!(request.method(), reqwest::Method::GET);
                assert!(request.uri().path().ends_with(&format!("/containers/{ID}/json")));
                if observed_gone.load(Ordering::SeqCst) {
                    ready(Err(super::super::container_not_found_for_test()))
                } else {
                    observed_timeout.notify_one();
                    ready(Err(BollardError::RequestTimeoutError))
                }
            },
            Some("http://fixture.invalid"),
            CLIENT_TIMEOUT_SECS,
            bollard::API_DEFAULT_VERSION,
        )
        .unwrap();
        let runtime = DockerRuntime::new(Path::new("unused-fixture-socket")).unwrap();
        *runtime.client.lock().unwrap() = Some(docker.clone());
        let capacity = runtime.relays.capacity_for_test();
        let _other_connection = capacity.clone().try_acquire_owned().unwrap();
        let available = capacity.available_permits();
        drop(Cleanup {
            docker,
            exec: "fixture-relay".into(),
            container: ID.into(),
            start: OffsetDateTime::UNIX_EPOCH,
            starts: runtime.starts.clone(),
            started: false,
            capacity: Some(capacity.clone().try_acquire_owned().unwrap()),
        });
        // This current-thread fixture returns the timeout in the same poll that signals it.
        tokio::time::timeout(Duration::from_secs(2), timed_out.notified()).await.unwrap();
        assert_eq!(capacity.available_permits(), available - 1, "uncertain retirement must still reserve its slot");
        gone.store(true, Ordering::SeqCst);
        for _ in 0..2 {
            assert!(runtime.inspect(ID).await.unwrap().is_none());
            assert_eq!(capacity.available_permits(), available, "confirmed retirement must restore exactly one slot");
        }
    }
}
