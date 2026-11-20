// SPDX-License-Identifier: FSL-1.1-ALv2
//! A manager snapshot turns over a real worker data lock before asking the stopped compute
//! to start again. The backend and its data are owned test fixtures; no VM monitor is started.
use super::{Manager, NodeError, Precondition, Records};
use crate::{
    config::Config,
    ids::{SnapshotId, WorkloadId},
    metrics::Metrics,
    protocol::{SnapshotRequest, WorkloadSpec, WorkloadState},
    runtime::{
        ContainerStatus, DataCopyLease,
        fake::FakeRuntime,
        vm::worker::{commands::Command, frontend_tests::worker, http, sessions},
    },
    store::Store,
};
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
};
use futures_util::{StreamExt, stream};
use http_body_util::BodyExt;
use rustix::fs::{FlockOperation, flock};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};
use tokio::{
    sync::{Notify, mpsc},
    time::timeout,
};

const DEADLINE: Duration = Duration::from_secs(20);
const SNAPSHOT: &str = "0b6f1f2e-6a47-4c9a-9a39-2f4ac7e51f10";
const CONTENTS: &[u8] = b"the stopped workload's data";

#[derive(Debug, PartialEq, Eq)]
enum RequestOrder {
    Close,
    PrematureStart,
}

#[derive(Clone, Copy)]
enum CloseReply {
    Confirmed,
    Lost,
}

struct ReleaseGate {
    response: Mutex<Option<Body>>,
    requests: mpsc::UnboundedSender<RequestOrder>,
    release: Notify,
    close_reply: CloseReply,
}

async fn delay_response_drop(State(gate): State<Arc<ReleaseGate>>, request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, http::MAX_CONTROL_BYTES).await.unwrap();
    let session = (parts.uri.path() == sessions::PATH).then(|| serde_json::from_slice(&bytes).unwrap());
    let start = parts.uri.path() == http::CONTROL_PATH
        && matches!(serde_json::from_slice(&bytes).unwrap(), Command::Start { .. });
    let request = Request::from_parts(parts, Body::from(bytes));
    match session {
        Some(sessions::Request::Copy { .. }) => {
            let (parts, mut body) = next.run(request).await.into_parts();
            let ready = body.frame().await.unwrap().unwrap().into_data().unwrap();
            // Deliver the worker's real readiness frame but keep the response lifetime alive,
            // modeling the server that has yet to see its client's dropped connection.
            assert!(gate.response.lock().unwrap().replace(body).is_none());
            let body = Body::from_stream(stream::iter([Ok::<_, std::io::Error>(ready)]).chain(stream::pending()));
            Response::from_parts(parts, body)
        }
        Some(sessions::Request::Close { .. }) => {
            gate.requests.send(RequestOrder::Close).unwrap();
            gate.release.notified().await;
            let response = next.run(request).await;
            drop(gate.response.lock().unwrap().take());
            match gate.close_reply {
                CloseReply::Confirmed => response,
                CloseReply::Lost => {
                    Response::builder().status(StatusCode::SERVICE_UNAVAILABLE).body(Body::empty()).unwrap()
                }
            }
        }
        _ => {
            if start && gate.response.lock().unwrap().is_some() {
                gate.requests.send(RequestOrder::PrematureStart).unwrap();
            }
            next.run(request).await
        }
    }
}

fn config(state: &Path) -> Config {
    let uid = rustix::process::geteuid().as_raw();
    let gid = rustix::process::getegid().as_raw();
    let data_owner = if uid == 0 { String::new() } else { format!("data_owner = '{uid}:{gid}'") };
    toml::from_str(&format!(
        "node_id = 'test-node'\ndeployment_id = 'test'\nstate_dir = {state:?}\n\
         [api]\nlisten = '127.0.0.1:0'\n[capacity]\nmin_free_disk_mb = 64\n\
         [workloads]\nallowed_images = ['alpine:']\n{data_owner}"
    ))
    .unwrap()
}

#[tokio::test]
async fn running_snapshot_waits_for_the_worker_copy_lease() {
    snapshot_with_worker_copy_close(CloseReply::Confirmed).await;
}

#[tokio::test]
async fn running_snapshot_stays_stopped_when_worker_copy_release_is_unconfirmed() {
    snapshot_with_worker_copy_close(CloseReply::Lost).await;
}

async fn snapshot_with_worker_copy_close(close_reply: CloseReply) {
    timeout(DEADLINE, async {
        let directory = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
        let (requests, mut received) = mpsc::unbounded_channel();
        let gate = Arc::new(ReleaseGate { response: Mutex::new(None), requests, release: Notify::new(), close_reply });
        let layer = middleware::from_fn_with_state(gate.clone(), delay_response_drop);
        let backend = Arc::new(FakeRuntime::new());
        backend.cold_data_copy.store(true, Ordering::SeqCst);
        let (frontend, served, retired) = worker(directory.path(), backend.clone(), |app| app.layer(layer)).await;
        let config = Arc::new(config(&directory.path().join("state")));
        let store = Store::open(&config.state_dir).unwrap();
        let manager = Manager::new(config, Arc::new(frontend), store, Arc::new(Metrics::new()), Arc::new(|_, _| true));
        let report = manager.reconcile(Records::FromDisk).await;
        assert!(report.error.is_none(), "{report:?}");
        let id = WorkloadId::parse("copy-source").unwrap();
        let spec: WorkloadSpec = serde_json::from_value(serde_json::json!({
            "image": "alpine:3.22",
            "resources": { "memoryMb": 256 },
            "storage": { "mountPath": "/data", "sizeGb": 1 }
        }))
        .unwrap();
        let workload = manager.ensure(id.clone(), spec, Precondition::None, Some(1)).await.unwrap().workload;
        manager.start(id.clone(), Some(1)).await.unwrap();
        assert_eq!(backend.container(&workload.locate.container_name).unwrap().status, ContainerStatus::Running);
        fs::create_dir(manager.store.data_dir(&id).join("tree")).unwrap();
        fs::write(manager.store.data_dir(&id).join("tree/file"), CONTENTS).unwrap();

        let lock_path = directory.path().join("copy.lock");
        let lease = File::create(&lock_path).unwrap();
        flock(&lease, FlockOperation::NonBlockingLockExclusive).unwrap();
        *backend.copy_lease.lock().unwrap() = Some(DataCopyLease::local(lease));
        let observer = OpenOptions::new().read(true).write(true).open(lock_path).unwrap();
        let snapshot = SnapshotId::parse(SNAPSHOT).unwrap();
        let copied = manager.store.snapshot_dir(&id, &snapshot).join("data/tree/file");
        let request = SnapshotRequest { id: snapshot, quiesced: true };
        let mut task = {
            let (manager, id) = (manager.clone(), id.clone());
            tokio::spawn(async move { manager.snapshot(id, request, Some(1)).await })
        };

        let next = tokio::select! {
            biased;
            next = received.recv() => next,
            result = &mut task => panic!("snapshot returned before the worker released the copy lease: {result:?}"),
        };
        assert_eq!(fs::read(&copied).unwrap(), CONTENTS, "the snapshot data is complete before release");
        assert_eq!(
            flock(&observer, FlockOperation::NonBlockingLockExclusive).unwrap_err(),
            rustix::io::Errno::WOULDBLOCK,
            "the worker still owns the copy lease"
        );
        assert_eq!(next, Some(RequestOrder::Close), "compute must wait for the worker to confirm release");
        assert_eq!(backend.container(&workload.locate.container_name).unwrap().status, ContainerStatus::Exited);
        assert_eq!(backend.count_calls("start "), 1);

        gate.release.notify_one();
        let result = task.await.unwrap();
        assert!(received.try_recv().is_err(), "the manager submitted Start before the worker acknowledged release");
        flock(&observer, FlockOperation::NonBlockingLockExclusive).unwrap();
        match close_reply {
            CloseReply::Confirmed => {
                assert!(result.unwrap().created);
                assert_eq!(backend.count_calls("start "), 2);
                assert_eq!(manager.view(&id).unwrap().state, WorkloadState::Running);
                assert_eq!(fs::read(copied).unwrap(), CONTENTS);
            }
            CloseReply::Lost => {
                assert!(matches!(result, Err(NodeError::RuntimeUnavailable(_))), "{result:?}");
                assert_eq!(backend.count_calls("start "), 1);
                assert_eq!(backend.container(&workload.locate.container_name).unwrap().status, ContainerStatus::Exited);
                assert!(!copied.exists(), "a snapshot with unconfirmed release is unpublished and cleaned");
            }
        }

        drop(manager);
        retired.await.unwrap();
        served.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}
