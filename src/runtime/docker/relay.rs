// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private ingress for Docker internal networks, which have no external-connectivity gateway
//! and do not publish ports. Relays bind the same host-policy addresses, reach only the selected
//! container and preserve its gateway-free egress policy. Inspection reconstructs them after restart,
//! from bindings pryxd wrote alone: an image's label of the same name opens nothing.
use super::{ContainerRuntime, DockerRuntime};
use crate::{
    protocol::Proto,
    runtime::{PortBinding, RuntimeError},
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
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::labels::LABEL_INTERNAL_BINDINGS;
const MAX_CONNECTIONS: usize = 256;
const DATAGRAM_BYTES: usize = u16::MAX as usize;
const UDP_IDLE: Duration = Duration::from_secs(60);
pub(super) struct Relays {
    held: Mutex<HashMap<String, (String, DropGuard)>>,
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
        self.held.lock().await.retain(|id, (held_name, _)| id != name && held_name != name);
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
        // Read, not yet believed: an empty list asks for nothing, whoever wrote it.
        let bindings = serde_json::from_str::<Vec<PortBinding>>(encoded);
        if bindings.as_ref().is_ok_and(Vec::is_empty) || self.held.lock().await.contains_key(id) {
            return Ok(());
        }
        if !written_by_host(runtime, info).await? {
            return Ok(());
        }
        let bindings = bindings.map_err(error)?;
        let mut held = self.held.lock().await;
        if held.contains_key(id) {
            return Ok(());
        }
        held.retain(|_, (previous, _)| previous != name);
        let ip = info
            .network_settings
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
            .ok_or_else(|| error("missing private container address"))?;
        let cancel = CancellationToken::new();
        let guard = cancel.clone().drop_guard();
        for binding in bindings {
            let target = SocketAddr::new(ip, binding.container_port);
            for host in binding.host_ips {
                let address = SocketAddr::new(host, binding.host_port);
                match binding.protocol {
                    Proto::Tcp => tcp(
                        TcpListener::bind(address).await.map_err(error)?,
                        target,
                        cancel.clone(),
                        self.capacity.clone(),
                    ),
                    Proto::Udp => udp(
                        Arc::new(UdpSocket::bind(address).await.map_err(error)?),
                        target,
                        cancel.clone(),
                        self.capacity.clone(),
                    ),
                }
            }
        }
        held.insert(id.into(), (name.into(), guard));
        Ok(())
    }
}

/// Whether pryxd wrote this container's bindings. It writes them only for a container it put on
/// its own gateway-free internal network, which the image can't choose, while every container
/// inherits its image's labels: one on any other network, or none, carries an image's. Native
/// Podman publishes its own ingress, so no bindings on it are pryxd's.
async fn written_by_host(runtime: &DockerRuntime, info: &ContainerInspectResponse) -> Result<bool, RuntimeError> {
    let cached = runtime.info.lock().unwrap().clone();
    let engine = match cached {
        Some(engine) => engine,
        None => runtime.info().await?,
    };
    if engine.podman {
        return Ok(false);
    }
    let attached: Vec<_> =
        info.network_settings.as_ref().and_then(|n| n.networks.as_ref()).into_iter().flat_map(|n| n.values()).collect();
    let [endpoint] = attached.as_slice() else { return Ok(false) };
    let Some(network) = endpoint.network_id.as_deref().filter(|id| !id.is_empty()) else { return Ok(false) };
    super::network::private(runtime, network).await
}
fn tcp(listener: TcpListener, target: SocketAddr, cancel: CancellationToken, capacity: Arc<Semaphore>) {
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! { () = cancel.cancelled() => break, result = listener.accept() => result };
            let Ok((mut incoming, _)) = accepted else { break };
            let Ok(permit) = capacity.clone().try_acquire_owned() else { continue };
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let _ = incoming.set_nodelay(true);
                let forward = async {
                    let mut outgoing = TcpStream::connect(target).await?;
                    outgoing.set_nodelay(true)?;
                    tokio::io::copy_bidirectional_with_sizes(&mut incoming, &mut outgoing, 8192, 8192).await
                };
                tokio::select! { () = cancel.cancelled() => {}, _ = forward => {} }
            });
        }
    });
}
fn udp(front: Arc<UdpSocket>, target: SocketAddr, cancel: CancellationToken, capacity: Arc<Semaphore>) {
    tokio::spawn(async move {
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
                tokio::spawn(async move {
                    let _permit = permit;
                    let work = udp_peer(front, peer, target, rx);
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
        tcp(listener, backend.local_addr().unwrap(), cancel.clone(), Arc::new(Semaphore::new(1)));
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
    /// An engine whose one network is internal or not, as asked; nothing else is asked of it.
    async fn engine(internal: bool) -> (tempfile::TempDir, DockerRuntime, tokio::task::JoinHandle<()>) {
        use axum::{Json, Router, extract::Request, routing::any};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let app = Router::new().fallback(any(move |request: Request| async move {
            assert!(request.uri().path().ends_with("/networks/attached"), "{}", request.uri());
            Json(bollard::models::NetworkInspect {
                internal: Some(internal),
                options: Some(HashMap::from([(super::super::ICC_OPTION.to_owned(), "false".to_owned())])),
                ..Default::default()
            })
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let runtime = DockerRuntime::new(&path).unwrap();
        *runtime.info.lock().unwrap() = Some(super::super::RuntimeInfo::default());
        (directory, runtime, server)
    }

    /// A running container on the network `attached`, carrying `label` as its bindings.
    fn container(label: &str) -> ContainerInspectResponse {
        use bollard::models::{ContainerConfig, ContainerState, EndpointSettings, NetworkSettings};
        let endpoint = EndpointSettings {
            network_id: Some("attached".into()),
            ip_address: Some(std::net::Ipv4Addr::LOCALHOST.to_string()),
            ..Default::default()
        };
        ContainerInspectResponse {
            id: Some("owned".into()),
            name: Some("/owned".into()),
            state: Some(ContainerState { running: Some(true), ..Default::default() }),
            config: Some(ContainerConfig {
                labels: Some(HashMap::from([(LABEL_INTERNAL_BINDINGS.to_owned(), label.to_owned())])),
                ..Default::default()
            }),
            network_settings: Some(NetworkSettings {
                networks: Some(HashMap::from([("attached".to_owned(), endpoint)])),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// One loopback listener the kernel chose, released at once, as bindings name it.
    fn bindings() -> (u16, String) {
        let port =
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        let binding = PortBinding {
            container_port: 9,
            protocol: Proto::Tcp,
            host_ips: vec![std::net::Ipv4Addr::LOCALHOST.into()],
            host_port: port,
        };
        (port, serde_json::to_string(&[binding]).unwrap())
    }

    async fn listens(port: u16) -> bool {
        TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await.is_ok()
    }

    #[tokio::test]
    async fn only_bindings_pryxd_wrote_open_host_listeners() {
        let relays = Relays::default();
        // pryxd's own, on its gateway-free internal network: they listen, and a corrupt one is said.
        let (_internal, runtime, server) = engine(true).await;
        let (port, label) = bindings();
        relays.ensure(&runtime, &container(&label)).await.unwrap();
        assert!(listens(port).await);
        relays.stop("owned").await;
        assert!(relays.ensure(&runtime, &container("forged")).await.is_err());
        // Native Podman publishes its own ingress: the same label there is never pryxd's.
        *runtime.info.lock().unwrap() = Some(super::super::RuntimeInfo { podman: true, ..Default::default() });
        let (port, label) = bindings();
        relays.ensure(&runtime, &container(&label)).await.unwrap();
        assert!(!listens(port).await, "a Podman container's label opened a listener");
        server.abort();
        // On any other network the label is its image's: nothing listens, and nothing fails.
        let (_ordinary, runtime, server) = engine(false).await;
        let (port, label) = bindings();
        relays.ensure(&runtime, &container(&label)).await.unwrap();
        assert!(!listens(port).await, "an image's label opened a listener");
        relays.ensure(&runtime, &container("forged")).await.unwrap();
        server.abort();
    }
    #[tokio::test]
    async fn udp_relay_keeps_each_peers_reply_address() {
        let backend = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let front = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let address = front.local_addr().unwrap();
        let cancel = CancellationToken::new();
        udp(front, backend.local_addr().unwrap(), cancel.clone(), Arc::new(Semaphore::new(2)));
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
