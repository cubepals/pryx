// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private ingress for Docker internal networks, which have no external-connectivity gateway
//! and do not publish ports. Relays bind the same host-policy addresses, reach only the selected
//! container and preserve its gateway-free egress policy. Inspection reconstructs them after restart.
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
    pub(super) async fn ensure(&self, info: &ContainerInspectResponse) -> Result<(), RuntimeError> {
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
        let mut held = self.held.lock().await;
        if held.contains_key(id) {
            return Ok(());
        }
        held.retain(|_, (previous, _)| previous != name);
        let bindings: Vec<PortBinding> = serde_json::from_str(encoded).map_err(error)?;
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
