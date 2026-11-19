// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real loopback services and HTTP/2 pipes prove opaque shutdown, UDP peer identity and bounds.
//! These tests exercise transport, not Docker isolation or production bootstrap authenticity.
use super::*;
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::task::AbortOnDropHandle;

async fn pipe(
    port: u16,
    protocol: Protocol,
    authority: &str,
) -> (TokioIo<hyper::upgrade::Upgraded>, AbortOnDropHandle<()>, tokio::task::JoinHandle<io::Result<()>>) {
    let (host, guest) = tokio::net::UnixStream::pair().unwrap();
    let server = tokio::spawn(serve(guest, port, protocol));
    let (mut sender, connection) =
        hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(host)).await.unwrap();
    let driver = AbortOnDropHandle::new(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let request = Request::builder().method(Method::CONNECT).uri(authority).body(Empty::<Bytes>::new()).unwrap();
    let response = sender.send_request(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    (TokioIo::new(hyper::upgrade::on(response).await.unwrap()), driver, server)
}

#[tokio::test]
async fn target_is_fixed_and_reply_survives_client_write_eof() {
    let target = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = target.local_addr().unwrap().port();
    let other = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let authority = format!("other:{}", other.local_addr().unwrap().port());
    let answer = tokio::spawn(async move {
        let (mut socket, _) = target.accept().await.unwrap();
        let mut request = Vec::new();
        socket.read_to_end(&mut request).await.unwrap();
        socket.write_all(b"after-eof:").await.unwrap();
        socket.write_all(&request).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    let (mut stream, driver, server) = pipe(port, Protocol::Tcp, &authority).await;
    let payload = (0..=u8::MAX).cycle().take(96 * 1024).collect::<Vec<_>>();
    stream.write_all(&payload).await.unwrap();
    stream.shutdown().await.unwrap();
    let mut reply = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut reply)).await.unwrap().unwrap();
    let mut expected = b"after-eof:".to_vec();
    expected.extend(payload);
    assert!(reply == expected, "opaque reply after a half-close changed");
    assert!(
        tokio::time::timeout(Duration::from_millis(30), other.accept()).await.is_err(),
        "request authority must not choose a different service"
    );
    answer.await.unwrap();
    drop(stream);
    drop(driver);
    tokio::time::timeout(Duration::from_secs(3), server).await.unwrap().unwrap().ok();
}

#[tokio::test]
async fn datagrams_keep_boundaries_and_a_distinct_stable_socket_per_peer() {
    let backend = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = backend.local_addr().unwrap().port();
    let (first, first_driver, first_server) = pipe(port, Protocol::Udp, "fixed:1").await;
    let (second, second_driver, second_server) = pipe(port, Protocol::Udp, "fixed:1").await;
    let mut first = Framed::new(first, packet_codec());
    let mut second = Framed::new(second, packet_codec());
    let mut bytes = vec![0; DATAGRAM_BYTES];
    let mut peers = Vec::new();
    for (index, payload) in
        [Bytes::from_static(b"one"), Bytes::new(), Bytes::from(vec![0xa5; 65_500])].into_iter().enumerate()
    {
        let stream = if index == 1 { &mut second } else { &mut first };
        stream.send(payload.clone()).await.unwrap();
        let (length, peer) =
            tokio::time::timeout(Duration::from_secs(3), backend.recv_from(&mut bytes)).await.unwrap().unwrap();
        assert_eq!(&bytes[..length], payload.as_ref());
        peers.push(peer);
        backend.send_to(&bytes[..length], peer).await.unwrap();
        let answer = tokio::time::timeout(Duration::from_secs(3), stream.next()).await.unwrap().unwrap().unwrap();
        assert_eq!(answer.as_ref(), payload.as_ref());
    }
    assert_ne!(peers[0], peers[1], "two front peers need separate backend sockets");
    assert_eq!(peers[0], peers[2], "a peer's backend socket must survive successive datagrams");
    assert!(
        first.send(Bytes::from(vec![0; DATAGRAM_BYTES + 1])).await.is_err(),
        "oversized records must be refused before UDP submission"
    );
    assert!(tokio::time::timeout(Duration::from_millis(30), backend.recv_from(&mut bytes)).await.is_err());
    drop(first);
    drop(second);
    drop(first_driver);
    drop(second_driver);
    for server in [first_server, second_server] {
        tokio::time::timeout(Duration::from_secs(3), server).await.unwrap().unwrap().ok();
    }
}

#[test]
fn an_ordinary_readable_process_cannot_enable_the_private_helper() {
    assert!(run(&["tcp".into(), "1".into()]).is_err());
}
