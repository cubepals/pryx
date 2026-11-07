// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private edge ingress reaches a virtual guest listener through agentOS's streaming HTTP API.
//! The existing agent port assignment preface is verified before this adapter sees a request.

use agentos_client::{AgentOs, fs::FileContent, net::HttpRequest};
use async_trait::async_trait;
use axum::body::Body;
use bytes::{Buf, Bytes};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;

// Host admission bounds simultaneous per-VM HTTP buffers, independent of the guest's limits.
const MAX_CONNECTIONS: usize = 16;
const READ_CHUNK: u32 = 64 * 1024;

pub(super) struct Network {
    vm: AgentOs,
    connections: Arc<Semaphore>,
    requests: Arc<Semaphore>,
}

impl Network {
    pub(super) fn new(vm: AgentOs, requests: Arc<Semaphore>) -> Self {
        Self { vm, requests, connections: Arc::new(Semaphore::new(MAX_CONNECTIONS)) }
    }
}

#[async_trait]
impl pryx_agent::guest::Forward for Network {
    async fn probe(&self, port: u16, path: Option<&str>) -> std::io::Result<Option<u16>> {
        if let Some(path) = path {
            let head = self
                .vm
                .http_request_stream_start(HttpRequest {
                    port,
                    path: path.into(),
                    method: "GET".into(),
                    headers: Default::default(),
                    body: None,
                })
                .await
                .map_err(std::io::Error::other)?;
            self.vm.http_request_stream_cancel(&head.stream_id).await.map_err(std::io::Error::other)?;
            return Ok(Some(head.status));
        }
        let script = format!(
            r#"node -e 'const s=require("net").connect({port},"127.0.0.1");s.on("connect",()=>{{s.destroy();process.exit(0)}});s.on("error",()=>process.exit(1));setTimeout(()=>process.exit(1),500)'"#
        );
        let result = self
            .vm
            .exec_process(&script, agentos_client::ExecOptions { capture_stdio: Some(true), ..Default::default() })
            .await
            .map_err(std::io::Error::other)?;
        if result.exit_code != 0 {
            return Err(std::io::ErrorKind::ConnectionRefused.into());
        }
        Ok(None)
    }
    async fn serve(&self, socket: TcpStream, port: u16, header: Vec<u8>) -> std::io::Result<()> {
        let _permit = self
            .connections
            .clone()
            .try_acquire_owned()
            .map_err(|_| std::io::Error::other("guest ingress capacity reached"))?;
        let vm = self.vm.clone();
        let requests = self.requests.clone();
        let service = hyper::service::service_fn(move |request| {
            let vm = vm.clone();
            let requests = requests.clone();
            async move {
                let permit = match requests.try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        let mut response = Response::new(Body::from("Guest ingress capacity reached"));
                        *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
                        return Ok::<_, std::convert::Infallible>(response);
                    }
                };
                let response = forward(vm, port, request, permit).await.unwrap_or_else(|_| {
                    let mut response = Response::new(Body::from("Guest application unavailable"));
                    *response.status_mut() = StatusCode::BAD_GATEWAY;
                    response
                });
                Ok::<_, std::convert::Infallible>(response)
            }
        });
        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(Prefaced { prefix: header.into(), socket }), service)
            .await
            .map_err(std::io::Error::other)
    }
}

async fn forward(
    vm: AgentOs,
    port: u16,
    request: Request<hyper::body::Incoming>,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> anyhow::Result<Response<Body>> {
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(Body::new(body), agentos_sidecar_client::wire::DEFAULT_MAX_FRAME_BYTES).await?;
    let head = start(
        &vm,
        HttpRequest {
            port,
            path: parts.uri.path_and_query().map_or("/", |p| p.as_str()).into(),
            method: parts.method.as_str().into(),
            headers: parts
                .headers
                .iter()
                .filter_map(|(name, value)| value.to_str().ok().map(|value| (name.to_string(), value.to_owned())))
                .collect(),
            body: (!body.is_empty()).then(|| FileContent::Bytes(body.to_vec())),
        },
    )
    .await?;
    let state = Ingress { vm, id: head.stream_id, done: false, _permit: permit };
    let stream = futures_util::stream::try_unfold(state, |mut state| async move {
        if state.done {
            return Ok::<_, std::io::Error>(None);
        }
        let chunk = state.vm.http_request_stream_read(&state.id, READ_CHUNK).await.map_err(std::io::Error::other)?;
        state.done = chunk.done;
        Ok(Some((Bytes::from(chunk.body), state)))
    });
    let mut response = Response::builder().status(StatusCode::from_u16(head.status)?);
    for (name, value) in head.headers {
        if !matches!(name.to_ascii_lowercase().as_str(), "connection" | "transfer-encoding" | "upgrade") {
            response = response.header(name, value);
        }
    }
    Ok(response.body(Body::from_stream(stream))?)
}

pub(super) async fn start(vm: &AgentOs, request: HttpRequest) -> anyhow::Result<agentos_client::net::HttpStreamHead> {
    // RFC 9110 section 9.2.1 defines GET/HEAD as safe. A newly spawned Node listener can
    // still be starting; retry only these safe requests after a kernel rejection before
    // a response head. Never repeat a mutation, an HTTP response or a stream read.
    let safe = matches!(request.method.as_str(), "GET" | "HEAD");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match vm.http_request_stream_start(request.clone()).await {
            Err(error)
                if safe
                    && tokio::time::Instant::now() < deadline
                    && matches!(error.downcast_ref(), Some(agentos_client::ClientError::Kernel { .. })) =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            result => return result,
        }
    }
}

struct Ingress {
    vm: AgentOs,
    id: String,
    done: bool,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl Drop for Ingress {
    fn drop(&mut self) {
        if !self.done {
            let (vm, id) = (self.vm.clone(), self.id.clone());
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    if vm.http_request_stream_cancel(&id).await.is_err() {
                        tracing::warn!("guest HTTP stream cancellation failed");
                    }
                });
            }
        }
    }
}

struct Prefaced {
    prefix: Bytes,
    socket: TcpStream,
}

impl AsyncRead for Prefaced {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buffer: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if !this.prefix.is_empty() {
            let count = this.prefix.len().min(buffer.remaining());
            buffer.put_slice(&this.prefix[..count]);
            this.prefix.advance(count);
            Poll::Ready(Ok(()))
        } else {
            Pin::new(&mut this.socket).poll_read(cx, buffer)
        }
    }
}

impl AsyncWrite for Prefaced {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, bytes: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().socket).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().socket).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().socket).poll_shutdown(cx)
    }
}
