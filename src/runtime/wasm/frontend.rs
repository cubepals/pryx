// SPDX-License-Identifier: FSL-1.1-ALv2
//! Own every agent HTTP connection. Disposal closes old keepalive connections and cancels
//! unfinished uploads before a stopped VM's data can be snapshotted or used by its replacement.

use axum::Router;
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use pryx_agent::guest::Forward;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

pub(super) struct Frontend {
    stop: CancellationToken,
    task: Option<JoinHandle<()>>,
}

#[derive(Clone)]
enum Service {
    Agent(Router),
    Application(Arc<super::network::Network>, u16),
}

impl Frontend {
    pub(super) fn start(listener: TcpListener, router: Router) -> Self {
        Self::run(listener, Service::Agent(router))
    }

    pub(super) fn forward(listener: TcpListener, network: Arc<super::network::Network>, port: u16) -> Self {
        Self::run(listener, Service::Application(network, port))
    }

    fn run(listener: TcpListener, service: Service) -> Self {
        let stop = CancellationToken::new();
        let cancel = stop.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    () = cancel.cancelled() => break,
                    Some(_) = connections.join_next(), if !connections.is_empty() => {},
                    accepted = listener.accept() => {
                        let (socket, _) = match accepted {
                            Ok(socket) => socket,
                            Err(_) => {
                                tracing::error!("Wasm agent accept failed");
                                break;
                            }
                        };
                        if socket.set_nodelay(true).is_err() {
                            tracing::warn!("Cannot enable agent ingress TCP_NODELAY");
                        }
                        let service = service.clone();
                        connections.spawn(async move {
                            match service {
                                Service::Agent(router) => {
                                    let service = TowerToHyperService::new(router);
                                    let _ = hyper::server::conn::http1::Builder::new()
                                        .serve_connection(TokioIo::new(socket), service).with_upgrades().await;
                                }
                                Service::Application(network, port) => { let _ = network.serve(socket, port, Vec::new()).await; }
                            }
                        });
                    }
                }
            }
            connections.shutdown().await;
        });
        Self { stop, task: Some(task) }
    }

    pub(super) async fn shutdown(mut self) -> Result<(), super::RuntimeError> {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.await.map_err(super::error)?;
        }
        Ok(())
    }
}

impl Drop for Frontend {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn disposal_closes_keepalive_and_cancels_unfinished_uploads() {
        let wrote = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(tokio::sync::Notify::new());
        let recorded = wrote.clone();
        let started = entered.clone();
        let router = Router::new().route("/live", axum::routing::get(|| async { "live" })).route(
            "/write",
            axum::routing::put(move |body: axum::body::Body| {
                let recorded = recorded.clone();
                let started = started.clone();
                async move {
                    started.notify_one();
                    let _ = axum::body::to_bytes(body, 1024).await.unwrap();
                    recorded.store(true, Ordering::Release);
                    hyper::StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let frontend = Frontend::start(listener, router);
        let mut keepalive = tokio::net::TcpStream::connect(address).await.unwrap();
        keepalive.write_all(b"GET /live HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
        let mut bytes = [0; 1024];
        let mut response = String::new();
        while !response.ends_with("live") {
            let n = keepalive.read(&mut bytes).await.unwrap();
            assert_ne!(n, 0);
            response.push_str(std::str::from_utf8(&bytes[..n]).unwrap());
        }
        assert!(response.contains(hyper::StatusCode::OK.as_str()));
        let mut upload = tokio::net::TcpStream::connect(address).await.unwrap();
        upload
            .write_all(b"PUT /write HTTP/1.1\r\nHost: localhost\r\nContent-Length: 100\r\n\r\npartial")
            .await
            .unwrap();
        entered.notified().await;
        frontend.shutdown().await.unwrap();
        assert!(!wrote.load(Ordering::Acquire));
        for mut stream in [keepalive, upload] {
            let read = tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut bytes)).await.unwrap();
            assert!(matches!(read, Ok(0)) || read.is_err());
        }
    }
}
