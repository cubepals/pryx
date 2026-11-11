// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Real engine memory-sleep refusal and cold stop/wake retention. Functional control-plane
//! direct-stop selection is covered separately against real PostgreSQL and its API peer.
use super::*;

#[tokio::test]
#[ignore = "needs a real Docker-compatible engine"]
async fn an_engine_without_memory_sleep_refuses_before_freezing_and_keeps_files_after_stop() {
    let socket = pryxd::runtime::engine::socket(&DockerConfig::default()).await.unwrap();
    let runtime = Arc::new(DockerRuntime::new(&socket).unwrap());
    let limits = runtime.resource_limits().await.unwrap().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dep = deployment();
    let _cleanup = engine::EngineCleanup { socket, deployment: dep.clone() };
    let owner: String = if limits.rootless { "[workloads]\n".into() } else { HOST_OWNERSHIP.into() };
    let manager = manager_on(dir.path(), runtime, &extra(&dep).replace(HOST_OWNERSHIP, &owner));
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    let mut value = support::spec_json();
    value["entrypoint"] = serde_json::json!(["/bin/sh", "-c", "trap 'exit 0' TERM; while :; do sleep 1; done"]);
    value["resources"] = serde_json::json!({"memoryMb": 256, "allowUnenforcedMemory": true});
    value["stop"]["timeoutSeconds"] = 5.into();
    manager.ensure(id("cold"), serde_json::from_value(value).unwrap(), Precondition::None, None).await.unwrap();
    manager.start(id("cold"), None).await.unwrap();
    assert_eq!(command(&manager, "printf retained > /data/cold-marker; cat /data/cold-marker").await, "retained");
    if limits.rootless || !limits.memory {
        let error = manager.suspend(id("cold"), None).await.unwrap_err();
        assert!(matches!(error, NodeError::Conflict { code: ConflictCode::MemorySleepUnavailable, .. }));
        assert_eq!(manager.view(&id("cold")).unwrap().state, WorkloadState::Running);
    } else {
        manager.suspend(id("cold"), None).await.unwrap();
        assert_eq!(manager.start(id("cold"), None).await.unwrap().resumed, Some(true));
    }
    manager.stop(id("cold"), None, None).await.unwrap();
    manager.start(id("cold"), None).await.unwrap();
    assert_eq!(command(&manager, "cat /data/cold-marker").await, "retained");
    cleanup(&manager, &["cold"]).await;
}

async fn command(manager: &Arc<Manager>, command: &str) -> String {
    let result = manager
        .exec(
            id("cold"),
            ExecRequest { command: vec!["sh".into(), "-c".into(), command.into()], timeout_seconds: 10 },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.exit_code, Some(0));
    result.stdout
}
