// SPDX-License-Identifier: FSL-1.1-ALv2
//! Actual VM acceptance through the manager and mutual-TLS node API. The existing VM CI
//! job supplies verified archives, KVM and real OCI images; backend installation and edge
//! routing remain separate acceptance paths. No fake runtime answers a workload request.
mod support;

use hyper::StatusCode;
use pryxd::manager::Records;
use pryxd::protocol::{CONFIRM_DELETE_HEADER, DataDisposition, EPOCH_HEADER, Guarantee};
use pryxd::runtime::vm::bundle::Artifact;
use serde_json::{Value, json};
use std::{fs, net::Ipv4Addr, os::unix::fs::PermissionsExt, path::PathBuf};

fn input(name: &str) -> PathBuf {
    std::env::var_os(name).unwrap_or_else(|| panic!("Stage {name}")).into()
}

async fn archive() -> (Artifact, tokio::task::JoinHandle<()>) {
    let mut artifact: Artifact =
        serde_json::from_slice(&fs::read(input("PRYX_VM_RUNTIME_DESCRIPTOR")).unwrap()).unwrap();
    let file = input("PRYX_VM_RUNTIME_ARCHIVE");
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    artifact.url = format!("http://{}/runtime", listener.local_addr().unwrap());
    let router = axum::Router::new().route(
        "/runtime",
        axum::routing::get(move || {
            let file = file.clone();
            async move {
                let file = tokio::fs::File::open(file).await.unwrap();
                axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file))
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (artifact, server)
}

struct Node {
    server: support::Server,
    pki: support::Pki,
    client: support::Identity,
}
impl Node {
    async fn bytes(&self, method: &str, path: &str, epoch: &str, body: Option<Vec<u8>>) -> support::Response {
        support::request(
            self.server.addr,
            &self.pki.ca.pem,
            Some(&self.client),
            method,
            path,
            &[(EPOCH_HEADER, epoch)],
            body,
        )
        .await
        .unwrap()
    }
    async fn call(&self, method: &str, path: &str, body: Option<Value>, expected: StatusCode) -> Value {
        let response = self.bytes(method, path, "1", body.map(|value| serde_json::to_vec(&value).unwrap())).await;
        assert_eq!(response.status, expected, "{method} {path}: {}", String::from_utf8_lossy(&response.body));
        response.json()
    }
    async fn exec(&self, path: &str, command: &[&str]) -> Value {
        self.call("POST", &format!("{path}/exec"), Some(json!({"command":command,"timeoutSeconds":10})), StatusCode::OK)
            .await
    }
    async fn content(&self, path: &str, expected: &[u8]) {
        let response = self.bytes("GET", &format!("{path}/files/content?path=retained"), "1", None).await;
        assert_eq!(response.status, StatusCode::OK, "{}", String::from_utf8_lossy(&response.body));
        assert_eq!(response.body, expected);
    }
}

#[tokio::test]
#[ignore = "requires actual KVM and the verified archive; the existing optional VM CI job runs it"]
async fn actual_vm_node_tls_admission_files_epochs_and_snapshot_restore() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let root = tempfile::tempdir().unwrap();
    let (artifact, distribution) = archive().await;
    let descriptor = root.path().join("vm-bundle.json");
    fs::write(&descriptor, serde_json::to_vec(&artifact).unwrap()).unwrap();
    fs::set_permissions(&descriptor, fs::Permissions::from_mode(0o444)).unwrap();
    let agent = input("PRYX_VM_HOST_LAUNCHER");
    let uid = rustix::process::geteuid().as_raw().checked_add(1).unwrap();
    let extra = format!(
        "runtime = 'vm'\n[vm]\nbundle = '{}'\n[workloads]\nagent_binary = '{}'\nuser = '{uid}:{uid}'\nallowed_images = ['docker.io/library/alpine@', 'docker.io/library/debian@']\ndefault_spare_ports = 0\n",
        descriptor.display(),
        agent.display()
    );
    let config = support::config(root.path(), &extra);
    fs::create_dir_all(&config.state_dir).unwrap();
    let runtime = pryxd::runtime::configured::create(&config).await.unwrap();
    assert_eq!(runtime.isolation(), "vm");
    automatic(&config).await;
    let manager = support::manager_probed(
        root.path(),
        runtime,
        &extra,
        pryxd::ports::bind_probe(vec![Ipv4Addr::LOCALHOST.into()]),
    );
    let report = manager.reconcile(Records::FromDisk).await;
    assert!(report.error.is_none(), "{report:?}");
    let pki = support::pki(root.path());
    let client = support::client(&pki.ca, support::CLIENT);
    let node = Node { server: support::serve(manager.clone()).await, pki, client };
    let health = node.call("GET", "/v1/health", None, StatusCode::OK).await;
    assert_eq!(health["status"], "ok");
    assert!(health["features"].as_array().unwrap().contains(&json!("tier-vm")));
    for guarantee in [Guarantee::OwnKernel, Guarantee::MemoryLimit, Guarantee::CpuLimit] {
        assert!(health["features"].as_array().unwrap().contains(&serde_json::to_value(guarantee).unwrap()));
    }
    // Memory sleep is announced only once the tier's start test, run at construction, has passed.
    let memory_sleep = serde_json::to_value(Guarantee::MemorySleep).unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
    while !node.call("GET", "/v1/health", None, StatusCode::OK).await["features"]
        .as_array()
        .unwrap()
        .contains(&memory_sleep)
    {
        assert!(tokio::time::Instant::now() < deadline, "The VM tier never announced memory sleep");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let images: Vec<Value> = serde_json::from_str(include_str!("../templates/vm-tools/image-inputs.json")).unwrap();
    for (index, image) in images.iter().enumerate() {
        lifecycle(&node, index, image["reference"].as_str().unwrap()).await;
    }
    node.server.cancel.cancel();
    node.server.task.await.unwrap();
    drop(manager);
    distribution.abort();
}

async fn automatic(config: &pryxd::config::Config) {
    let mut automatic = config.clone();
    automatic.runtime = pryxd::config::RuntimeKind::Auto;
    automatic.state_dir = config.state_dir.join("automatic");
    fs::create_dir_all(&automatic.state_dir).unwrap();
    automatic.docker.enabled = false;
    automatic.wasm.distribution = automatic.state_dir.join("absent-wasm");
    let runtime = pryxd::runtime::configured::create(&automatic).await.unwrap();
    assert!(runtime.multi_tier());
    assert!(runtime.backend("vm").is_some());
    assert_eq!(runtime.isolation(), "process", "The existing default remains first");
}

async fn lifecycle(node: &Node, index: usize, image: &str) {
    let path = format!("/v1/workloads/vm-api-{index}");
    let mut spec = support::spec_json();
    spec["isolation"] = "vm".into();
    spec["image"] = image.into();
    spec["ports"] = json!([]);
    spec["resources"] = json!({"memoryMb":256,"cpuMillis":1000,"pidsLimit":128});
    spec["network"] = "none".into();
    spec["agent"] = json!({"domain":"vm.pryx.invalid","keepAlive":true});
    let secret = uuid::Uuid::new_v4().simple().to_string();
    spec["secrets"] = json!({"APP_TOKEN":secret});
    let created = node.call("PUT", &path, Some(spec.clone()), StatusCode::CREATED).await;
    assert_eq!(created["workload"]["isolation"], "vm");
    assert!(!created.to_string().contains(&secret));
    let duplicate = node.call("PUT", &path, Some(spec), StatusCode::OK).await;
    assert_eq!(duplicate["outcome"], "unchanged");
    let old = node.bytes("POST", &format!("{path}/start"), "0", None).await;
    assert_eq!(old.status, StatusCode::CONFLICT);
    node.call("POST", &format!("{path}/start"), None, StatusCode::OK).await;
    let result = node.exec(&path, &["/bin/sh", "-c", "printf retained > /data/retained; printf api-vm"]).await;
    assert_eq!(result["exitCode"], 0);
    assert_eq!(result["stdout"], "api-vm");
    node.content(&path, b"retained").await;
    // A suspend keeps the guest's memory, here a file on its tmpfs, and a start resumes it.
    assert_eq!(node.exec(&path, &["/bin/sh", "-c", "printf kept > /tmp/marker"]).await["exitCode"], 0);
    let suspended = node.call("POST", &format!("{path}/suspend"), None, StatusCode::OK).await;
    assert_eq!(suspended["workload"]["state"], "suspended");
    let woken = node.call("POST", &format!("{path}/start"), None, StatusCode::OK).await;
    assert_eq!(woken["resumed"], true, "{woken}");
    assert_eq!(node.exec(&path, &["/bin/cat", "/tmp/marker"]).await["stdout"], "kept");
    node.call("POST", &format!("{path}/stop"), Some(json!({})), StatusCode::OK).await;
    node.content(&path, b"retained").await;
    snapshot_restore(node, &path).await;
    node.call("POST", &format!("{path}/start"), None, StatusCode::OK).await;
    node.content(&path, b"retained").await;
    node.call("POST", &format!("{path}/stop"), Some(json!({})), StatusCode::OK).await;
    let keep = serde_json::to_value(DataDisposition::Keep).unwrap();
    node.call("DELETE", &format!("{path}?data={}", keep.as_str().unwrap()), None, StatusCode::OK).await;
    node.content(&path, b"retained").await;
    let deleted = support::request(
        node.server.addr,
        &node.pki.ca.pem,
        Some(&node.client),
        "DELETE",
        &format!("{path}?data=delete"),
        &[(EPOCH_HEADER, "1"), (CONFIRM_DELETE_HEADER, path.rsplit('/').next().unwrap())],
        None,
    )
    .await
    .unwrap();
    assert_eq!(deleted.status, StatusCode::OK, "{}", String::from_utf8_lossy(&deleted.body));
    node.call("GET", &path, None, StatusCode::NOT_FOUND).await;
}

async fn snapshot_restore(node: &Node, path: &str) {
    let snapshot =
        node.call("POST", &format!("{path}/snapshots"), Some(json!({"id":"one"})), StatusCode::CREATED).await;
    assert_eq!(snapshot["snapshot"]["id"], "one");
    let response =
        node.bytes("PUT", &format!("{path}/files/content?path=retained"), "1", Some(b"changed".to_vec())).await;
    assert_eq!(response.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&response.body));
    node.content(path, b"changed").await;
    node.call("POST", &format!("{path}/restore"), Some(json!({"snapshot":"one"})), StatusCode::OK).await;
    node.content(path, b"retained").await;
}
