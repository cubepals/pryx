// SPDX-License-Identifier: FSL-1.1-ALv2
//! A protected Docker-exec pipe to one host-declared container loopback service. HTTP/2 owns
//! cancellation and independent shutdown; the target never comes from request headers. The
//! helper starts no children and receives no bootstrap credentials or host network capability.
//!
//! Parts (`exec_ingress/`):
//! - `tests.rs`: real private TCP/UDP transport and cancellation regressions.
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::Empty;
use hyper::{Method, Request, Response, StatusCode, body::Incoming, server::conn::http2, service::service_fn};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::{convert::Infallible, io, net::Ipv4Addr};
use tokio::net::UdpSocket;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

pub const MODE: &str = "internal-ingress";
pub const VERSION: u16 = 1;
/// A UDP length is a sixteen-bit field; the codec rejects a larger record before allocation.
pub const DATAGRAM_BYTES: usize = u16::MAX as usize;

#[derive(Clone, Copy)]
pub enum Protocol {
    Tcp,
    Udp,
}
impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

pub fn packet_codec() -> LengthDelimitedCodec {
    LengthDelimitedCodec::builder().length_field_length(2).max_frame_length(DATAGRAM_BYTES).new_codec()
}

pub fn run(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if arguments.len() != 2 {
        return Err("Internal ingress requires a protocol and declared port".into());
    }
    let protocol = match arguments[0].as_str() {
        "tcp" => Protocol::Tcp,
        "udp" => Protocol::Udp,
        _ => return Err("Internal ingress protocol is unsupported".into()),
    };
    let port: u16 = arguments[1].parse()?;
    if port == 0 {
        return Err("Internal ingress requires a nonzero port".into());
    }
    // The kernel must protect private ingress bytes before this process exists, as it does
    // for the exec completion owner. A late prctl cannot revoke an already opened pipe.
    if crate::fuse_namespace::readable(&std::env::current_exe()?) {
        return Err("Internal ingress requires a protected execute-only static agent".into());
    }
    crate::exec_deadline::protect_owner(rustix::process::dumpable_behavior()?)?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(serve(tokio::io::join(tokio::io::stdin(), tokio::io::stdout()), port, protocol))?;
    Ok(())
}

pub async fn serve(
    socket: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    port: u16,
    protocol: Protocol,
) -> io::Result<()> {
    match protocol {
        Protocol::Tcp => crate::vm_ingress::stream::serve(socket, port).await,
        Protocol::Udp => http2::Builder::new(TokioExecutor::new())
            .max_concurrent_streams(1)
            .serve_connection(TokioIo::new(socket), service_fn(move |request| packet(request, port)))
            .await
            .map_err(io::Error::other),
    }
}

async fn packet(mut request: Request<Incoming>, port: u16) -> Result<Response<Empty<Bytes>>, Infallible> {
    let mut response = Response::new(Empty::new());
    if request.method() != Method::CONNECT {
        *response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
        return Ok(response);
    }
    let socket = match UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await {
        Ok(socket) => socket,
        Err(_) => {
            *response.status_mut() = StatusCode::BAD_GATEWAY;
            return Ok(response);
        }
    };
    if socket.connect((Ipv4Addr::LOCALHOST, port)).await.is_err() {
        *response.status_mut() = StatusCode::BAD_GATEWAY;
        return Ok(response);
    }
    tokio::spawn(async move {
        let Ok(upgraded) = hyper::upgrade::on(&mut request).await else { return };
        let (mut sink, mut source) = Framed::new(TokioIo::new(upgraded), packet_codec()).split();
        let mut bytes = vec![0; DATAGRAM_BYTES];
        loop {
            tokio::select! {
                packet = source.next() => {
                    let Some(Ok(packet)) = packet else { return };
                    if socket.send(&packet).await.is_err() { return }
                }
                packet = socket.recv(&mut bytes) => {
                    let Ok(length) = packet else { return };
                    if sink.send(Bytes::copy_from_slice(&bytes[..length])).await.is_err() { return }
                }
            }
        }
    });
    Ok(response)
}

#[cfg(test)]
#[path = "exec_ingress/tests.rs"]
mod tests;
