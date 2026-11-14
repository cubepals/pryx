// SPDX-License-Identifier: FSL-1.1-ALv2
//! A real two-vCPU guest must complete its signed disk stop and retire its monitor without
//! a VMM panic. The existing optional hardware job retains only signal syscalls for diagnosis.
use super::*;
use crate::runtime::ContainerRuntime;

#[tokio::test]
#[ignore = "requires real KVM and the verified archive; optional VM CI traces this one guest"]
async fn actual_monitor_shutdown_threads_exit_cleanly() {
    assert!(!rustix::process::geteuid().is_root());
    let _ = rustls::crypto::ring::default_provider().install_default();
    let root = tempfile::tempdir().unwrap();
    let (artifact, server) = tests::artifact_server().await;
    let runtime = VmRuntime::new(
        root.path(),
        Settings {
            monitor_owner: None,
            artifact,
            ingress_connections: 8,
            free_floor_bytes: 0,
            image_timeout: Duration::from_secs(120),
        },
    )
    .await
    .unwrap();
    let inputs: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../../templates/vm-tools/image-inputs.json")).unwrap();
    let data = root.path().join("data");
    directory(&data).unwrap();
    let spec = tests::spec(&data, inputs[0]["reference"].as_str().unwrap());
    let id = runtime.create(&spec).await.unwrap();
    runtime.start(&id).await.unwrap();
    let live = runtime.state.live.lock().unwrap()[&id].clone();
    runtime.stop(&id, &spec.stop_signal, Duration::from_secs(15)).await.unwrap();
    assert_eq!(runtime.inspect(&id).await.unwrap().unwrap().exit_code, i64::from(libc::EXIT_SUCCESS));
    let mut live = live.lock().await;
    let status = live.monitor.exited().unwrap().unwrap();
    assert!(status.success(), "Monitor failed after a verified orderly guest stop: {status:?}");
    drop(live);
    runtime.remove(&id).await.unwrap();
    server.abort();
}
