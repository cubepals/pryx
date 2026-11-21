// SPDX-License-Identifier: FSL-1.1-ALv2
//! Separate, opt-in test using a real committed RAM snapshot and the verified stock wake.
#[path = "fork_replay.rs"]
mod fork_replay;

use super::memory::MEMORY;
use super::tests::{artifact_server, exec, spec};
use super::{Settings, VmRuntime, directory};
use crate::runtime::ContainerRuntime;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

const BYTES: u64 = 256 * 1024 * 1024;

fn required(name: &str) -> PathBuf {
    std::env::var_os(name).expect(name).into()
}

fn checked_copy(original: &mut File, target: &Path) {
    let mut copy = OpenOptions::new().create_new(true).read(true).write(true).mode(0o600).open(target).unwrap();
    assert_eq!(std::io::copy(original, &mut copy).unwrap(), BYTES);
    copy.flush().unwrap();
    original.rewind().unwrap();
    copy.rewind().unwrap();
    let mut expected = vec![0; 128 * 1024];
    let mut actual = vec![0; expected.len()];
    for _ in 0..(BYTES / expected.len() as u64) {
        original.read_exact(&mut expected).unwrap();
        copy.read_exact(&mut actual).unwrap();
        assert!(expected == actual, "Local replay input must match every source byte");
    }
    assert_eq!(copy.metadata().unwrap().len(), BYTES);
}

#[tokio::test]
#[ignore = "requires the explicit lab replay, stock VM archive and ordinary native KVM"]
async fn captured_sleep_image() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let replay = required("PRYX_LAB_REPLAY");
    let report = required("PRYX_LAB_REPLAY_REPORT");
    let private = tempfile::tempdir().unwrap();
    let (artifact, server) = artifact_server().await;
    let settings = Settings {
        monitor_owner: None,
        artifact,
        ingress_connections: 8,
        free_floor_bytes: 0,
        image_timeout: Duration::from_secs(120),
    };
    let runtime = VmRuntime::new(private.path(), settings).await.unwrap();
    runtime.probe_memory_sleep_in_background();
    tokio::time::timeout(Duration::from_secs(120), async {
        while !runtime.memory_sleep() {
            assert!(
                !matches!(&*runtime.state.memory_sleep.lock().unwrap(), Some(Err(_))),
                "Stock restore preflight failed"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();

    let inputs: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../templates/vm-tools/image-inputs.json")).unwrap();
    let data = private.path().join("data");
    directory(&data).unwrap();
    let mut workload = spec(&data, inputs[0]["reference"].as_str().unwrap());
    workload.agent.as_mut().unwrap().keep_alive = false;
    workload.entrypoint = Some(vec!["/bin/sleep".into(), Duration::from_hours(1).as_secs().to_string()]);
    assert_eq!(u64::try_from(workload.memory_bytes).unwrap(), BYTES);
    let id = runtime.create(&workload).await.unwrap();
    runtime.start(&id).await.unwrap();
    exec(&runtime, &id, "printf captured > /tmp/lab-replay-marker").await;

    let started = Instant::now();
    runtime.pause(&id).await.unwrap();
    let capture = started.elapsed();
    let entry = runtime.entry(&id).unwrap();
    assert!(entry.memory.is_some());
    let mut original = File::open(runtime.memory_directory(&entry).unwrap().join(MEMORY)).unwrap();
    let metadata = original.metadata().unwrap();
    assert!(metadata.is_file() && metadata.permissions().mode() & 0o777 == 0o600);
    assert_eq!(metadata.len(), BYTES);

    // An open read-only descriptor survives the stock wake's removal. No page
    // from the snapshot is read by this observer until the wake has completed.
    let wake_started = Instant::now();
    runtime.unpause(&id).await.unwrap();
    let wake = wake_started.elapsed();
    assert!(exec(&runtime, &id, "cat /tmp/lab-replay-marker").await == b"captured");
    let cycle = started.elapsed();
    // Park the parent again so the offline writer and fork lookup run without a live guest.
    runtime.pause(&id).await.unwrap();

    let scratch = tempfile::tempdir_in(private.path()).unwrap();
    std::fs::set_permissions(scratch.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = private.path().join("replay-input");
    checked_copy(&mut original, &source);
    drop(original);
    let observation = serde_json::json!({
        "sourceBytes": BYTES, "sourceAllocatedBytes": metadata.blocks() * 512,
        "monitorPauseThroughCommittedSnapshotNs": capture.as_nanos() as u64,
        "unpauseThroughAuthenticatedReadinessNs": wake.as_nanos() as u64,
        "pauseThroughAuthenticatedTmpfsMarkerNs": cycle.as_nanos() as u64,
        "readOnlySourceCopyMatched": true, "authenticatedTmpfsMarkerRestored": true
    });
    std::fs::write(report.join("capture.json"), serde_json::to_vec_pretty(&observation).unwrap()).unwrap();

    let mut child = tokio::process::Command::new(replay);
    child
        .arg(&source)
        .arg(scratch.path())
        .arg(report.join("writer.csv"))
        .env_clear()
        .current_dir(scratch.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    child.env("PATH", "/usr/bin:/bin");
    let result = tokio::time::timeout(Duration::from_secs(180), child.status())
        .await
        .expect("Writer exceeded 180 seconds")
        .unwrap();
    assert!(result.success(), "Writer replay must restore every file");
    assert!(std::fs::metadata(report.join("writer.csv")).is_ok_and(|file| file.len() > 0));
    std::fs::remove_file(source).unwrap();

    fork_replay::compare(&runtime, &id, &workload, private.path(), &report).await;
    runtime.remove(&id).await.unwrap();
    server.abort();
}
