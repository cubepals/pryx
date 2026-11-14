// SPDX-License-Identifier: FSL-1.1-ALv2
//! HTTP/2 CONNECT delegates stream framing and flow control to hyper/h2. The local TCP
//! target is fixed by the host's declared service; neither request headers nor paths select it.
use bytes::Bytes;
use http_body_util::Empty;
use hyper::{Method, Request, Response, StatusCode, body::Incoming, server::conn::http2, service::service_fn};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::{convert::Infallible, io, net::Ipv4Addr};
use tokio::net::TcpStream;

async fn connect(mut request: Request<Incoming>, port: u16) -> Result<Response<Empty<Bytes>>, Infallible> {
    let mut response = Response::new(Empty::new());
    if request.method() != Method::CONNECT {
        *response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
        return Ok(response);
    }
    let mut target = match TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await {
        Ok(target) => target,
        Err(_) => {
            *response.status_mut() = StatusCode::BAD_GATEWAY;
            return Ok(response);
        }
    };
    tokio::spawn(async move {
        if let Ok(upgraded) = hyper::upgrade::on(&mut request).await {
            let mut stream = TokioIo::new(upgraded);
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut target).await;
        }
    });
    Ok(response)
}

pub(super) async fn serve(
    socket: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    port: u16,
) -> io::Result<()> {
    http2::Builder::new(TokioExecutor::new())
        .serve_connection(TokioIo::new(socket), service_fn(move |request| connect(request, port)))
        .await
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn connected_stream_retains_reply_after_client_write_eof() {
        let target = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = target.local_addr().unwrap().port();
        let echo = tokio::spawn(async move {
            let (mut stream, _) = target.accept().await.unwrap();
            let mut input = Vec::new();
            stream.read_to_end(&mut input).await.unwrap();
            stream.write_all(b"after-eof:").await.unwrap();
            stream.write_all(&input).await.unwrap();
            stream.shutdown().await.unwrap();
        });
        let (host, guest) = tokio::net::UnixStream::pair().unwrap();
        let server = tokio::spawn(serve(guest, port));
        let (mut sender, connection) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(host)).await.unwrap();
        let client = tokio::spawn(connection);
        let request = Request::builder()
            .method(Method::CONNECT)
            .uri(format!("guest:{port}"))
            .body(Empty::<Bytes>::new())
            .unwrap();
        let response = sender.send_request(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = TokioIo::new(hyper::upgrade::on(response).await.unwrap());
        let payload = (0..=u8::MAX).cycle().take(64 * 1024).collect::<Vec<_>>();
        stream.write_all(&payload).await.unwrap();
        stream.shutdown().await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(3), stream.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        let mut expected = b"after-eof:".to_vec();
        expected.extend(payload);
        assert_eq!(response.len(), expected.len());
        assert!(response == expected, "Opaque reply after EOF changed");
        echo.await.unwrap();
        client.abort();
        server.abort();
    }
}
