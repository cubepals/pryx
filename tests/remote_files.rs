// SPDX-License-Identifier: FSL-1.1-ALv2
//! Node mTLS file APIs proxy the real authenticated agent router and retain their guards
//! through cancelled cleanup. The in-memory compute seam proves routing and ownership only;
//! actual VM disks, confinement and booted file guests are the optional KVM workflow's proof.
mod support;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use pryx_agent::{auth::SigningKey, paths::DataDir, ports::Ports, process::Processes, server::Agent};
use pryxd::{manager::Precondition, runtime::files::Service};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use support::{CLIENT, client, fixture, id, pki, request, serve, spec};

#[tokio::test]
async fn remote_files_preserve_epochs_shapes_streams_and_owned_cleanup() {
    let f = fixture("").await;
    let key = id("files");
    f.manager.ensure(key.clone(), spec(), Precondition::None, Some(7)).await.unwrap();
    let memory_mb =
        serde_json::to_value(f.manager.view(&key).unwrap()).unwrap()["resources"]["memoryMb"].as_u64().unwrap();
    let root = f.manager.store.data_dir(&key);
    let (signing, encoded) = SigningKey::generate().unwrap();
    let agent = Arc::new(Agent {
        sandbox: key.to_string(),
        epoch: 7,
        public_key: signing.public_key(),
        key: signing,
        file_tasks: Default::default(),
        shells: Default::default(),
        telemetry: Arc::default(),
        data: DataDir::open(&root).unwrap(),
        logs: Arc::new(pryx_agent::logbook::Logbook::open(&root).unwrap()),
        processes: Processes::new(root.clone(), "private".into()),
        ports: Ports::bind_ports(key.to_string(), "invalid".into(), pryx_agent::contract::AGENT_PORT, Vec::new())
            .await
            .unwrap(),
    });
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    agent.logs.record("stdout", &STANDARD.encode(b"first\n")).unwrap();
    agent.logs.record("stderr", &STANDARD.encode(b"last\n")).unwrap();
    let agent_server =
        tokio::spawn(async move { axum::serve(listener, pryx_agent::server::files_router(agent)).await.unwrap() });
    let hold = Arc::new(AtomicBool::new(false));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let started = Arc::new(tokio::sync::Notify::new());
    let (holding, waiting, starting) = (hold.clone(), gate.clone(), started.clone());
    *f.fake.remote_files.lock().unwrap() = Some(Arc::new(move |request| {
        assert_eq!(request.sandbox, "files");
        assert_eq!(request.epoch, 7);
        let wait = holding.load(Ordering::SeqCst);
        let (waiting, starting) = (waiting.clone(), starting.clone());
        Service::new(
            endpoint,
            SigningKey::from_encoded(&encoded).unwrap(),
            (request.sandbox, request.epoch),
            Duration::from_secs(5),
            async move {
                if wait {
                    starting.notify_one();
                    waiting.acquire().await.unwrap().forget();
                }
                Ok(())
            },
        )
    }));
    let pki = pki(f.dir.path());
    let me = client(&pki.ca, CLIENT);
    let server = serve(f.manager.clone()).await;
    let path = "/v1/workloads/files/files/content?path=src/blob";
    let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(3 * 1024 * 1024).collect();
    let result =
        request(server.addr, &pki.ca.pem, Some(&me), "PUT", path, &[("blocklyd-epoch", "6")], Some(bytes.clone()))
            .await
            .unwrap();
    assert_eq!(result.status, hyper::StatusCode::CONFLICT);
    assert_eq!(f.fake.count_calls("files "), 0, "Epoch refusal must precede remote preparation");
    let result =
        request(server.addr, &pki.ca.pem, Some(&me), "PUT", path, &[("blocklyd-epoch", "7")], Some(bytes.clone()))
            .await
            .unwrap();
    assert_eq!(result.status, hyper::StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&result.body));
    let result =
        request(server.addr, &pki.ca.pem, Some(&me), "GET", path, &[("blocklyd-epoch", "7")], None).await.unwrap();
    assert_eq!(result.status, hyper::StatusCode::OK);
    assert_eq!(result.body, bytes);
    let result = request(
        server.addr,
        &pki.ca.pem,
        Some(&me),
        "GET",
        "/v1/workloads/files/files/list?path=src",
        &[("blocklyd-epoch", "7")],
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.status, hyper::StatusCode::OK);
    assert_eq!(result.json()["files"][0]["size"], bytes.len());
    let result = request(
        server.addr,
        &pki.ca.pem,
        Some(&me),
        "GET",
        "/v1/workloads/files/files/stat?path=missing",
        &[("blocklyd-epoch", "7")],
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.status, hyper::StatusCode::NOT_FOUND);
    assert_eq!(result.json()["error"]["code"], "file_not_found");
    std::os::unix::fs::symlink("/etc", root.join("outside")).unwrap();
    let result = request(
        server.addr,
        &pki.ca.pem,
        Some(&me),
        "GET",
        "/v1/workloads/files/files/content?path=outside/passwd",
        &[("blocklyd-epoch", "7")],
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.status, hyper::StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(f.fake.count_calls("start "), 0, "File access must not start workload compute");
    let logs = request(server.addr, &pki.ca.pem, Some(&me), "GET", "/v1/workloads/files/logs?tail=1", &[], None)
        .await
        .unwrap();
    assert_eq!(logs.status, hyper::StatusCode::OK);
    let records: Vec<serde_json::Value> = logs
        .body
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect();
    assert_eq!(records[0]["stream"], "stderr");
    assert_eq!(records[0]["line"], "last");
    assert_eq!(f.manager.capacity().running_memory_mb, 0);
    assert_eq!(f.fake.count_calls("start "), 0, "Stopped logs must not start workload compute");
    hold.store(true, Ordering::SeqCst);
    let addr = server.addr;
    let caller = tokio::spawn(async move {
        request(addr, &pki.ca.pem, Some(&me), "PUT", path, &[("blocklyd-epoch", "7")], Some(b"last".to_vec())).await
    });
    tokio::time::timeout(Duration::from_secs(5), started.notified()).await.unwrap();
    caller.abort();
    assert!(matches!(caller.await, Err(error) if error.is_cancelled()));
    assert_eq!(f.manager.capacity().running_memory_mb, memory_mb, "Cleanup must retain admitted memory");
    assert!(
        tokio::time::timeout(Duration::from_millis(50), f.manager.start(key.clone(), Some(7))).await.is_err(),
        "Cancelled cleanup must retain the workload guard"
    );
    gate.add_permits(1);
    support::eventually("file-session memory release", async || {
        let memory = f.manager.capacity().running_memory_mb;
        (memory == 0, memory.to_string())
    })
    .await;
    let logs = f.manager.logs(&key, Some(1), None, false).unwrap();
    tokio::time::timeout(Duration::from_secs(5), started.notified()).await.unwrap();
    drop(logs);
    assert_eq!(f.manager.capacity().running_memory_mb, memory_mb, "Cancelled log cleanup retains admitted memory");
    assert!(
        tokio::time::timeout(Duration::from_millis(50), f.manager.start(key.clone(), Some(7))).await.is_err(),
        "Cancelled log cleanup retains the workload guard"
    );
    gate.add_permits(1);
    support::eventually("log-session memory release", async || {
        let memory = f.manager.capacity().running_memory_mb;
        (memory == 0, memory.to_string())
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), f.manager.start(key, Some(7))).await.unwrap().unwrap();
    agent_server.abort();
}
