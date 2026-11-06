// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real agentOS sidecar acceptance without Docker or root. Stage the checksum-pinned distribution
//! first; absence is a setup failure, never a skipped or fake-runtime pass.

mod support;

use pryxd::config::WasmConfig;
use pryxd::manager::{Precondition, Records};
use pryxd::protocol::ExecRequest;
use pryxd::runtime::wasm::WasmRuntime;
use std::sync::Arc;

#[tokio::test]
async fn real_wasm_files_processes_sleep_stop_and_isolation() {
    let _ = tracing_subscriber::fmt().with_env_filter("pryxd::runtime::wasm=warn").try_init();
    let directory = tempfile::tempdir().unwrap();
    let began = std::time::Instant::now();
    let mut config = support::config(directory.path(), "runtime = 'wasm'");
    let assets = std::env::var_os("PRYX_AGENTOS_DISTRIBUTION")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| WasmConfig::default().distribution);
    config.wasm = WasmConfig {
        distribution: assets,
        launcher_executable: Some(env!("CARGO_BIN_EXE_pryxd").into()),
        ..Default::default()
    };
    std::fs::create_dir_all(&config.state_dir).unwrap();
    let runtime = Arc::new(WasmRuntime::new(&config.state_dir, &config.wasm).await.unwrap());
    eprintln!("real wasm host ready in {:?}", began.elapsed());
    let manager = support::manager_on(directory.path(), runtime.clone(), "runtime = 'wasm'");
    let reconciled = manager.reconcile(Records::FromDisk).await;
    assert!(reconciled.error.is_none(), "{reconciled:?}");
    let mut spec = support::spec_json();
    spec["image"] = "".into();
    spec["ports"] = serde_json::json!([{"name":"web","containerPort":3000,"audience":["edge"]}]);
    spec["entrypoint"] = serde_json::json!([
        "node",
        "-e",
        "require('node:http').createServer((req,res)=>res.end('declared')).listen(3000,'0.0.0.0')"
    ]);
    spec["resources"]["memoryMb"] = 256.into();
    spec["agent"] = serde_json::json!({"domain":"pryx.test", "keepAlive":false});
    spec["secrets"] = serde_json::json!({"APP_TOKEN":"wasm-private-fixture"});
    let spec: pryxd::protocol::WorkloadSpec = serde_json::from_value(spec).unwrap();
    manager.ensure(support::id("wasm-one"), spec.clone(), Precondition::None, None).await.unwrap();
    manager.start(support::id("wasm-one"), None).await.unwrap();
    eprintln!("real wasm workload started at {:?}", began.elapsed());
    declared_http(&manager).await;
    let stats = serde_json::to_value(manager.stats(&support::id("wasm-one")).await.unwrap()).unwrap();
    assert_eq!(stats["pids"], 1);
    let request =
        |script: &str| ExecRequest { command: vec!["sh".into(), "-c".into(), script.into()], timeout_seconds: 10 };
    let first = manager
        .exec(
            support::id("wasm-one"),
            request("printf hello; printf saved > /data/result; printf root-saved > /tmp/pryx-root-persisted"),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(first.exit_code, Some(0), "{first:?}");
    assert_eq!(first.stdout, "hello");
    let content = directory.path().join("state/workloads/wasm-one/data/content/result");
    assert_eq!(std::fs::read_to_string(content).unwrap(), "saved", "host data after command");
    let saved = manager.exec(support::id("wasm-one"), request("cat /data/result"), None, None).await.unwrap();
    assert_eq!(saved.stdout, "saved", "before sleep: {saved:?}");
    let secret =
        manager.exec(support::id("wasm-one"), request("env | grep PRYX_AGENT_SECRET_"), None, None).await.unwrap();
    assert_ne!(secret.exit_code, Some(0));
    let outside = directory.path().join("outside-marker");
    std::fs::write(&outside, "outside-only").unwrap();
    let refused = manager
        .exec(
            support::id("wasm-one"),
            ExecRequest { command: vec!["cat".into(), outside.to_string_lossy().into()], timeout_seconds: 10 },
            None,
            None,
        )
        .await
        .unwrap();
    assert_ne!(refused.exit_code, Some(0));
    manager.suspend(support::id("wasm-one"), None).await.unwrap();
    manager.start(support::id("wasm-one"), None).await.unwrap();
    let resumed = manager.exec(support::id("wasm-one"), request("cat /data/result"), None, None).await.unwrap();
    assert_eq!(resumed.exit_code, Some(0), "after wake: {resumed:?}");
    assert_eq!(resumed.stdout, "saved", "after sleep: {resumed:?}");
    let root_file =
        manager.exec(support::id("wasm-one"), request("cat /tmp/pryx-root-persisted"), None, None).await.unwrap();
    assert_eq!(root_file.stdout, "root-saved");
    manager.stop(support::id("wasm-one"), None, None).await.unwrap();
    manager.start(support::id("wasm-one"), None).await.unwrap();
    let restarted = manager.exec(support::id("wasm-one"), request("cat /data/result"), None, None).await.unwrap();
    assert_eq!(restarted.exit_code, Some(0));
    assert_eq!(restarted.stdout, "saved");
    compact(&manager).await;
    languages(&manager).await;
    let journal = std::fs::read_to_string(config.state_dir.join("wasm-runtime/workloads.json")).unwrap();
    assert!(!journal.contains("wasm-private-fixture"));
    let metadata: serde_json::Value = serde_json::from_str(&journal).unwrap();
    assert!(metadata.as_object().unwrap().values().all(|entry| entry["spec"]["env"].as_array().unwrap().is_empty()));
    restart(directory.path(), &config, runtime, manager, spec).await;
}

async fn restart(
    directory: &std::path::Path,
    config: &pryxd::config::Config,
    runtime: Arc<WasmRuntime>,
    manager: Arc<pryxd::manager::Manager>,
    spec: pryxd::protocol::WorkloadSpec,
) {
    runtime.shutdown().await.unwrap();
    drop(manager);
    drop(runtime);
    let recovered = Arc::new(WasmRuntime::new(&config.state_dir, &config.wasm).await.unwrap());
    let manager = support::manager_on(directory, recovered.clone(), "runtime = 'wasm'");
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    manager.ensure(support::id("wasm-one"), spec, Precondition::None, None).await.unwrap();
    manager.start(support::id("wasm-one"), None).await.unwrap();
    let restored = manager
        .exec(
            support::id("wasm-one"),
            ExecRequest {
                command: vec![
                    "sh".into(),
                    "-c".into(),
                    "cat /data/result; cat /tmp/pryx-root-persisted; printf %s \"$APP_TOKEN\"".into(),
                ],
                timeout_seconds: 10,
            },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(restored.stdout, "savedroot-savedwasm-private-fixture");
    recovered.shutdown().await.unwrap();
}

async fn languages(manager: &Arc<pryxd::manager::Manager>) {
    for (program, args) in [("node", vec!["-e", "console.log(21*2)"]), ("python3", vec!["-c", "print(21*2)"])] {
        let result = manager
            .exec(
                support::id("wasm-one"),
                ExecRequest {
                    command: std::iter::once(program).chain(args).map(String::from).collect(),
                    timeout_seconds: 30,
                },
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.exit_code, Some(0), "{program}: {result:?}");
        assert_eq!(result.stdout.trim(), "42");
    }
}

async fn compact(manager: &Arc<pryxd::manager::Manager>) {
    let mut spec = support::spec_json();
    spec["image"] = "".into();
    spec["ports"] = serde_json::json!([]);
    spec["resources"]["memoryMb"] = 64.into();
    spec["entrypoint"] = serde_json::json!(["sh", "-c", "sleep 60"]);
    spec["agent"] = serde_json::json!({"domain":"pryx.test", "keepAlive":true});
    manager
        .ensure(support::id("wasm-compact"), serde_json::from_value(spec).unwrap(), Precondition::None, None)
        .await
        .unwrap();
    manager.start(support::id("wasm-compact"), None).await.unwrap();
    let result = manager
        .exec(
            support::id("wasm-compact"),
            ExecRequest { command: vec!["sh".into(), "-c".into(), "printf compact".into()], timeout_seconds: 10 },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.stdout, "compact");
    let stats = serde_json::to_value(manager.stats(&support::id("wasm-compact")).await.unwrap()).unwrap();
    assert_eq!(stats["pids"], 0, "keepAlive must suppress the placeholder entrypoint and exclude exited commands");
    manager.stop(support::id("wasm-compact"), None, None).await.unwrap();
}

async fn declared_http(manager: &Arc<pryxd::manager::Manager>) {
    let port = manager.view(&support::id("wasm-one")).unwrap().ports[0].host_port;
    use http_body_util::BodyExt;
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if let Ok(socket) = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await {
                let (mut sender, connection) =
                    hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(socket)).await.unwrap();
                let connection = tokio::spawn(connection);
                let request = hyper::Request::builder()
                    .uri("/")
                    .header("Host", "localhost")
                    .header("Connection", "close")
                    .body(http_body_util::Empty::<bytes::Bytes>::new())
                    .unwrap();
                if let Ok(response) = sender.send_request(request).await {
                    let status = response.status();
                    let body = response.into_body().collect().await.unwrap().to_bytes();
                    if status == hyper::StatusCode::OK && body == "declared" {
                        connection.await.unwrap().unwrap();
                        break;
                    }
                }
                connection.abort();
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
