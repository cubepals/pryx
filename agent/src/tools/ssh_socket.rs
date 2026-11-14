// SPDX-License-Identifier: FSL-1.1-ALv2
//! Opaque SSH transport reuses the lease-bound spare listener, including ownership checks and
//! immediate revocation. OpenSSH owns every byte of the SSH protocol; this adapter only frames.
use crate::{
    auth::Claims,
    context::{Agent, Failure},
};
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use std::{net::Ipv4Addr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

pub(super) async fn connect(
    agent: Arc<Agent>,
    claims: Claims,
    id: String,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Failure> {
    let view = super::ssh::view(&agent, &id).await?;
    let expires = view.expires_at.min(claims.expires_at);
    let remaining = expires - crate::telemetry::now();
    if remaining <= 0 {
        return Err(Failure(StatusCode::UNAUTHORIZED, "expired", "SSH access expired".into()));
    }
    let mut upstream = tokio::time::timeout(
        crate::ports::FORWARD_CONNECT_TIMEOUT,
        TcpStream::connect((Ipv4Addr::LOCALHOST, view.slot)),
    )
    .await
    .map_err(|_| super::state::failure("ssh_unavailable", "SSH forward connection timed out"))??;
    let header = format!(
        "CONNECT / HTTP/1.1\r\n{}: {}\r\n{}: {}\r\n\r\n",
        crate::contract::FORWARD_HEADER,
        view.port,
        crate::contract::SSH_FORWARD_HEADER,
        id
    );
    upstream.write_all(header.as_bytes()).await?;
    // The spare validates the exact target/session and owns its revocation watch before forwarding.
    Ok(upgrade
        .protocols([crate::contract::WEBSOCKET_PROTOCOL])
        .max_message_size(crate::contract::CAPTURE_BYTES)
        .on_upgrade(move |socket| relay(socket, upstream, Duration::from_secs(remaining as u64)))
        .into_response())
}
async fn relay(socket: WebSocket, upstream: TcpStream, remaining: Duration) {
    let (mut outgoing, mut incoming) = socket.split();
    let (mut reader, mut writer) = upstream.into_split();
    {
        let to_client = async {
            let mut bytes = vec![0u8; 16 * 1024];
            loop {
                let count = reader.read(&mut bytes).await?;
                if count == 0 {
                    return Ok::<(), std::io::Error>(());
                }
                outgoing.send(Message::Binary(bytes[..count].to_vec().into())).await.map_err(std::io::Error::other)?;
            }
        };
        let from_client = async {
            while let Some(message) = incoming.next().await {
                match message.map_err(std::io::Error::other)? {
                    Message::Binary(bytes) => writer.write_all(&bytes).await?,
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) => {}
                    Message::Text(_) => return Err(std::io::Error::other("SSH requires binary frames")),
                }
            }
            Ok::<(), std::io::Error>(())
        };
        tokio::select! { _ = to_client => {}, _ = from_client => {}, _ = tokio::time::sleep(remaining) => {} }
    }
    let _ = outgoing.close().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{Message as ClientMessage, client::IntoClientRequest},
    };

    #[tokio::test]
    async fn opaque_ssh_frames_keep_first_bytes_and_expire() {
        let peer = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let target = peer.local_addr().unwrap();
        let echo = tokio::spawn(async move {
            let (mut socket, _) = peer.accept().await.unwrap();
            socket.write_all(b"early\0bytes").await.unwrap();
            let (mut read, mut write) = socket.into_split();
            let _ = tokio::io::copy(&mut read, &mut write).await;
        });
        let app = axum::Router::new().route(
            "/socket",
            axum::routing::get(move |upgrade: WebSocketUpgrade| async move {
                let upstream = TcpStream::connect(target).await.unwrap();
                upgrade
                    .protocols([crate::contract::WEBSOCKET_PROTOCOL])
                    .on_upgrade(move |socket| relay(socket, upstream, Duration::from_secs(1)))
            }),
        );
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut request = format!("ws://{}/socket", listener.local_addr().unwrap()).into_client_request().unwrap();
        request
            .headers_mut()
            .insert(axum::http::header::SEC_WEBSOCKET_PROTOCOL, crate::contract::WEBSOCKET_PROTOCOL.parse().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (mut socket, response) = connect_async(request).await.unwrap();
        assert_eq!(response.headers()[axum::http::header::SEC_WEBSOCKET_PROTOCOL], crate::contract::WEBSOCKET_PROTOCOL);
        assert_eq!(socket.next().await.unwrap().unwrap().into_data(), b"early\0bytes".as_slice());
        socket.send(ClientMessage::Binary(b"opaque\0\xff".to_vec().into())).await.unwrap();
        assert_eq!(socket.next().await.unwrap().unwrap().into_data(), b"opaque\0\xff".as_slice());
        tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(frame) = socket.next().await {
                if frame.unwrap().is_close() {
                    return;
                }
            }
        })
        .await
        .expect("SSH stream must close at its expiry");
        server.abort();
        echo.abort();
    }
}
