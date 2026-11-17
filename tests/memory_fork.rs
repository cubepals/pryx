// SPDX-License-Identifier: FSL-1.1-ALv2
//! Completed memory-fork restores survive a lost reply without copying again. The opt-in fake
//! supplies a paused target; the manager's real records, atomic suspension file, reconciliation,
//! epoch checks and disk admission are exercised here without a VM or privileged host control.
mod support;

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use pryxd::ids::SnapshotId;
use pryxd::manager::{ConflictCode, NodeError, Precondition, Records};
use pryxd::protocol::{EnsureOutcome, RestoreRequest, SnapshotRequest, WorkloadSpec, WorkloadState};
use pryxd::runtime::ContainerRuntime;
use serde_json::{Value, json};
use support::{Fixture, fixture, id, manager_on, spec_with};

const SNAPSHOT: &str = "0b6f1f2e-6a47-4c9a-9a39-2f4ac7e51f10";
const TARGET_EPOCH: u64 = 2;
const TARGET: &str = "blockly-test-fork";

fn specification() -> WorkloadSpec {
    spec_with(|value| value["resumesOn"] = "fork-replay".into())
}

fn request(source: &str, snapshot: &str) -> RestoreRequest {
    serde_json::from_value(json!({"fork":{"workload":source,"snapshot":snapshot}})).unwrap()
}

fn suspension(f: &Fixture) -> PathBuf {
    f.manager.store.workload_dir(&id("fork")).join("suspend.json")
}

async fn ready(extra: &str) -> Fixture {
    let f = fixture(extra).await;
    f.fake.memory_forks.store(true, Ordering::SeqCst);
    let made = f.manager.ensure(id("source"), specification(), Precondition::None, Some(1)).await.unwrap();
    std::fs::write(PathBuf::from(made.workload.locate.data_dir).join("data"), b"saved").unwrap();
    f.manager.start(id("source"), Some(1)).await.unwrap();
    f.manager.suspend(id("source"), Some(1)).await.unwrap();
    let snapshot: SnapshotRequest = serde_json::from_value(json!({"id":SNAPSHOT,"quiesced":true})).unwrap();
    let taken = f.manager.snapshot(id("source"), snapshot, Some(1)).await.unwrap();
    assert!(taken.snapshot.memory.is_some(), "the memory path is actually selected");
    f.manager.ensure(id("fork"), specification(), Precondition::None, Some(TARGET_EPOCH)).await.unwrap();
    f
}

async fn completed() -> Fixture {
    let f = ready("").await;
    f.manager.restore(id("fork"), request("source", SNAPSHOT), Some(TARGET_EPOCH)).await.unwrap();
    assert_eq!(f.fake.count_calls("fork-memory "), 1);
    assert_eq!(f.manager.view(&id("fork")).unwrap().state, WorkloadState::Suspended);
    f
}

#[tokio::test]
async fn a_lost_fork_reply_replays_after_the_same_put_and_a_fresh_manager() {
    let f = ready("").await;
    let reply = f.manager.restore(id("fork"), request("source", SNAPSHOT), Some(TARGET_EPOCH)).await.unwrap();
    // The node committed this reply, but control did not receive it or settle its placement.
    let original = f.fake.container(TARGET).unwrap();
    let saved = std::fs::read(suspension(&f)).unwrap();
    let unchanged =
        f.manager.ensure(id("fork"), specification(), Precondition::None, Some(TARGET_EPOCH)).await.unwrap();
    assert_eq!(unchanged.outcome, EnsureOutcome::Unchanged);
    assert_eq!(std::fs::read(suspension(&f)).unwrap(), saved, "the same PUT retains completion");
    // Completion belongs to the target, even after the source wakes and its snapshot is gone.
    f.manager.start(id("source"), Some(1)).await.unwrap();
    f.manager.delete_snapshot(id("source"), SnapshotId::parse(SNAPSHOT).unwrap()).await.unwrap();
    let restarted = manager_on(f.dir.path(), f.fake.clone(), "");
    assert!(restarted.reconcile(Records::FromDisk).await.error.is_none());
    let unchanged =
        restarted.ensure(id("fork"), specification(), Precondition::None, Some(TARGET_EPOCH)).await.unwrap();
    assert_eq!(unchanged.outcome, EnsureOutcome::Unchanged);
    let full = manager_on(f.dir.path(), f.fake.clone(), "[capacity]\nmin_free_disk_mb = 1000000000\n");
    assert!(full.reconcile(Records::FromDisk).await.error.is_none());
    let replay = full.restore(id("fork"), request("source", SNAPSHOT), Some(TARGET_EPOCH)).await.unwrap();
    assert_eq!(replay, reply, "return the durably committed reply");
    assert_eq!(f.fake.count_calls("fork-memory "), 1, "no second fork");
    assert_eq!(f.fake.container(TARGET).unwrap().spec.labels, original.spec.labels);
    assert_eq!(restarted.view(&id("fork")).unwrap().locate.container_id, unchanged.workload.locate.container_id);
    assert_eq!(full.view(&id("fork")).unwrap().state, WorkloadState::Suspended);
}

#[tokio::test]
async fn a_paused_target_needs_the_exact_completed_fork_and_current_epoch() {
    let f = completed().await;
    for (source, snapshot) in [("other", SNAPSHOT), ("source", "1b6f1f2e-6a47-4c9a-9a39-2f4ac7e51f10")] {
        let error = f.manager.restore(id("fork"), request(source, snapshot), Some(TARGET_EPOCH)).await.unwrap_err();
        assert!(matches!(error, NodeError::Conflict { code: ConflictCode::NotStopped, .. }), "{error:?}");
    }
    for epoch in [1, 3] {
        let error = f.manager.restore(id("fork"), request("source", SNAPSHOT), Some(epoch)).await.unwrap_err();
        assert!(matches!(error, NodeError::StaleEpoch { .. } | NodeError::EpochAhead { .. }), "{error:?}");
    }
    let path = suspension(&f);
    let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("forkRestore");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let restarted = manager_on(f.dir.path(), f.fake.clone(), "");
    assert!(restarted.reconcile(Records::FromDisk).await.error.is_none());
    let error = restarted.restore(id("fork"), request("source", SNAPSHOT), Some(TARGET_EPOCH)).await.unwrap_err();
    assert!(matches!(error, NodeError::Conflict { code: ConflictCode::NotStopped, .. }), "{error:?}");
    assert_eq!(f.fake.count_calls("fork-memory "), 1, "a partial operation is never repeated");
}

#[tokio::test]
async fn a_receipt_for_another_target_generation_spec_or_container_is_refused() {
    let f = completed().await;
    let path = suspension(&f);
    let original: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for (field, bad) in [
        ("target", json!("another")),
        ("epoch", json!(3)),
        ("generation", json!(999)),
        ("specDigest", json!("sha256:another")),
        ("containerName", json!("another")),
        ("containerId", json!("another")),
    ] {
        let mut value = original.clone();
        value["forkRestore"][field] = bad;
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let restarted = manager_on(f.dir.path(), f.fake.clone(), "");
        assert!(restarted.reconcile(Records::FromDisk).await.error.is_none());
        let error = restarted.restore(id("fork"), request("source", SNAPSHOT), Some(TARGET_EPOCH)).await.unwrap_err();
        assert!(matches!(error, NodeError::Conflict { code: ConflictCode::NotStopped, .. }), "{field}: {error:?}");
    }
    assert_eq!(f.fake.count_calls("fork-memory "), 1);
}

#[tokio::test]
async fn a_new_paused_container_under_the_same_name_cannot_claim_the_old_completion() {
    let f = completed().await;
    let original = f.fake.container(TARGET).unwrap();
    f.fake.vanish(TARGET);
    let replacement = f.fake.create(&original.spec).await.unwrap();
    assert_ne!(Some(replacement), f.manager.view(&id("fork")).unwrap().locate.container_id);
    f.fake.start(TARGET).await.unwrap();
    f.fake.pause(TARGET).await.unwrap();
    let error = f.manager.restore(id("fork"), request("source", SNAPSHOT), Some(TARGET_EPOCH)).await.unwrap_err();
    assert!(matches!(error, NodeError::Conflict { code: ConflictCode::NotStopped, .. }), "{error:?}");
    assert_eq!(f.fake.count_calls("fork-memory "), 1);
}

#[tokio::test]
async fn a_pending_spec_and_a_later_suspension_do_not_replay_the_old_fork() {
    let f = completed().await;
    let changed = spec_with(|value| {
        value["resumesOn"] = "fork-replay".into();
        value["env"]["VERSION"] = "next".into();
    });
    let deferred = f.manager.ensure(id("fork"), changed, Precondition::None, Some(TARGET_EPOCH)).await.unwrap();
    assert_eq!(deferred.outcome, EnsureOutcome::Deferred);
    let error = f.manager.restore(id("fork"), request("source", SNAPSHOT), Some(TARGET_EPOCH)).await.unwrap_err();
    assert!(matches!(error, NodeError::Conflict { code: ConflictCode::NotStopped, .. }), "{error:?}");
    // Clearing the pending spec changes no memory; starting ends the old completion.
    f.manager.ensure(id("fork"), specification(), Precondition::None, Some(TARGET_EPOCH)).await.unwrap();
    f.manager.start(id("fork"), Some(TARGET_EPOCH)).await.unwrap();
    f.manager.suspend(id("fork"), Some(TARGET_EPOCH)).await.unwrap();
    let value: Value = serde_json::from_slice(&std::fs::read(suspension(&f)).unwrap()).unwrap();
    assert!(value.get("forkRestore").is_none(), "a new suspension has no old receipt");
    let error = f.manager.restore(id("fork"), request("source", SNAPSHOT), Some(TARGET_EPOCH)).await.unwrap_err();
    assert!(matches!(error, NodeError::Conflict { code: ConflictCode::NotStopped, .. }), "{error:?}");
    assert_eq!(f.fake.count_calls("fork-memory "), 1);
}

#[tokio::test]
async fn stop_and_fence_end_the_completed_fork_receipt() {
    for fence in [false, true] {
        let f = completed().await;
        if fence {
            f.manager.fence(id("fork"), TARGET_EPOCH + 1).await.unwrap();
        } else {
            f.manager.stop(id("fork"), None, Some(TARGET_EPOCH)).await.unwrap();
        }
        let value: Value = serde_json::from_slice(&std::fs::read(suspension(&f)).unwrap()).unwrap();
        assert!(value.get("forkRestore").is_none());
        assert_eq!(f.fake.count_calls("fork-memory "), 1);
    }
}

#[tokio::test]
async fn a_memory_fork_reserves_the_snapshots_full_sparse_size_before_the_runtime() {
    let f = ready("[capacity]\nmin_free_disk_mb = 0\n").await;
    let snapshot = f.manager.store.snapshot_dir(&id("source"), &SnapshotId::parse(SNAPSHOT).unwrap());
    let (_, available) = pryxd::host::disk(&snapshot).expect("the fixture's real filesystem");
    let sparse = snapshot.join("data/large-disk");
    let file = std::fs::File::create(&sparse).unwrap();
    file.set_len(available.saturating_add(1024 * 1024 * 1024)).unwrap();
    let logical_size = file.metadata().unwrap().len();
    let mut view: Value = serde_json::from_slice(&std::fs::read(snapshot.join("snapshot.json")).unwrap()).unwrap();
    view["sizeBytes"] = view["sizeBytes"].as_u64().unwrap().saturating_add(logical_size).into();
    view["files"] = (view["files"].as_u64().unwrap() + 1).into();
    std::fs::write(snapshot.join("snapshot.json"), serde_json::to_vec(&view).unwrap()).unwrap();
    let error = f.manager.restore(id("fork"), request("source", SNAPSHOT), Some(TARGET_EPOCH)).await.unwrap_err();
    assert!(matches!(error, NodeError::InsufficientDisk(_)), "{error:?}");
    assert_eq!(f.fake.count_calls("fork-memory "), 0, "full-size refusal precedes every runtime copy");
    assert_eq!(f.manager.view(&id("fork")).unwrap().state, WorkloadState::Created);
    assert!(
        !suspension(&f).exists() || {
            let value: Value = serde_json::from_slice(&std::fs::read(suspension(&f)).unwrap()).unwrap();
            value.get("forkRestore").is_none()
        }
    );
}
