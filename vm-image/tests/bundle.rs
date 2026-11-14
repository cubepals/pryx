// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! The real assembled static VM bundle is verified before execution and after lazy HTTP
//! acquisition. Digest, missing/extra member, mode and link failures cannot publish compute.
use pryxd::runtime::{
    RuntimeError,
    vm::{
        bundle::{Artifact, Bundle},
        disk,
        filesystems::sha256,
    },
};
use std::{
    fs,
    io::Read,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    time::Duration,
};
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
    assert!(capability.vm_shutdown_signals && verified.bootstrap().metadata.vm_shutdown_signals);
    assert_eq!(capability.vm_service_port_base, pryx_agent::vm_boot::SERVICE_PORT_BASE);
    assert!(!verified.filesystems().identity().is_empty());
    forwarding(&root).await;
    writable_disk(verified.filesystems()).await;
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

async fn writable_disk(tools: &pryxd::runtime::vm::filesystems::FilesystemTools) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("data");
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let lease = temporary.path().join("monitor.lock");
    let size = 64 * 1024 * 1024;
    let timeout = Duration::from_secs(15);
    let prepared = disk::prepare(tools, &root, &lease, size, timeout, 0).await.unwrap();
    assert!(prepared.created && !prepared.recovered);
    assert_eq!(prepared.path, root.join(disk::FILE));
    assert_eq!(fs::metadata(&prepared.path).unwrap().len(), size);
    let before = fs::metadata(&prepared.path).unwrap().ino();
    let original = sha256(fs::File::open(&prepared.path).unwrap()).unwrap();
    let again = disk::prepare(tools, &root, &lease, size, timeout, 0).await.unwrap();
    assert!(!again.created && !again.recovered);
    assert_eq!(fs::metadata(&again.path).unwrap().ino(), before);
    assert_eq!(sha256(fs::File::open(&again.path).unwrap()).unwrap(), original);
    let lock = fs::OpenOptions::new().read(true).write(true).open(&lease).unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    assert!(matches!(disk::prepare(tools, &root, &lease, size, timeout, 0).await, Err(RuntimeError::Conflict(_))));
    drop(lock);
    assert!(disk::prepare(tools, &root, &lease, size + 1, timeout, 0).await.is_err());
    assert!(disk::prepare(tools, &root, &lease, size, timeout, u64::MAX).await.is_err());
    assert_eq!(sha256(fs::File::open(&again.path).unwrap()).unwrap(), original);
    fs::remove_file(&again.path).unwrap();
    let outside = temporary.path().join("outside");
    fs::write(&outside, b"outside the disk directory").unwrap();
    std::os::unix::fs::symlink(&outside, &again.path).unwrap();
    assert!(disk::prepare(tools, &root, &lease, size, timeout, 0).await.is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"outside the disk directory");
    fs::remove_file(&again.path).unwrap();
    let file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&again.path).unwrap();
    file.set_len(size).unwrap();
    let corrupt = sha256(fs::File::open(&again.path).unwrap()).unwrap();
    assert!(disk::prepare(tools, &root, &lease, size, timeout, 0).await.is_err());
    assert_eq!(
        sha256(fs::File::open(&again.path).unwrap()).unwrap(),
        corrupt,
        "Failed recovery must preserve the entire current disk"
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    fs::remove_file(&again.path).unwrap();
    let fixture = upstream_recovery_fixture();
    let fixture_file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&again.path).unwrap();
    use std::io::Write;
    (&fixture_file).write_all(&fixture).unwrap();
    fixture_file.sync_all().unwrap();
    drop(fixture_file);
    let old = fs::File::open(&again.path).unwrap();
    let old_digest = sha256(old.try_clone().unwrap()).unwrap();
    let before = old.metadata().unwrap().ino();
    assert!(
        !tools
            .builder("e2fsck")
            .args(["-f", "-n"])
            .arg(&again.path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    let recovered = disk::prepare(tools, &root, &lease, fixture.len() as u64, timeout, 0).await.unwrap();
    assert!(recovered.recovered && !recovered.created);
    assert_ne!(fs::metadata(&recovered.path).unwrap().ino(), before);
    use std::io::{Seek, SeekFrom};
    let mut old = old;
    old.seek(SeekFrom::Start(0)).unwrap();
    assert_eq!(sha256(old).unwrap(), old_digest, "The old inode must stay intact during recovery");
    assert_ne!(sha256(fs::File::open(&recovered.path).unwrap()).unwrap(), old_digest);
    assert!(
        tools
            .builder("e2fsck")
            .args(["-f", "-n"])
            .arg(&recovered.path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    println!(
        "Actual filesystems: ext4 create/reuse, active lease and unsafe input refusals, failed recovery preservation and staged upstream-fixture repair passed"
    );
}

fn upstream_recovery_fixture() -> Vec<u8> {
    let inputs: serde_json::Value =
        serde_json::from_str(include_str!("../../templates/vm-tools/filesystem-inputs.json")).unwrap();
    let owner = inputs["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["origin"] == "e2fsprogs" && source["archive"] == true)
        .unwrap();
    let source = input("PRYX_VM_FILESYSTEM_COMPONENT").join("sources/e2fsprogs").join(owner["name"].as_str().unwrap());
    assert_eq!(sha256(fs::File::open(&source).unwrap()).unwrap(), owner["sha256"].as_str().unwrap());
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(source).unwrap()));
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        if entry.path().unwrap().ends_with("tests/f_bad_bbitmap/image.gz") {
            // Keep this GPL upstream filesystem fixture a separate build input. No owner
            // code or image bytes are copied into pryx or embedded in its executable.
            let mut fixture = Vec::new();
            flate2::read::GzDecoder::new(&mut entry).take(8 * 1024 * 1024 + 1).read_to_end(&mut fixture).unwrap();
            assert!(!fixture.is_empty() && fixture.len() <= 8 * 1024 * 1024);
            return fixture;
        }
    }
    panic!("Pinned e2fsprogs source lacks its primary recovery fixture");
}

async fn forwarding(root: &std::path::Path) {
    use pryxd::runtime::vm::forward::{Binding, Forwarder};
    use std::net::Ipv4Addr;
    assert!(Binding::service(Ipv4Addr::UNSPECIFIED.into(), 0, pryx_agent::contract::AGENT_PORT).is_err());
    let directory = tempfile::tempdir().unwrap();
    let bindings = [Binding::service(Ipv4Addr::LOCALHOST.into(), 0, pryx_agent::contract::AGENT_PORT).unwrap()];
    let mut forwarder = Forwarder::start(
        &root.join("bin/vm-transport"),
        &directory.path().join("unused-vsock"),
        &bindings,
        1,
        tokio::time::Instant::now() + Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert!(forwarder.exited().unwrap().is_none());
    let address = forwarder.addresses()[0];
    assert_eq!(std::net::TcpListener::bind(address).unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
    forwarder.stop().await.unwrap();
    let listener = std::net::TcpListener::bind(address).unwrap();
    drop(listener);
    println!(
        "Actual SDK forwarder: private readiness, owned listener and port release passed; no VM transport is inferred"
    );
}
