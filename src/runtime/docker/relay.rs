// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private ingress for Docker internal networks, which have no external-connectivity gateway
//! and do not publish ports. Relays bind the same host-policy addresses, reach only the selected
//! container and preserve its gateway-free egress policy. Inspection reconstructs them after restart,
//! from bindings pryxd wrote alone: an image's label of the same name opens nothing.
use crate::{
    protocol::Proto,
    runtime::docker::DockerRuntime,
    runtime::{ContainerRuntime, PortBinding, RuntimeError},
};
use bollard::models::ContainerInspectResponse;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{Mutex, Semaphore, mpsc},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::labels::{LABEL_INTERNAL_BINDINGS, LABEL_RECORD, LabelRecord};
const MAX_CONNECTIONS: usize = 256;
const DATAGRAM_BYTES: usize = pryx_agent::exec_ingress::DATAGRAM_BYTES;
const UDP_IDLE: Duration = Duration::from_secs(60);
pub(super) struct Relays {
    held: Mutex<HashMap<String, Lease>>,
    capacity: Arc<Semaphore>,
}
impl Default for Relays {
    fn default() -> Self {
        Self { held: Mutex::default(), capacity: Arc::new(Semaphore::new(MAX_CONNECTIONS)) }
    }
}
fn error(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Other(format!("Internal ingress: {error}"))
}
impl Relays {
    pub(super) async fn stop(&self, name: &str) {
        let mut held = self.held.lock().await;
        let ids: Vec<_> = held
            .iter()
            .filter(|(id, lease)| id.as_str() == name || lease.name == name)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(lease) = held.remove(&id) {
                lease.retire().await;
            }
        }
    }
    /// Retire any lease whose host declaration disappeared or cannot be authenticated.
    async fn verified_bindings(
        &self,
        info: &ContainerInspectResponse,
        id: &str,
        bindings: Result<Vec<PortBinding>, serde_json::Error>,
    ) -> Result<Vec<PortBinding>, RuntimeError> {
        let bindings = declared(info, bindings);
        match &bindings {
            Ok(bindings) if !bindings.is_empty() => {}
            _ => self.stop(id).await,
        }
        bindings
    }
    pub(super) async fn ensure(
        &self,
        runtime: &DockerRuntime,
        info: &ContainerInspectResponse,
    ) -> Result<(), RuntimeError> {
        let Some(encoded) =
            info.config.as_ref().and_then(|c| c.labels.as_ref()).and_then(|l| l.get(LABEL_INTERNAL_BINDINGS))
        else {
            return Ok(());
        };
        let id = info.id.as_deref().ok_or_else(|| error("missing container identity"))?;
        let name = info.name.as_deref().unwrap_or(id).trim_start_matches('/');
        if info.state.as_ref().and_then(|s| s.running) != Some(true) {
            self.stop(id).await;
            return Ok(());
        }
        let bindings = serde_json::from_str::<Vec<PortBinding>>(encoded);
        if bindings.as_ref().is_ok_and(Vec::is_empty) {
            self.stop(id).await;
            return Ok(());
        }
        let metadata = runtime.info.lock().unwrap().clone();
        let metadata = match metadata {
            Some(metadata) => metadata,
            None => runtime.info().await?,
        };
        // Native Podman always owns its published ingress, including containers made before
        // empty binding labels were written. An inherited image label is never a relay lease.
        if metadata.podman {
            self.stop(id).await;
            return Ok(());
        }
        let bindings = self.verified_bindings(info, id, bindings).await?;
        if bindings.is_empty() {
            return Ok(());
        }
        let start = info.state.as_ref().and_then(|state| state.started_at.clone());
        let mut held = self.held.lock().await;
        if held.get(id).is_some_and(|lease| lease.start == start) {
            return Ok(());
        }
        let previous: Vec<_> = held
            .iter()
            .filter(|(previous, lease)| previous.as_str() == id || lease.name == name)
            .map(|(id, _)| id.clone())
            .collect();
        for id in previous {
            if let Some(lease) = held.remove(&id) {
                lease.retire().await;
            }
        }
        let execute = metadata.rootless && !metadata.podman;
        if execute {
            super::relay_exec::retire_orphans(runtime, info).await?;
        }
        let ip = (!execute)
            .then(|| {
                info.network_settings
                    .as_ref()
                    .and_then(|n| n.networks.as_ref())
                    .and_then(|networks| {
                        networks.values().find_map(|n| {
                            n.ip_address
                                .as_deref()
                                .and_then(|v| v.parse::<IpAddr>().ok())
                                .or_else(|| n.global_ipv6_address.as_deref().and_then(|v| v.parse().ok()))
                        })
                    })
                    .ok_or_else(|| error("missing private container address"))
            })
            .transpose()?;
        let cancel = CancellationToken::new();
        let tasks = TaskTracker::new();
        let lease = Lease { name: name.into(), start, cancel: cancel.clone(), tasks: tasks.clone() };
        for binding in bindings {
            let target = if execute {
                Backend::Exec(super::relay_exec::Target::new(runtime, info, &binding).await?)
            } else {
                Backend::Direct(SocketAddr::new(
                    ip.expect("direct ingress has a private address"),
                    binding.container_port,
                ))
            };
            for host in binding.host_ips {
                let address = SocketAddr::new(host, binding.host_port);
                match binding.protocol {
                    Proto::Tcp => tcp(
                        TcpListener::bind(address).await.map_err(error)?,
                        target.clone(),
                        cancel.clone(),
                        self.capacity.clone(),
                        tasks.clone(),
                    ),
                    Proto::Udp => udp(
                        Arc::new(UdpSocket::bind(address).await.map_err(error)?),
                        target.clone(),
                        cancel.clone(),
                        self.capacity.clone(),
                        tasks.clone(),
                    ),
                }
            }
        }
        held.insert(id.into(), lease);
        Ok(())
    }
}
/// Image defaults are not declarations. Only the immutable host record can authorize a
/// private lease; historical internet labels are ignored even when their JSON is malformed.
fn declared(
    info: &ContainerInspectResponse,
    bindings: Result<Vec<PortBinding>, serde_json::Error>,
) -> Result<Vec<PortBinding>, RuntimeError> {
    let Some(record) = info
        .config
        .as_ref()
        .and_then(|config| config.labels.as_ref())
        .and_then(|labels| labels.get(LABEL_RECORD))
        .and_then(|encoded| serde_json::from_str::<LabelRecord>(encoded).ok())
    else {
        // pryxd writes its record on every container it makes. Bindings without one are an
        // image's, or a container's pryxd never made: they open nothing and fail nothing.
        return Ok(Vec::new());
    };
    if record.spec.network != crate::protocol::NetworkMode::None {
        return Ok(Vec::new());
    }
    let bindings = bindings.map_err(|_| error("network-policy private binding record is malformed"))?;
    if !bindings.iter().all(|binding| {
        record.ports.iter().any(|port| {
            port.container_port == binding.container_port
                && port.host_port == binding.host_port
                && port.protocol == binding.protocol
        })
    }) {
        return Err(error("network-policy private bindings do not match the host-written port record"));
    }
    Ok(bindings)
}

#[derive(Clone)]
enum Backend {
    Direct(SocketAddr),
    Exec(Arc<super::relay_exec::Target>),
}
struct Lease {
    name: String,
    start: Option<String>,
    cancel: CancellationToken,
    tasks: TaskTracker,
}
impl Lease {
    async fn retire(self) {
        self.cancel.cancel();
        self.tasks.close();
        self.tasks.wait().await;
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn tcp(
    listener: TcpListener,
    target: Backend,
    cancel: CancellationToken,
    capacity: Arc<Semaphore>,
    tasks: TaskTracker,
) {
    tasks.clone().spawn(async move {
        loop {
            let accepted = tokio::select! { () = cancel.cancelled() => break, result = listener.accept() => result };
            let Ok((mut incoming, _)) = accepted else { break };
            let Ok(permit) = capacity.clone().try_acquire_owned() else { continue };
            let cancel = cancel.clone();
            let target = target.clone();
            tasks.spawn(async move {
                let _ = incoming.set_nodelay(true);
                let forward = async {
                    match target {
                        Backend::Direct(address) => {
                            let _permit = permit;
                            let mut outgoing = TcpStream::connect(address).await?;
                            outgoing.set_nodelay(true)?;
                            tokio::io::copy_bidirectional_with_sizes(&mut incoming, &mut outgoing, 8192, 8192).await
                        }
                        Backend::Exec(target) => {
                            let mut outgoing = target.connect(Some(permit)).await?;
                            tokio::io::copy_bidirectional_with_sizes(&mut incoming, &mut outgoing.stream, 8192, 8192)
                                .await
                        }
                    }
                };
                tokio::select! { () = cancel.cancelled() => {}, _ = forward => {} }
            });
        }
    });
}
fn udp(
    front: Arc<UdpSocket>,
    target: Backend,
    cancel: CancellationToken,
    capacity: Arc<Semaphore>,
    tasks: TaskTracker,
) {
    tasks.clone().spawn(async move {
        let mut peers: HashMap<SocketAddr, (u64, mpsc::Sender<Vec<u8>>)> = HashMap::new();
        let (done_tx, mut done_rx) = mpsc::channel(MAX_CONNECTIONS);
        let mut generation = 0u64;
        let mut bytes = vec![0; DATAGRAM_BYTES];
        loop {
            let received = tokio::select! {
                () = cancel.cancelled() => break,
                Some((peer, generation)) = done_rx.recv() => {
                    if peers.get(&peer).is_some_and(|(current, _)| *current == generation) { peers.remove(&peer); }
                    continue;
                }
                result = front.recv_from(&mut bytes) => result,
            };
            let Ok((length, peer)) = received else { break };
            if peers.get(&peer).is_some_and(|(_, sender)| sender.is_closed()) {
                peers.remove(&peer);
            }
            if let std::collections::hash_map::Entry::Vacant(entry) = peers.entry(peer) {
                let Ok(permit) = capacity.clone().try_acquire_owned() else { continue };
                let (tx, rx) = mpsc::channel(2);
                generation = generation.wrapping_add(1);
                entry.insert((generation, tx));
                let done = done_tx.clone();
                let front = front.clone();
                let cancel = cancel.clone();
                let target = target.clone();
                tasks.spawn(async move {
                    let work = async {
                        match target {
                            Backend::Direct(target) => {
                                let _permit = permit;
                                udp_peer(front, peer, target, rx).await
                            }
                            Backend::Exec(target) => {
                                super::relay_exec::datagrams(&target, front, peer, rx, UDP_IDLE, permit).await
                            }
                        }
                    };
                    tokio::select! { () = cancel.cancelled() => {}, _ = work => {} }
                    let _ = done.send((peer, generation)).await;
                });
            }
            if let Some((_, sender)) = peers.get(&peer) {
                let _ = sender.try_send(bytes[..length].to_vec());
            }
        }
    });
}
async fn udp_peer(
    front: Arc<UdpSocket>,
    peer: SocketAddr,
    target: SocketAddr,
    mut input: mpsc::Receiver<Vec<u8>>,
) -> std::io::Result<()> {
    let bind = if target.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let socket = UdpSocket::bind(bind).await?;
    socket.connect(target).await?;
    let mut response = vec![0; DATAGRAM_BYTES];
    loop {
        tokio::select! {
            () = tokio::time::sleep(UDP_IDLE) => return Ok(()),
            packet = input.recv() => {
                let Some(packet) = packet else { return Ok(()) };
                socket.send(&packet).await?;
            }
            result = socket.recv(&mut response) => { let size = result?; front.send_to(&response[..size], peer).await?; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn tcp_relay_preserves_bytes_and_cancellation_closes_connections() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        tcp(
            listener,
            Backend::Direct(backend.local_addr().unwrap()),
            cancel.clone(),
            Arc::new(Semaphore::new(1)),
            TaskTracker::new(),
        );
        let mut client = TcpStream::connect(address).await.unwrap();
        let (mut server, _) = backend.accept().await.unwrap();
        client.write_all(b"private").await.unwrap();
        let mut bytes = [0; 7];
        server.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"private");
        server.write_all(b"reply").await.unwrap();
        let mut reply = [0; 5];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"reply");
        cancel.cancel();
        assert_eq!(tokio::time::timeout(Duration::from_secs(1), client.read(&mut reply)).await.unwrap().unwrap(), 0);
    }
    /// A running container carrying the supplied bindings and optional node record.
    fn container(label: &str, record: Option<&str>) -> ContainerInspectResponse {
        use bollard::models::{ContainerConfig, ContainerState, EndpointSettings, NetworkSettings};
        let endpoint =
            EndpointSettings { ip_address: Some(std::net::Ipv4Addr::LOCALHOST.to_string()), ..Default::default() };
        let mut labels = HashMap::from([(LABEL_INTERNAL_BINDINGS.to_owned(), label.to_owned())]);
        if let Some(record) = record {
            labels.insert(LABEL_RECORD.to_owned(), record.to_owned());
        }
        ContainerInspectResponse {
            id: Some("owned".into()),
            name: Some("/owned".into()),
            state: Some(ContainerState { running: Some(true), ..Default::default() }),
            config: Some(ContainerConfig { labels: Some(labels), ..Default::default() }),
            network_settings: Some(NetworkSettings {
                networks: Some(HashMap::from([("attached".to_owned(), endpoint)])),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    async fn listens(port: u16) -> bool {
        TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await.is_ok()
    }

    #[tokio::test]
    async fn inspect_retires_empty_relay_without_engine_metadata() {
        use axum::{Json, Router, http::StatusCode, routing::get};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let metadata_calls = Arc::new(AtomicUsize::new(0));
        let calls = metadata_calls.clone();
        let app = Router::new()
            .route("/containers/owned/json", get(|| async { Json(container("[]", None)) }))
            .fallback(move || {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::Relaxed);
                    StatusCode::SERVICE_UNAVAILABLE
                }
            });
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let runtime = DockerRuntime::new(&path).unwrap();
        let cancel = CancellationToken::new();
        runtime.relays.held.lock().await.insert(
            "owned".into(),
            Lease { name: "owned".into(), start: None, cancel: cancel.clone(), tasks: TaskTracker::new() },
        );

        let inspected = runtime.inspect("owned").await;
        assert_eq!(metadata_calls.load(Ordering::Relaxed), 0, "empty bindings queried metadata: {inspected:?}");
        assert_eq!(inspected.unwrap().unwrap().id, "owned");
        assert!(cancel.is_cancelled());
        assert!(runtime.relays.held.lock().await.is_empty());
        server.abort();
    }

    #[tokio::test]
    async fn only_matching_private_records_open_host_listeners() {
        let relays = Relays::default();
        let directory = tempfile::tempdir().unwrap();
        let runtime = DockerRuntime::new(&directory.path().join("unused.sock")).unwrap();
        *runtime.info.lock().unwrap() = Some(super::super::RuntimeInfo::default());
        let port =
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        let binding = PortBinding {
            container_port: 8080,
            protocol: Proto::Tcp,
            host_ips: vec![std::net::Ipv4Addr::LOCALHOST.into()],
            host_port: port,
        };
        let label = serde_json::to_string(std::slice::from_ref(&binding)).unwrap();
        let record = crate::labels::private_record("fixture", std::slice::from_ref(&binding));
        relays.ensure(&runtime, &container(&label, Some(&record))).await.unwrap();
        assert!(listens(port).await);
        let mut mismatch = binding;
        mismatch.container_port += 1;
        let mismatched = serde_json::to_string(&[mismatch]).unwrap();
        assert!(relays.ensure(&runtime, &container(&mismatched, Some(&record))).await.is_err());
        assert!(!listens(port).await, "a mismatched record must retire its listener");
        assert!(relays.ensure(&runtime, &container("malformed", Some(&record))).await.is_err());

        let mut historical: LabelRecord = serde_json::from_str(&record).unwrap();
        historical.spec.network = crate::protocol::NetworkMode::Internet;
        historical.spec_digest = historical.spec.digest();
        let historical = serde_json::to_string(&historical).unwrap();
        for untrusted in [None, Some(historical.as_str())] {
            relays.ensure(&runtime, &container(&label, untrusted)).await.unwrap();
            assert!(!listens(port).await);
            relays.ensure(&runtime, &container("malformed", untrusted)).await.unwrap();
        }

        *runtime.info.lock().unwrap() = Some(super::super::RuntimeInfo { podman: true, ..Default::default() });
        relays.ensure(&runtime, &container(&label, Some(&record))).await.unwrap();
        assert!(!listens(port).await, "Podman owns its published ingress");
    }

    #[tokio::test]
    async fn udp_relay_keeps_each_peers_reply_address() {
        let backend = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let front = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let address = front.local_addr().unwrap();
        let cancel = CancellationToken::new();
        udp(
            front,
            Backend::Direct(backend.local_addr().unwrap()),
            cancel.clone(),
            Arc::new(Semaphore::new(2)),
            TaskTracker::new(),
        );
        for text in [b"one", b"two"] {
            let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            client.send_to(text, address).await.unwrap();
            let mut bytes = [0; 8];
            let (size, from) =
                tokio::time::timeout(Duration::from_secs(1), backend.recv_from(&mut bytes)).await.unwrap().unwrap();
            backend.send_to(&bytes[..size], from).await.unwrap();
            let (size, _) =
                tokio::time::timeout(Duration::from_secs(1), client.recv_from(&mut bytes)).await.unwrap().unwrap();
            assert_eq!(&bytes[..size], text);
        }
        cancel.cancel();
    }
}
