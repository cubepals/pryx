// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real private HTTP file transfers, backend locks and cancellation-owned cleanup.
//! The existing fake runtime supplies a real signed agent session, without a VM or engine.
use super::{
    authority::{Authority, Owner},
    data_client::Client,
    file_operations, http, sessions,
    transport::Endpoint,
};
use crate::{
    http_client,
    runtime::{
        fake::FakeRuntime,
        files::{self, Operation, Service},
    },
};
use axum::{
    Router,
    body::Body,
    routing::{get, put},
};
use futures_util::stream;
use http_body_util::BodyExt;
use pryx_agent::auth::{Scope, SigningKey};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Notify, Semaphore},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

fn lock(path: &std::path::Path) -> File {
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(path).unwrap();
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    file
}

fn occupied(observer: &File) {
    assert_eq!(
        rustix::fs::flock(observer, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap_err(),
        rustix::io::Errno::WOULDBLOCK
    );
}

fn request(root: &std::path::Path) -> files::Request {
    files::Request {
        name: "files".into(),
        sandbox: "one".into(),
        epoch: 7,
        data_dir: root.into(),
        mount_path: "/data".into(),
        user: String::new(),
        memory_bytes: 0,
        storage_bytes: 0,
        pids_limit: 0,
        tmp_size_mb: 0,
        log_max_size_mb: 0,
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    owner: Option<Owner>,
    observer: File,
    data_observer: File,
    retired: tokio::task::JoinHandle<std::io::Result<()>>,
    runtime: Arc<FakeRuntime>,
    calls: Arc<AtomicUsize>,
    started: Arc<Notify>,
    finishing: Arc<Semaphore>,
    cancel: CancellationToken,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
    agent: tokio::task::JoinHandle<()>,
    files: Option<Service>,
}

// The fake agent verifies the session's signed identity and echoes writes; reads never end.
fn agent(public: String, calls: Arc<AtomicUsize>) -> Router {
    Router::new()
        .route(
            "/_pryx/files/content",
            put(move |request: axum::extract::Request| {
                let public = public.clone();
                let count = calls.clone();
                async move {
                    let token = request.headers()["authorization"].to_str().unwrap().strip_prefix("Bearer ").unwrap();
                    let claims =
                        pryx_agent::auth::verify(token, &public, "one", 7, crate::clock::now().unix_timestamp())
                            .unwrap();
                    assert_eq!(claims.scope, Scope::Sandbox);
                    assert_eq!(request.uri().query(), Some("path=name%26path%3Dother"));
                    count.fetch_add(1, Ordering::SeqCst);
                    let body = request.into_body().collect().await.unwrap().to_bytes();
                    Body::from(body)
                }
            })
            .merge(get(|| async {
                Body::from_stream(stream::repeat_with(|| {
                    Ok::<_, std::io::Error>(bytes::Bytes::from(vec![1; 256 * 1024]))
                }))
            })),
        )
        .layer(axum::extract::DefaultBodyLimit::disable())
}

async fn fixture() -> Fixture {
    let root = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
    let path = root.path().join("worker.sock");
    let uid = rustix::process::geteuid().as_raw();
    let endpoint = Endpoint::bind(&path, uid).await.unwrap();
    let (owner, lifetime) = tokio::net::UnixStream::pair().unwrap();
    let lease_path = root.path().join("data.lock");
    let compute = lock(&root.path().join("compute.lock"));
    let observer = OpenOptions::new().read(true).write(true).open(root.path().join("compute.lock")).unwrap();
    let (authority, retired) = Authority::watch(lifetime, uid, async move {
        drop(compute);
        Ok(())
    })
    .unwrap();
    let owner = Owner::ready(owner).await.unwrap();
    let runtime = Arc::new(FakeRuntime::new());
    let (key, encoded) = SigningKey::generate().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = agent(key.public_key(), calls.clone());
    let agent = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let started = Arc::new(Notify::new());
    let finishing = Arc::new(Semaphore::new(0));
    let cleanup_started = started.clone();
    let cleanup_gate = finishing.clone();
    *runtime.remote_files.lock().unwrap() = Some(Arc::new(move |request| {
        let lease = lock(&lease_path);
        let started = cleanup_started.clone();
        let gate = cleanup_gate.clone();
        let key = SigningKey::from_encoded(&encoded).unwrap();
        Service::new(address, key, (request.sandbox, request.epoch), Duration::from_secs(1), async move {
            started.notify_one();
            let _finish = gate.acquire().await.unwrap();
            drop(lease);
            Ok(())
        })
    }));
    let sessions = sessions::Sessions::new(runtime.clone(), authority);
    let app = sessions::app(sessions.clone()).merge(file_operations::router(sessions));
    let cancel = CancellationToken::new();
    let server = tokio::spawn(http::serve(endpoint, app, cancel.clone()));
    let client = Client { socket: path, uid, timeout: Duration::from_secs(1) };
    let files = client.files(request(root.path())).await.unwrap();
    let data_observer = OpenOptions::new().read(true).write(true).open(root.path().join("data.lock")).unwrap();
    occupied(&data_observer);
    Fixture {
        _root: root,
        owner: Some(owner),
        observer,
        data_observer,
        retired,
        runtime,
        calls,
        started,
        finishing,
        cancel,
        server,
        agent,
        files: Some(files),
    }
}

async fn finish(fixture: Fixture) {
    assert_eq!(fixture.runtime.calls.lock().unwrap().iter().filter(|call| call.starts_with("files ")).count(), 1);
    fixture.cancel.cancel();
    timeout(Duration::from_secs(2), fixture.server).await.unwrap().unwrap().unwrap();
    fixture.agent.abort();
}

#[tokio::test]
async fn owner_loss_retires_an_unread_file_stream_and_its_real_backend_lease() {
    let mut fixture = fixture().await;
    let files = fixture.files.take().unwrap();
    let response = files.request(Operation::Read, "unread", http_client::empty()).await.unwrap();
    occupied(&fixture.observer);
    drop(fixture.owner.take());
    timeout(Duration::from_secs(2), fixture.started.notified()).await.unwrap();
    occupied(&fixture.observer);
    occupied(&fixture.data_observer);
    fixture.finishing.add_permits(1);
    timeout(Duration::from_secs(2), &mut fixture.retired).await.unwrap().unwrap().unwrap();
    rustix::fs::flock(&fixture.observer, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    rustix::fs::flock(&fixture.data_observer, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    drop(response);
    drop(files);
    finish(fixture).await;
}

#[tokio::test]
async fn streamed_file_close_remains_owned_after_its_caller_is_cancelled() {
    let mut fixture = fixture().await;
    let files = fixture.files.take().unwrap();
    let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(http::MAX_CONTROL_BYTES + 1).collect();
    let payload = bytes.clone();
    // The body arrives after the client's one-second deadline: only connection setup is bounded.
    let delayed = stream::once(async move {
        tokio::time::sleep(Duration::from_millis(1100)).await;
        Ok::<_, std::io::Error>(hyper::body::Frame::data(bytes::Bytes::from(payload)))
    });
    let response = files
        .request(Operation::Write, "name&path=other", BodyExt::boxed(http_body_util::StreamBody::new(delayed)))
        .await
        .unwrap();
    assert_eq!(response.status(), hyper::StatusCode::OK);
    assert_eq!(http_client::read_body(response, bytes.len()).await.unwrap().as_ref(), bytes);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    let close = tokio::spawn(files.close());
    timeout(Duration::from_secs(2), fixture.started.notified()).await.unwrap();
    close.abort();
    assert!(close.await.unwrap_err().is_cancelled());
    occupied(&fixture.data_observer);
    fixture.finishing.add_permits(1);
    timeout(Duration::from_secs(2), async {
        loop {
            match rustix::fs::flock(&fixture.data_observer, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => break,
                Err(error) if error == rustix::io::Errno::WOULDBLOCK => tokio::task::yield_now().await,
                Err(error) => panic!("{error}"),
            }
        }
    })
    .await
    .unwrap();
    drop(fixture.owner.take());
    timeout(Duration::from_secs(2), &mut fixture.retired).await.unwrap().unwrap().unwrap();
    finish(fixture).await;
}
