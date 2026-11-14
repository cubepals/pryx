// SPDX-License-Identifier: FSL-1.1-ALv2
//! Additional actual KVM boots exercise production DHCP, packet transport and immutable
//! hostname policy as an ordinary workload on both images. Disconnected lifecycle cases in
//! tests.rs remain intact. The public CA bundle is copied unchanged; TLS verification stays on.
use super::*;
use crate::runtime::{ContainerRuntime, ContainerStatus};
use serde_json::json;
use std::net::Ipv4Addr;

pub(super) async fn check(runtime: &VmRuntime, root: &Path, image: &str, index: usize, unrestricted: bool) {
    eprintln!("VM runtime image {index}: production policy, unrestricted={unrestricted}");
    let data = root.join(format!("network-data{index}-{unrestricted}"));
    directory(&data).unwrap();
    let host = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut spec = tests::spec(&data, image);
    spec.internal_network = false;
    spec.egress =
        (!unrestricted).then(|| crate::protocol::EgressPolicy { allow: vec!["example.com".into()], deny: vec![] });
    let id = runtime.create(&spec).await.unwrap();
    runtime.start(&id).await.unwrap();
    assert_eq!(runtime.inspect(&id).await.unwrap().unwrap().status, ContainerStatus::Running);
    let fixture = {
        let live = runtime.state.live.lock().unwrap()[&id].clone();
        let live = live.lock().await;
        let profile = live.network.as_ref().unwrap().profile();
        json!({
            "unrestricted":unrestricted, "hostname":"example.com", "host":profile["host"], "http":profile["http"],
            "dns":profile["dns"], "privatePort":host.local_addr().unwrap().port(), "caFile":"/data/host-ca.pem"
        })
    };
    upload_probe(runtime, &id, &fixture).await;
    let chmod = runtime
        .exec(&id, &["/bin/chmod".into(), "500".into(), "/data/network-probe".into()], Duration::from_secs(5), 4096)
        .await
        .unwrap();
    assert_eq!(chmod.exit_code, Some(0));
    let result = runtime
        .exec(
            &id,
            &["/data/network-probe".into(), "--production-policy".into(), "/data/network-fixture.json".into()],
            Duration::from_secs(15),
            pryx_agent::contract::CAPTURE_BYTES,
        )
        .await
        .unwrap();
    assert_eq!(result.exit_code, Some(0), "Production network probe: {}", String::from_utf8_lossy(&result.stderr));
    assert!(!result.timed_out && !result.killed);
    let checks: BTreeMap<String, bool> = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(checks.len(), if unrestricted { 7 } else { 9 });
    assert!(checks.values().all(|passed| *passed));
    assert!(tokio::time::timeout(Duration::from_millis(10), host.accept()).await.is_err());
    runtime.stop(&id, &spec.stop_signal, Duration::from_secs(15)).await.unwrap();
    assert_eq!(runtime.inspect(&id).await.unwrap().unwrap().status, ContainerStatus::Exited);
    runtime.remove(&id).await.unwrap();
}

async fn upload_probe(runtime: &VmRuntime, id: &str, fixture: &serde_json::Value) {
    let files = runtime.open_files(id).await.unwrap();
    for (path, bytes) in [
        ("network-probe", fs::read(tests::input("PRYX_VM_POLICY_PROBE")).unwrap()),
        ("network-fixture.json", serde_json::to_vec(&fixture).unwrap()),
        ("host-ca.pem", fs::read(crate::http_client::public_bundle().unwrap()).unwrap()),
    ] {
        let response = files
            .request(hyper::Method::PUT, &format!("/_pryx/files/content?path={path}"), crate::http_client::full(bytes))
            .await
            .unwrap();
        assert_eq!(response.status(), hyper::StatusCode::NO_CONTENT);
    }
    files.close().await.unwrap();
}
