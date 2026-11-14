// SPDX-License-Identifier: FSL-1.1-ALv2
//! Authenticated streaming file sessions keep backend cleanup owned after caller cancellation.
//! Only four fixed data operations are available. This transport never selects a workload,
//! boots compute, retries a mutation or grants a caller a different backend policy.
use super::RuntimeError;
use crate::http_client;
use hyper::{Method, Response, body::Incoming};
use pryx_agent::auth::{Claims, Scope, SigningKey, TOKEN_LIFETIME_SECONDS};
use std::{future::Future, net::SocketAddr, time::Duration};

pub enum Operation {
    Read,
    Write,
    List,
    Stat,
}
impl Operation {
    fn route(&self) -> (&'static str, Method) {
        match self {
            Self::Read => ("content", Method::GET),
            Self::Write => ("content", Method::PUT),
            Self::List => ("list", Method::GET),
            Self::Stat => ("stat", Method::GET),
        }
    }
}
pub struct Service {
    endpoint: SocketAddr,
    key: SigningKey,
    sandbox: String,
    epoch: u64,
    timeout: Duration,
    release: Option<tokio::sync::oneshot::Sender<()>>,
    cleanup: Option<tokio::task::JoinHandle<Result<(), RuntimeError>>>,
}
impl Service {
    pub fn new(
        endpoint: SocketAddr,
        key: SigningKey,
        identity: (String, u64),
        timeout: Duration,
        cleanup: impl Future<Output = Result<(), RuntimeError>> + Send + 'static,
    ) -> Result<Self, RuntimeError> {
        let private = match endpoint.ip() {
            std::net::IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
            std::net::IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
        };
        if !private || endpoint.port() == 0 || timeout.is_zero() {
            return Err(RuntimeError::Other("File service requires a private endpoint and positive deadline".into()));
        }
        let (release, released) = tokio::sync::oneshot::channel();
        let cleanup = tokio::spawn(async move {
            let _ = released.await;
            let result = cleanup.await;
            if let Err(problem) = &result {
                tracing::warn!(%problem, "File-service cleanup failed");
            }
            result
        });
        Ok(Self {
            endpoint,
            key,
            sandbox: identity.0,
            epoch: identity.1,
            timeout,
            release: Some(release),
            cleanup: Some(cleanup),
        })
    }
    pub async fn request(
        &self,
        operation: Operation,
        path: &str,
        body: http_client::Body,
    ) -> Result<Response<Incoming>, RuntimeError> {
        let (route, method) = operation.route();
        let mut url = reqwest::Url::parse(&format!("http://{}/_pryx/files/{route}", self.endpoint))
            .map_err(|_| RuntimeError::Other("Invalid file-service endpoint".into()))?;
        url.query_pairs_mut().append_pair("path", path);
        let now = crate::clock::now().unix_timestamp();
        let token = self
            .key
            .sign(&Claims {
                sandbox: self.sandbox.clone(),
                epoch: self.epoch,
                issued_at: now,
                expires_at: now + TOKEN_LIFETIME_SECONDS,
                scope: Scope::Sandbox,
                max_ports: 0,
                max_exec_seconds: None,
                file: None,
            })
            .map_err(|_| RuntimeError::Other("Cannot authorize file service".into()))?;
        http_client::send(
            method,
            url.as_str(),
            &[("authorization", format!("Bearer {token}"))],
            body,
            None,
            self.timeout,
        )
        .await
        .map_err(|problem| RuntimeError::Other(problem.to_string()))
    }
    /// Cleanup stays in its own task if waiting for it is cancelled.
    pub async fn close(mut self) -> Result<(), RuntimeError> {
        self.release.take();
        self.cleanup
            .take()
            .ok_or_else(|| RuntimeError::Other("File service is closed".into()))?
            .await
            .map_err(|problem| RuntimeError::Other(problem.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, extract::State, http::StatusCode, routing::put};
    use http_body_util::BodyExt;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn streamed_requests_are_scoped_once_and_cleanup_outlives_cancelled_close() {
        let (key, _) = SigningKey::generate().unwrap();
        let public = key.public_key();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let app = Router::new()
            .route(
                "/_pryx/files/content",
                put(move |State(public): State<String>, request: axum::extract::Request| {
                    let count = count.clone();
                    async move {
                        let token =
                            request.headers()["authorization"].to_str().unwrap().strip_prefix("Bearer ").unwrap();
                        let claims =
                            pryx_agent::auth::verify(token, &public, "one", 7, crate::clock::now().unix_timestamp())
                                .unwrap();
                        assert_eq!(claims.scope, Scope::Sandbox);
                        assert_eq!(request.uri().query(), Some("path=name%26path%3Dother"));
                        count.fetch_add(1, Ordering::SeqCst);
                        let bytes = request.into_body().collect().await.unwrap().to_bytes();
                        (StatusCode::OK, Body::from(bytes))
                    }
                })
                .layer(axum::extract::DefaultBodyLimit::disable()),
            )
            .with_state(public);
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        let held = gate.clone().lock_owned().await;
        let (finish, finishing) = tokio::sync::oneshot::channel();
        let (started, starting) = tokio::sync::oneshot::channel();
        let service = Service::new(address, key, ("one".into(), 7), Duration::from_secs(5), async move {
            let _ = started.send(());
            let _ = finishing.await;
            drop(held);
            Ok(())
        })
        .unwrap();
        let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(pryx_agent::contract::MAX_REQUEST_BYTES + 1).collect();
        let response =
            service.request(Operation::Write, "name&path=other", http_client::full(bytes.clone())).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(http_client::read_body(response, bytes.len()).await.unwrap().as_ref(), bytes);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let closing = tokio::spawn(service.close());
        starting.await.unwrap();
        closing.abort();
        assert!(closing.await.unwrap_err().is_cancelled());
        assert!(gate.try_lock().is_err(), "Cancelled caller must not release backend authority");
        finish.send(()).unwrap();
        let _released = tokio::time::timeout(Duration::from_secs(1), gate.lock()).await.unwrap();
        server.abort();
    }
}
