// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! The real assembled static VM bundle is verified before execution and after lazy HTTP
//! acquisition. Digest, missing/extra member, mode and link failures cannot publish compute.
use pryxd::runtime::vm::bundle::{Artifact, Bundle};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use tokio::io::AsyncReadExt;

fn input(name: &str) -> PathBuf {
    std::env::var_os(name).unwrap_or_else(|| panic!("Stage {name}")).into()
}

#[tokio::test]
#[ignore = "requires the actual assembled VM bundle; the optional VM workflow runs it"]
async fn actual_bundle_acquires_whole_and_refuses_changed_inputs() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let root = input("PRYX_VM_RUNTIME");
    let archive = input("PRYX_VM_RUNTIME_ARCHIVE");
    let mut artifact: Artifact =
        serde_json::from_slice(&fs::read(input("PRYX_VM_RUNTIME_DESCRIPTOR")).unwrap()).unwrap();
    let verified = Bundle::verified(&root, &artifact.manifest_sha256).unwrap();
    assert_eq!(verified.root(), root);
    let launcher = std::process::Command::new(root.join("bin/vm-launcher"))
        .arg(pryx_agent::bootstrap_capabilities::PROBE)
        .output()
        .unwrap();
    assert!(launcher.status.success());
    let capability: pryx_agent::bootstrap_capabilities::Capabilities =
        serde_json::from_slice(&launcher.stdout).unwrap();
    assert_eq!(capability.vm_monitor_version, verified.bootstrap().metadata.vm_monitor_version);
    assert_eq!(capability.vm_service_port_base, pryx_agent::vm_boot::SERVICE_PORT_BASE);
    assert!(!verified.filesystems().identity().is_empty());
    let directory = tempfile::tempdir().unwrap();
    let cache = directory.path().join("cache");
    let downloads = directory.path().join("downloads");
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    artifact.url = format!("http://{}/bundle", listener.local_addr().unwrap());
    let router = axum::Router::new().route(
        "/bundle",
        axum::routing::get(move || {
            let archive = archive.clone();
            async move {
                let file = tokio::fs::File::open(archive).await.unwrap();
                let stream = futures_util::stream::try_unfold(file, |mut file| async move {
                    let mut bytes = vec![0u8; 64 * 1024];
                    let size = file.read(&mut bytes).await?;
                    bytes.truncate(size);
                    Ok::<_, std::io::Error>((size != 0).then_some((bytes, file)))
                });
                axum::body::Body::from_stream(stream)
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut wrong = artifact.clone();
    wrong.sha256 = "0".repeat(artifact.sha256.len());
    assert!(Bundle::acquire(&cache, &downloads, &wrong).await.is_err());
    assert!(!cache.join(&wrong.sha256).exists());
    let acquired = Bundle::acquire(&cache, &downloads, &artifact).await.unwrap();
    assert!(Bundle::verified(acquired.root(), &artifact.manifest_sha256).is_ok());
    server.abort();
    // The published complete cache can be reused even while its source is unavailable.
    assert!(Bundle::acquire(&cache, &downloads, &artifact).await.is_ok());
    let changed = directory.path().join("changed");
    pryxd::tree::copy_tree(&root, &changed, None, &[], &mut |_| Ok(())).unwrap();
    assert!(Bundle::verified(&changed, &artifact.manifest_sha256).is_ok());
    let monitor = changed.join("bin/firecracker");
    let original = fs::read(&monitor).unwrap();
    fs::set_permissions(&monitor, fs::Permissions::from_mode(0o755)).unwrap();
    let mut bytes = original.clone();
    bytes[0] ^= 1;
    fs::write(&monitor, &bytes).unwrap();
    assert!(Bundle::verified(&changed, &artifact.manifest_sha256).is_err());
    fs::write(&monitor, original).unwrap();
    fs::set_permissions(&monitor, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(Bundle::verified(&changed, &artifact.manifest_sha256).is_err());
    fs::set_permissions(&monitor, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(changed.join("unexpected"), "not in the signed inventory").unwrap();
    assert!(Bundle::verified(&changed, &artifact.manifest_sha256).is_err());
    fs::remove_file(changed.join("unexpected")).unwrap();
    let kernel = changed.join("kernel/vmlinux");
    fs::remove_file(&kernel).unwrap();
    assert!(Bundle::verified(&changed, &artifact.manifest_sha256).is_err());
    std::os::unix::fs::symlink(root.join("kernel/vmlinux"), &kernel).unwrap();
    assert!(Bundle::verified(&changed, &artifact.manifest_sha256).is_err());
    println!("Actual bundle: whole acquisition, offline reuse and all corruption refusals passed");
}
