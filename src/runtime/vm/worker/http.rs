// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! HTTP calls over the worker's kernel-authenticated Unix transport. Hyper owns framing and
//! Axum owns serving; each operation opens one connection and is submitted once. Control
//! messages are bounded, while separately admitted file bodies can stream through `send`.
use super::transport::{self, Endpoint};
use crate::{http_client, runtime::RuntimeError};
use hyper::{
    Method, Request, Response,
    body::Incoming,
    header::{CONTENT_TYPE, HOST},
};
use hyper_util::rt::TokioIo;
use serde::{Serialize, de::DeserializeOwned};
use std::path::Path;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

// This private control transport's allocation ceiling. File streams have separate admission.
pub const MAX_CONTROL_BYTES: usize = 16 * 1024 * 1024;
pub const CONTROL_PATH: &str = "/runtime/v1";

fn unavailable(problem: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Unavailable(format!("VM worker transport failed: {problem}"))
}

pub async fn send(
    path: &Path,
    owner: u32,
    method: Method,
    route: &str,
    body: http_client::Body,
    content_type: &str,
    deadline: Instant,
) -> Result<Response<Incoming>, RuntimeError> {
    tokio::time::timeout_at(deadline, stream(path, owner, method, route, body, content_type, deadline))
        .await
        .map_err(|_| RuntimeError::Timeout("VM worker response exceeded its deadline".into()))?
}

/// Bound connection setup while allowing an admitted file body to stream until completion.
pub async fn stream(
    path: &Path,
    owner: u32,
    method: Method,
    route: &str,
    body: http_client::Body,
    content_type: &str,
    deadline: Instant,
) -> Result<Response<Incoming>, RuntimeError> {
    let mut sender = tokio::time::timeout_at(deadline, async {
        let socket = transport::connect(path, owner).await.map_err(unavailable)?;
        let (sender, connection) =
            hyper::client::conn::http1::handshake(TokioIo::new(socket)).await.map_err(unavailable)?;
        tokio::spawn(async move {
            // A lost response never authorizes another submission. Dropping the response
            // closes its one connection; admitted backend cleanup keeps its own ownership.
            let _ = connection.await;
        });
        Ok::<_, RuntimeError>(sender)
    })
    .await
    .map_err(|_| RuntimeError::Timeout("VM worker connection exceeded its deadline".into()))??;
    let request = Request::builder()
        .method(method)
        .uri(route)
        .header(HOST, "pryx-worker")
        .header(CONTENT_TYPE, content_type)
        .body(body)
        .map_err(|_| unavailable("Invalid private worker route"))?;
    sender.send_request(request).await.map_err(unavailable)
}

pub async fn control<T: DeserializeOwned>(
    path: &Path,
    owner: u32,
    request: &impl Serialize,
    deadline: Instant,
) -> Result<T, RuntimeError> {
    control_at(path, owner, CONTROL_PATH, request, deadline).await
}

pub async fn control_at<T: DeserializeOwned>(
    path: &Path,
    owner: u32,
    route: &str,
    request: &impl Serialize,
    deadline: Instant,
) -> Result<T, RuntimeError> {
    let bytes = serde_json::to_vec(request).map_err(|_| unavailable("Cannot encode private worker command"))?;
    if bytes.len() > MAX_CONTROL_BYTES {
        return Err(unavailable("Private worker command exceeds its bound"));
    }
    let response =
        send(path, owner, Method::POST, route, http_client::full(bytes), "application/json", deadline).await?;
    if !response.status().is_success() {
        return Err(unavailable(format!("Private worker refused HTTP {}", response.status())));
    }
    let bytes = tokio::time::timeout_at(deadline, http_client::read_body(response, MAX_CONTROL_BYTES))
        .await
        .map_err(|_| RuntimeError::Timeout("VM worker control body exceeded its deadline".into()))?
        .map_err(unavailable)?;
    // JSON errors may quote credential-bearing input. Keep those bytes out of errors/logs.
    serde_json::from_slice(&bytes).map_err(|_| unavailable("Invalid private worker answer"))
}

pub async fn serve(endpoint: Endpoint, app: axum::Router, cancel: CancellationToken) -> std::io::Result<()> {
    axum::serve(endpoint, app.into_make_service_with_connect_info::<transport::Peer>())
        .with_graceful_shutdown(cancel.cancelled_owned())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    #[tokio::test]
    async fn an_invalid_answer_after_mutation_is_never_retried_or_quoted() {
        let root = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
        let path = root.path().join("worker.sock");
        let owner = rustix::process::geteuid().as_raw();
        let endpoint = Endpoint::bind(&path, owner).await.unwrap();
        let mutations = Arc::new(AtomicUsize::new(0));
        let count = mutations.clone();
        let challenge = uuid::Uuid::new_v4().to_string();
        let private = challenge.clone();
        let app = Router::new().route(
            CONTROL_PATH,
            post(move |Json(value): Json<String>| {
                let count = count.clone();
                let private = private.clone();
                async move {
                    assert_eq!(value, private);
                    count.fetch_add(1, Ordering::SeqCst);
                    Json(private)
                }
            }),
        );
        let cancel = CancellationToken::new();
        let server = tokio::spawn(serve(endpoint, app, cancel.clone()));
        let result = control::<u64>(&path, owner, &challenge, Instant::now() + Duration::from_secs(2)).await;
        let problem = result.unwrap_err().to_string();
        assert!(!problem.contains(&challenge));
        assert_eq!(mutations.load(Ordering::SeqCst), 1);
        cancel.cancel();
        server.await.unwrap().unwrap();
    }
}
