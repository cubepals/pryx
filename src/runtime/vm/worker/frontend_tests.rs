// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! The frontend against the actual worker service lifetime over real private sockets. The fake
//! backend stands in for the VM: each verb must reach it once, with its own answers and errors,
//! and dropping the frontend must retire the worker. Real KVM runs in the existing VM job.
use super::{frontend::Frontend, service, transport::Endpoint};
use crate::runtime::{ContainerRuntime, ContainerSpec, ContainerStatus, LogOptions, RuntimeError, fake::FakeRuntime};
use futures_util::StreamExt;
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::time::timeout;

fn spec(root: &std::path::Path) -> ContainerSpec {
    serde_json::from_value(serde_json::json!({
        "name": "frontend-one",
        "image": "alpine:3.22",
        "env": [],
        "labels": { "pryx.test": "frontend" },
        "user": "",
        "memory_bytes": 268_435_456,
        "cpu_shares": 1024,
        "pids_limit": 128,
        "read_only_rootfs": true,
        "tmp_size_mb": 16,
        "data_dir": root.join("data"),
        "mount_path": "/data",
        "ports": [],
        "stop_signal": "SIGTERM",
        "stop_timeout_secs": 1,
        "network": "none",
        "internal_network": false,
        "log_max_size_mb": 1,
        "log_max_files": 1,
        "oom_score_adj": 0,
    }))
    .unwrap()
}

pub(crate) async fn worker(
    root: &std::path::Path,
    runtime: Arc<FakeRuntime>,
    decorate: impl FnOnce(axum::Router) -> axum::Router + Send + 'static,
) -> (Frontend, tokio::task::JoinHandle<std::io::Result<()>>, tokio::sync::oneshot::Receiver<()>) {
    let uid = rustix::process::geteuid().as_raw();
    let (lifetime, requests) = (root.join("lifetime.sock"), root.join("requests.sock"));
    let backend: Arc<dyn ContainerRuntime> = runtime;
    let (retire, retired) = tokio::sync::oneshot::channel();
    let served = tokio::spawn(service::serve_app(
        Endpoint::bind(&lifetime, uid).await.unwrap(),
        Endpoint::bind(&requests, uid).await.unwrap(),
        uid,
        move |authority| decorate(super::app(backend, authority)),
        async move {
            let _ = retire.send(());
            Ok(())
        },
    ));
    (Frontend::connect(&lifetime, &requests, uid).await.unwrap(), served, retired)
}

#[tokio::test]
async fn each_verb_reaches_the_worker_once_and_frontend_loss_retires_it() {
    let root = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
    let runtime = Arc::new(FakeRuntime::new());
    runtime.cold_data_copy.store(true, Ordering::SeqCst);
    let (frontend, served, mut retired) = worker(root.path(), runtime.clone(), std::convert::identity).await;
    assert_eq!(frontend.isolation(), crate::runtime::vm::instance::ISOLATION);
    assert_eq!(frontend.requires_stopped_data_copy(), runtime.requires_stopped_data_copy());
    assert_eq!(frontend.uses_remote_data(), runtime.uses_remote_data());
    assert_eq!(frontend.supports_hostname_egress(), runtime.supports_hostname_egress());
    assert_eq!(frontend.data_owner("any").unwrap(), runtime.data_owner("any").unwrap());
    let spec = spec(root.path());
    let id = frontend.create(&spec).await.unwrap();
    let conflict = frontend.create(&spec).await.unwrap_err();
    let actual = runtime.create(&spec).await.unwrap_err();
    assert_eq!((conflict.to_string(), format!("{conflict:?}")), (actual.to_string(), format!("{actual:?}")));
    frontend.start(&spec.name).await.unwrap();
    let info = frontend.inspect(&spec.name).await.unwrap().unwrap();
    assert_eq!((info.id.as_str(), info.status), (id.as_str(), ContainerStatus::Running));
    assert_eq!(frontend.list(&["pryx.test=frontend".into()]).await.unwrap().len(), 1);
    let output = frontend.exec(&id, &["printf".into(), "once".into()], Duration::from_secs(1), 64).await.unwrap();
    assert_eq!(output.stdout, b"printf once");
    let stats = frontend.stats(&spec.name).await.unwrap().unwrap();
    assert_eq!(stats.pids, runtime.stats(&spec.name).await.unwrap().unwrap().pids);
    let missing = frontend.logs("missing", LogOptions::default()).next().await.unwrap().unwrap_err();
    let actual = runtime.logs("missing", LogOptions::default()).next().await.unwrap().unwrap_err();
    assert_eq!(format!("{missing:?}"), format!("{actual:?}"));
    let mut events = frontend.events(&[]);
    let observed = tokio::spawn(async move { events.next().await });
    // The worker subscribes when it handles the stream; exits repeat until one is observed.
    timeout(Duration::from_secs(5), async {
        while !observed.is_finished() {
            runtime.crash(&spec.name, 7, false);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let event = observed.await.unwrap().unwrap().unwrap();
    assert_eq!((event.container_id.as_str(), event.exit_code), (id.as_str(), Some(7)));
    frontend.remove(&spec.name).await.unwrap();
    assert!(frontend.inspect(&spec.name).await.unwrap().is_none());
    for (verb, count) in [("create ", 3), ("start ", 1), ("exec ", 1), ("remove ", 1)] {
        assert_eq!(runtime.count_calls(verb), count, "{verb}");
    }
    assert!(retired.try_recv().is_err());
    drop(frontend);
    timeout(Duration::from_secs(5), retired).await.unwrap().unwrap();
    timeout(Duration::from_secs(5), served).await.unwrap().unwrap().unwrap();
}

#[tokio::test]
async fn an_installed_frontend_waits_for_its_worker_and_a_second_lifetime_is_refused() {
    let root = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
    let state = root.path().to_path_buf();
    let uid = rustix::process::geteuid().as_raw();
    let (lifetime, requests) = service::paths(&state);
    assert!(matches!(Frontend::available_as(&state, uid), Err(RuntimeError::Unavailable(_))));
    let backend: Arc<dyn ContainerRuntime> = Arc::new(FakeRuntime::new());
    let (worker_lifetime, worker_requests) = (lifetime.clone(), requests.clone());
    // The worker starts after the frontend: its socket is absent until then.
    let served = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        fs::DirBuilder::new().mode(0o700).create(worker_lifetime.parent().unwrap()).unwrap();
        let requests = Endpoint::bind(&worker_requests, uid).await.unwrap();
        let lifetime = Endpoint::bind(&worker_lifetime, uid).await.unwrap();
        service::serve_app(lifetime, requests, uid, move |authority| super::app(backend, authority), async { Ok(()) })
            .await
    });
    let frontend = Frontend::installed_as(&state, uid).await.unwrap();
    Frontend::available_as(&state, uid).unwrap();
    // The served lifetime is the only one: another frontend is refused at once, not queued.
    let second = timeout(Duration::from_secs(2), Frontend::connect(&lifetime, &requests, uid)).await.unwrap();
    assert!(matches!(second, Err(RuntimeError::Unavailable(_))));
    assert!(frontend.list(&[]).await.unwrap().is_empty());
    drop(frontend);
    timeout(Duration::from_secs(5), served).await.unwrap().unwrap().unwrap();
}
