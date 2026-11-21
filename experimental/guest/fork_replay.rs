// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Compare two privately held sleep images after a fork has actually run and changed.
//! Only aggregate match counts and timing can leave the disposable test directory.
use super::super::ContainerSpec;
use super::{BYTES, ContainerRuntime, MEMORY, VmRuntime, exec, required, spec};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

fn image(runtime: &VmRuntime, id: &str) -> PathBuf {
    let entry = runtime.entry(id).unwrap();
    assert!(entry.memory.is_some());
    let path = runtime.memory_directory(&entry).unwrap().join(MEMORY);
    let metadata = std::fs::symlink_metadata(&path).unwrap();
    assert!(metadata.is_file() && metadata.mode() & 0o777 == 0o600);
    assert_eq!(metadata.len(), BYTES);
    path
}

pub(super) async fn compare(runtime: &VmRuntime, parent: &str, source: &ContainerSpec, private: &Path, report: &Path) {
    let mut fork_spec = spec(&private.join("fork-data"), &source.image);
    fork_spec.entrypoint.clone_from(&source.entrypoint);
    fork_spec.agent.as_mut().unwrap().keep_alive = source.agent.as_ref().unwrap().keep_alive;
    let fork = runtime.fork_with_memory(parent, &fork_spec).await.unwrap();
    runtime.unpause(&fork).await.unwrap();
    assert_eq!(exec(runtime, &fork, "cat /tmp/lab-replay-marker").await, b"captured");
    // Post-restore random data makes this a genuinely diverged guest. Keep every byte private.
    let changed = "dd if=/dev/urandom of=/tmp/lab-fork-new bs=4096 count=32 2>/dev/null && wc -c < /tmp/lab-fork-new";
    assert_eq!(String::from_utf8(exec(runtime, &fork, changed).await).unwrap().trim(), "131072");
    runtime.pause(&fork).await.unwrap();

    let original = image(runtime, parent);
    let modified = image(runtime, &fork);
    let identity = |path: &Path| {
        let meta = std::fs::metadata(path).unwrap();
        (meta.dev(), meta.ino())
    };
    assert_ne!(identity(&original), identity(&modified), "The untouched fork initially shares the parent inode");
    let output = report.join("fork-lookup.csv");
    let mut child = tokio::process::Command::new(required("PRYX_LAB_FORK_LOOKUP"));
    child
        .arg(original)
        .arg(modified)
        .arg(&output)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .current_dir(private)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let done = tokio::time::timeout(Duration::from_secs(180), child.status()).await.unwrap().unwrap();
    assert!(done.success(), "The fork lookup must agree on every page with the bytewise oracle");

    runtime.unpause(&fork).await.unwrap();
    let length = exec(runtime, &fork, "wc -c < /tmp/lab-fork-new").await;
    assert_eq!(String::from_utf8(length).unwrap().trim(), "131072");
    runtime.remove(&fork).await.unwrap();
    runtime.unpause(parent).await.unwrap();
    assert_eq!(exec(runtime, parent, "test ! -e /tmp/lab-fork-new && cat /tmp/lab-replay-marker").await, b"captured");
}
