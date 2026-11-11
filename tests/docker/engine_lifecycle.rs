// SPDX-License-Identifier: FSL-1.1-ALv2
//! Image-backed lifecycle and adversarial checks on the selected real engine. Archive transport
//! uses a bounded loopback HTTP peer; no runtime, container, filesystem or network is simulated.
use super::*;
use pryxd::protocol::{ExportRequest, Guarantee, RestoreRequest, SnapshotRequest};

struct EngineCase {
    manager: Arc<Manager>,
    socket: std::path::PathBuf,
    _cleanup: engine::EngineCleanup,
    _directory: tempfile::TempDir,
}

impl EngineCase {
    async fn new() -> Option<Self> {
        let socket = engine::selected_socket().await;
        let runtime = Arc::new(DockerRuntime::new(&socket).unwrap());
        let limits = engine::limits_or_mapping_refusal(&runtime, &socket).await?;
        let directory = tempfile::tempdir().unwrap();
        let deployment = deployment();
        let manager = manager_on(directory.path(), runtime, &engine::configuration(&deployment, limits.rootless));
        let report = manager.reconcile(Records::FromDisk).await;
        assert!(report.error.is_none(), "{report:?}");
        Some(Self {
            manager,
            socket: socket.clone(),
            _cleanup: engine::EngineCleanup { socket, deployment },
            _directory: directory,
        })
    }

    async fn create(&self, name: &str, internal: bool) {
        let mut value = support::spec_json();
        value["resources"] = serde_json::json!({"memoryMb": 256, "allowUnenforcedMemory": true});
        value["ports"].as_array_mut().unwrap().truncate(1);
        if internal {
            value["network"] = "none".into();
        }
        let status = axum::http::StatusCode::OK;
        let response = format!(
            r#"while IFS= read -r line && [ "$line" != "$(printf "\r")" ]; do :; done; printf "{:?} {} {}\r\nConnection: close\r\nContent-Length: %s\r\n\r\n" "$(wc -c </data/index.html)"; cat /data/index.html"#,
            axum::http::Version::HTTP_10,
            status.as_u16(),
            status.canonical_reason().unwrap()
        );
        value["entrypoint"] = serde_json::json!([
            "sh",
            "-c",
            format!("exec nc -lk -p {} -e sh -c '{response}'", pryxd::examples::HTTP_PORT)
        ]);
        let made = self
            .manager
            .ensure(id(name), serde_json::from_value(value).unwrap(), Precondition::None, None)
            .await
            .unwrap();
        assert_eq!(made.outcome, EnsureOutcome::Created);
    }

    async fn command(&self, name: &str, script: &str) -> String {
        let result = self
            .manager
            .exec(
                id(name),
                ExecRequest { command: vec!["sh".into(), "-c".into(), script.into()], timeout_seconds: 10 },
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.exit_code, Some(0), "{result:?}");
        assert!(!result.timed_out);
        result.stdout
    }

    async fn measurements(&self, output: &std::path::Path) {
        let mut report = serde_json::json!({
            "status": "running", "requestedSamples": 30, "samples": [],
            "scope": "direct node manager, locally available image, serial workloads, no public edge",
            "engine": std::env::var("PRYX_TEST_ENGINE").unwrap_or_else(|_| "docker".into()),
            "architecture": std::env::consts::ARCH,
        });
        std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        for index in 0..30 {
            let name = format!("measure-{index}");
            let began = Instant::now();
            self.create(&name, false).await;
            self.manager.start(id(&name), None).await.unwrap();
            let create = began.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(self.command(&name, "printf measured > /data/index.html").await, "");
            let began = Instant::now();
            assert_eq!(self.command(&name, "true").await, "");
            let exec = began.elapsed().as_secs_f64() * 1000.0;
            let began = Instant::now();
            assert_eq!(self.page(&name).await, "measured");
            let network = began.elapsed().as_secs_f64() * 1000.0;
            report["samples"].as_array_mut().unwrap().push(serde_json::json!({
                "createMs": create, "execMs": exec, "networkMs": network,
            }));
            std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
            self.manager.delete(id(&name), DataDisposition::Delete, Some(name.clone()), None).await.unwrap();
        }
        report["status"] = "complete".into();
        std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }

    async fn page(&self, name: &str) -> String {
        let port = self.manager.view(&id(name)).unwrap().ports[0].host_port;
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
        client
            .get(format!("http://127.0.0.1:{port}/"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap()
    }
}

#[tokio::test]
#[ignore = "needs the actual selected engine"]
async fn engine_images_ports_snapshot_fork_and_stop_keep_their_boundaries() {
    let output = std::env::var_os("PRYX_ENGINE_MEASUREMENTS").map(std::path::PathBuf::from);
    let Some(case) = EngineCase::new().await else {
        if let Some(output) = output {
            let report = serde_json::json!({"status": "refused", "requirement": "data-owner-mapping", "samples": []});
            std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
        return;
    };
    if let Some(output) = output {
        case.measurements(&output).await;
    }
    case.create("original", false).await;
    case.manager.start(id("original"), None).await.unwrap();
    assert_eq!(case.command("original", "printf original > /data/index.html; cat /data/index.html").await, "original");
    assert_eq!(case.page("original").await, "original");
    let result = case
        .command("original", "if touch /etc/pryx-escape 2>/dev/null; then echo writable; else echo readonly; fi; id -u")
        .await;
    assert_eq!(result, "readonly\n1000\n");
    case.manager.stop(id("original"), None, None).await.unwrap();
    snapshot_and_fork(&case).await;
    case.manager.start(id("original"), None).await.unwrap();
    assert_eq!(case.page("original").await, "original");
    assert_eq!(case.page("fork").await, "fork");
    attack_neighbour(&case).await;
    let view = case.manager.view(&id("original")).unwrap();
    if std::env::var("PRYX_TEST_ENGINE").is_ok() {
        assert!(!view.guarantees[&Guarantee::MemorySleep]);
        assert!(!view.guarantees[&Guarantee::HostFirewall]);
        let error = case.manager.suspend(id("original"), None).await.unwrap_err();
        assert!(matches!(error, NodeError::Conflict { code: ConflictCode::MemorySleepUnavailable, .. }));
    }
    case.manager.stop(id("original"), None, None).await.unwrap();
    case.manager.start(id("original"), None).await.unwrap();
    assert_eq!(case.command("original", "cat /data/index.html").await, "original");
    assert_eq!(case.page("original").await, "original");
    for name in ["original", "fork"] {
        case.manager.delete(id(name), DataDisposition::Delete, Some(name.into()), None).await.unwrap();
    }
}

async fn snapshot_and_fork(case: &EngineCase) {
    let snapshot: SnapshotRequest = serde_json::from_value(serde_json::json!({"id": "baseline"})).unwrap();
    let taken = case.manager.snapshot(id("original"), snapshot, None).await.unwrap();
    assert!(taken.created && taken.snapshot.files > 0);
    let object = Arc::new(tokio::sync::RwLock::new(bytes::Bytes::new()));
    let read = object.clone();
    let write = object.clone();
    let app = axum::Router::new().route(
        "/archive",
        axum::routing::get(move || {
            let read = read.clone();
            async move { read.read().await.clone() }
        })
        .put(move |body: bytes::Bytes| {
            let write = write.clone();
            async move {
                *write.write().await = body;
                axum::http::StatusCode::OK
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/archive", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let export: ExportRequest = serde_json::from_value(serde_json::json!({"url": url})).unwrap();
    let archive = case.manager.export(id("original"), export, None).await.unwrap();
    assert!(archive.size_bytes > 0);
    case.create("fork", false).await;
    let restore: RestoreRequest =
        serde_json::from_value(serde_json::json!({"url": url, "sha256": archive.sha256})).unwrap();
    case.manager.restore(id("fork"), restore, None).await.unwrap();
    server.abort();
    case.manager.start(id("fork"), None).await.unwrap();
    assert_eq!(case.command("fork", "cat /data/index.html; printf fork > /data/index.html").await, "original");
    let restore: RestoreRequest = serde_json::from_value(serde_json::json!({"snapshot": "baseline"})).unwrap();
    case.manager.restore(id("original"), restore, None).await.unwrap();
}

async fn attack_neighbour(case: &EngineCase) {
    let original_pid = case.command("original", "readlink /proc/1/ns/pid").await;
    let fork_pid = case.command("fork", "readlink /proc/1/ns/pid").await;
    assert!(!original_pid.trim().is_empty() && !fork_pid.trim().is_empty());
    assert_ne!(original_pid, fork_pid, "a neighbour must have its own PID namespace");
    let backend =
        bollard::Docker::connect_with_unix(case.socket.to_str().unwrap(), 60, bollard::API_DEFAULT_VERSION).unwrap();
    let view = case.manager.view(&id("original")).unwrap();
    let target = backend.inspect_container(&view.locate.container_name, None).await.unwrap();
    let networks = target.network_settings.unwrap().networks.unwrap();
    let address = networks
        .values()
        .find_map(|network| network.ip_address.as_ref())
        .unwrap()
        .parse::<std::net::Ipv4Addr>()
        .unwrap();
    let script = format!(
        "if wget -q -T 2 -O - http://{address}:{}/; then echo neighbour-reached; fi; true",
        pryxd::examples::HTTP_PORT
    );
    let output = case.command("fork", &script).await;
    assert!(
        !output.contains("original") && !output.contains("neighbour-reached"),
        "a neighbour reached the private bridge: {output}"
    );
    assert_eq!(case.page("original").await, "original", "the target remains reachable from its published port");
    assert!(view.guarantees[&Guarantee::PrivateNetwork]);
    assert!(view.guarantees[&Guarantee::PrivatePids]);
}

#[tokio::test]
#[ignore = "needs the actual selected engine"]
async fn engine_exec_timeout_retires_children_and_keeps_the_workload_running() {
    let Some(case) = EngineCase::new().await else { return };
    case.create("timeout", false).await;
    case.manager.start(id("timeout"), None).await.unwrap();
    let before = case.manager.view(&id("timeout")).unwrap();
    let backend =
        bollard::Docker::connect_with_unix(case.socket.to_str().unwrap(), 60, bollard::API_DEFAULT_VERSION).unwrap();
    let original = backend.inspect_container(&before.locate.container_name, None).await.unwrap().state.unwrap();
    let runtime = DockerRuntime::new(&case.socket).unwrap();
    runtime.disable_restart(before.locate.container_id.as_deref().unwrap()).await.unwrap();
    let policy = backend.inspect_container(&before.locate.container_name, None).await.unwrap().host_config.unwrap();
    assert_eq!(policy.restart_policy.unwrap().name, Some(bollard::models::RestartPolicyNameEnum::NO));
    for script in ["sleep 30; echo never", "exec 1>&- 2>&-; sleep 30; echo never"] {
        let began = Instant::now();
        let result = case
            .manager
            .exec(
                id("timeout"),
                ExecRequest { command: vec!["sh".into(), "-c".into(), script.into()], timeout_seconds: 1 },
                None,
                None,
            )
            .await
            .unwrap();
        assert!(result.timed_out && result.killed, "{result:?}");
        assert!(began.elapsed() < Duration::from_secs(6));
    }
    assert_eq!(case.command("timeout", "ps -o args | grep -c '^sleep 30' || true").await.trim(), "0");
    let actual = backend.inspect_container(&before.locate.container_name, None).await.unwrap().state.unwrap();
    assert_eq!(actual.running, Some(true));
    assert_eq!(actual.started_at, original.started_at, "termination never restarts the workload");
    let after = case.manager.view(&id("timeout")).unwrap();
    assert_eq!(after.locate.container_id, before.locate.container_id);
    assert_eq!(after.started_at, before.started_at);
    case.manager.delete(id("timeout"), DataDisposition::Delete, Some("timeout".into()), None).await.unwrap();
}

#[tokio::test]
#[ignore = "needs the actual selected engine"]
async fn engine_internal_network_preserves_ingress_and_refuses_external_egress() {
    use tokio::io::AsyncWriteExt;
    let Some(case) = EngineCase::new().await else { return };
    if !case.manager.features().iter().any(|feature| feature == "network-policy") {
        let mut value = support::spec_json();
        value["resources"] = serde_json::json!({"memoryMb": 256, "allowUnenforcedMemory": true});
        value["network"] = "none".into();
        let error = case
            .manager
            .ensure(id("internal"), serde_json::from_value(value).unwrap(), Precondition::None, None)
            .await
            .unwrap_err();
        let NodeError::Invalid(fields) = error else { panic!("unexpected refusal: {error:?}") };
        assert!(serde_json::to_string(&fields).unwrap().contains("network-policy"));
        assert!(!case.manager.store.data_dir(&id("internal")).exists());
        return;
    }
    let host = host_address();
    let listener = tokio::net::TcpListener::bind((host, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let _ = stream.write_all(b"canary").await;
        }
    });
    case.create("internet", false).await;
    case.manager.start(id("internet"), None).await.unwrap();
    let dial = format!("nc -w 1 {host} {port}");
    assert_eq!(case.command("internet", &dial).await, "canary", "the controlled target is reachable before policy");
    case.create("internal", true).await;
    case.manager.start(id("internal"), None).await.unwrap();
    case.command("internal", "printf internal > /data/index.html").await;
    assert_eq!(case.page("internal").await, "internal", "private ingress remains reachable");
    assert_eq!(case.command("internal", "ip route | grep -c '^default' || true").await.trim(), "0");
    case.command("internal", &format!("if {dial}; then exit 1; fi")).await;
    server.abort();
    for name in ["internal", "internet"] {
        case.manager.delete(id(name), DataDisposition::Delete, Some(name.into()), None).await.unwrap();
    }
}
