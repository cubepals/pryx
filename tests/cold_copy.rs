// SPDX-License-Identifier: FSL-1.1-ALv2
//! Manager copy coordination against its in-memory runtime and a real stalled HTTP sink.
//! This proves cancellation, locking and resume behavior; real disk leases and KVM are CI's.
mod support;
use axum::{Router, extract::State, http::StatusCode, routing::put};
use pryxd::{manager::Precondition, protocol::WorkloadState};
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};
use support::{fixture, id, spec};

type Gate = Arc<Mutex<Option<(tokio::sync::oneshot::Sender<()>, tokio::sync::oneshot::Receiver<()>)>>>;

struct Export {
    fixture: support::Fixture,
    caller: tokio::task::JoinHandle<Result<pryxd::protocol::ExportResponse, pryxd::manager::NodeError>>,
    release: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}
async fn stalled_export() -> Export {
    let f = fixture("").await;
    f.fake.cold_data_copy.store(true, Ordering::SeqCst);
    f.manager.ensure(id("w"), spec(), Precondition::None, Some(1)).await.unwrap();
    f.manager.start(id("w"), Some(1)).await.unwrap();
    let (held_tx, held_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let state = Arc::new(Mutex::new(Some((held_tx, release_rx))));
    let app = Router::new()
        .route(
            "/archive",
            put(|State(state): State<Gate>| async move {
                let (held, release) = state.lock().unwrap().take().unwrap();
                held.send(()).unwrap();
                release.await.unwrap();
                StatusCode::INTERNAL_SERVER_ERROR
            }),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/archive", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let request = serde_json::from_value(serde_json::json!({"url":url,"quiesced":true})).unwrap();
    let manager = f.manager.clone();
    let caller = tokio::spawn(async move { manager.export(id("w"), request, Some(1)).await });
    tokio::time::timeout(Duration::from_secs(5), held_rx).await.unwrap().unwrap();
    assert_eq!(f.fake.container("blockly-test-w").unwrap().status, pryxd::runtime::ContainerStatus::Exited);
    Export { fixture: f, caller, release: release_tx, server }
}

#[tokio::test]
async fn cancelled_cold_export_keeps_its_lock_and_resumes_after_upload_failure() {
    let Export { fixture: f, caller, release, server } = stalled_export().await;
    let reserved = f.manager.capacity().running_memory_mb;
    f.manager.stats(&id("w")).await.unwrap();
    assert!(reserved > 0);
    assert_eq!(f.manager.capacity().running_memory_mb, reserved, "Stopped copy retains its resume reservation");
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), f.manager.start(id("w"), Some(1))).await.is_err(),
        "Caller cancellation must not release the copy's workload lock"
    );
    assert_eq!(f.fake.count_calls("start blockly-test-w"), 1);
    release.send(()).unwrap();
    support::until(&f.manager, "w", "cold resume after failed export", |view| view.state == WorkloadState::Running)
        .await;
    assert_eq!(f.fake.count_calls("start blockly-test-w"), 2);
    server.abort();
}

#[tokio::test]
async fn a_fence_during_a_cold_export_forbids_its_resume() {
    let Export { fixture: f, caller, release, server } = stalled_export().await;
    f.manager.fence_soon(id("w"), 2);
    release.send(()).unwrap();
    assert!(caller.await.unwrap().is_err());
    support::until(&f.manager, "w", "fence after copy", |view| view.state == WorkloadState::Fenced).await;
    assert_eq!(f.fake.count_calls("start blockly-test-w"), 1, "Copy completion must not restart a fenced guest");
    assert_eq!(f.manager.capacity().running_memory_mb, 0);
    server.abort();
}
