// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Actual root runtime lifecycle, stopped-file maintenance and conservative daemon-loss
//! recovery. The existing optional VM job supplies root, KVM and verified OCI inputs;
//! hostname and public TAP packets run after each disconnected lifecycle; installed worker
//! authority remains a separate prerequisite.
use super::*;
use crate::runtime::{ContainerRuntime, ContainerStatus};
use http_body_util::BodyExt as _;
use hyper::Method;

async fn cold_boot(runtime: &VmRuntime, spec: &ContainerSpec, id: &str, host: &Path, retained: bool) {
    runtime.start(id).await.unwrap();
    assert_eq!(runtime.inspect(id).await.unwrap().unwrap().status, ContainerStatus::Running);
    root_acceptance::probe(spec, host, retained).await.unwrap();
    let inode = fs::metadata(spec.data_dir.join(super::super::disk::FILE)).unwrap().ino();
    {
        let live = runtime.state.live.lock().unwrap()[id].clone();
        let mut live = live.lock().await;
        let pid = live.monitor.pid().unwrap();
        // The VM's own pair from the host's range, which its jail record holds too.
        let owner = runtime.monitor_owner(id).unwrap().unwrap();
        let status = procfs::process::Process::new(pid.try_into().unwrap()).unwrap().status().unwrap();
        assert_eq!((status.euid, status.egid), owner);
        let observed = fs::metadata(format!("/proc/{pid}/root")).unwrap();
        let jail = live.jail.as_mut().unwrap();
        let expected = fs::metadata(jail.root()).unwrap();
        assert_eq!((observed.dev(), observed.ino()), (expected.dev(), expected.ino()));
        assert_ne!(fs::read_link(format!("/proc/{pid}/ns/mnt")).unwrap(), fs::read_link("/proc/self/ns/mnt").unwrap());
        assert!(jail.retire().is_err());
        assert!(lock(&runtime.lease(&runtime.entry(id).unwrap()).unwrap()).is_err());
    }
    runtime.stop(id, &spec.stop_signal, Duration::from_secs(15)).await.unwrap();
    assert_eq!(runtime.inspect(id).await.unwrap().unwrap().status, ContainerStatus::Exited);
    let disk = spec.data_dir.join(super::super::disk::FILE);
    assert_eq!(fs::metadata(&disk).unwrap().uid(), rustix::process::Uid::ROOT.as_raw());
    assert_eq!(fs::metadata(&disk).unwrap().ino(), inode);
    let files = runtime.open_files(id).await.unwrap();
    let response =
        files.request(Method::GET, "/_pryx/files/content?path=retained", crate::http_client::empty()).await.unwrap();
    assert_eq!(response.status(), hyper::StatusCode::OK);
    assert_eq!(response.into_body().collect().await.unwrap().to_bytes(), "root-jailer");
    files.close().await.unwrap();
    assert_eq!(fs::metadata(&disk).unwrap().uid(), rustix::process::Uid::ROOT.as_raw());
    assert_eq!(fs::metadata(&disk).unwrap().ino(), inode);
}

#[tokio::test]
#[ignore = "requires real root, KVM and the verified bundle; existing optional VM CI runs it"]
async fn actual_root_runtime_cold_boot_files_and_daemon_loss() {
    assert!(rustix::process::geteuid().is_root());
    let _ = rustls::crypto::ring::default_provider().install_default();
    let root = tempfile::tempdir().unwrap();
    let host = root.path().join("host-only");
    crate::durable::write_atomic(&host, uuid::Uuid::new_v4().as_bytes(), 0o600).unwrap();
    let (artifact, server) = tests::artifact_server().await;
    let settings = Settings {
        artifact,
        ingress_connections: 8,
        free_floor_bytes: 0,
        image_timeout: Duration::from_secs(120),
        // The host's default range: the runner user's own IDs run processes, so no VM takes them.
        monitor_owner: Some({
            let config = crate::config::VmConfig::default();
            MonitorIds { uid: config.monitor_uid, gid: config.monitor_gid, count: config.monitor_ids }
        }),
    };
    let ids = settings.monitor_owner.unwrap();
    let mut removed = None;
    let mut runtime = VmRuntime::new(root.path(), settings.clone()).await.unwrap();
    let inputs: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../../templates/vm-tools/image-inputs.json")).unwrap();
    for (index, input) in inputs.iter().enumerate() {
        let data = root.path().join(format!("data{index}"));
        directory(&data).unwrap();
        let spec = tests::spec(&data, input["reference"].as_str().unwrap());
        let id = runtime.create(&spec).await.unwrap();
        let owner = runtime.monitor_owner(&id).unwrap().unwrap();
        assert_eq!(owner.0 - ids.uid, owner.1 - ids.gid);
        // The pair a removed VM let go is the lowest free one again.
        assert_eq!(removed.unwrap_or((ids.uid, ids.gid)), owner);
        for generation in 0..2 {
            eprintln!("Actual root runtime image {index}, generation {generation}");
            cold_boot(&runtime, &spec, &id, &host, generation != 0).await;
        }
        runtime.start(&id).await.unwrap();
        drop(runtime);
        runtime = VmRuntime::new(root.path(), settings.clone()).await.unwrap();
        assert_eq!(runtime.monitor_owner(&id).unwrap(), Some(owner), "Recovery kept the VM's pair");
        assert_eq!(runtime.inspect(&id).await.unwrap().unwrap().status, ContainerStatus::Dead);
        assert_eq!(
            fs::metadata(data.join(super::super::disk::FILE)).unwrap().uid(),
            rustix::process::Uid::ROOT.as_raw()
        );
        cold_boot(&runtime, &spec, &id, &host, true).await;
        runtime.remove(&id).await.unwrap();
        assert!(runtime.inspect(&id).await.unwrap().is_none());
        removed = Some(owner);
        super::network_acceptance::check(&runtime, root.path(), input["reference"].as_str().unwrap(), index, false)
            .await;
        super::network_acceptance::check(&runtime, root.path(), input["reference"].as_str().unwrap(), index, true)
            .await;
    }
    server.abort();
}
