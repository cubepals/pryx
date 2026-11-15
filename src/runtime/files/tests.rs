// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real HTTP streaming and cancellation-owned file-session cleanup acceptance.
use super::*;
use crate::http_client;
use axum::{Router, body::Body, extract::State, http::StatusCode, routing::put};
use http_body_util::BodyExt;
use pryx_agent::auth::Scope;
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
                    let token = request.headers()["authorization"].to_str().unwrap().strip_prefix("Bearer ").unwrap();
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
    let service = Service::new(address, key, ("one".into(), 7), Duration::from_secs(1), async move {
        let _ = started.send(());
        let _ = finishing.await;
        drop(held);
        Ok(())
    })
    .unwrap();
    let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(pryx_agent::contract::MAX_REQUEST_BYTES + 1).collect();
    let payload = bytes.clone();
    let delayed = futures_util::stream::once(async move {
        tokio::time::sleep(Duration::from_millis(1100)).await;
        Ok::<_, std::io::Error>(hyper::body::Frame::data(bytes::Bytes::from(payload)))
    });
    let body = BodyExt::boxed(http_body_util::StreamBody::new(delayed));
    let response = service.request(Operation::Write, "name&path=other", body).await.unwrap();
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
