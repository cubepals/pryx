// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Real engine memory-sleep refusal and cold stop/wake retention. Functional control-plane
//! direct-stop selection is covered separately against real PostgreSQL and its API peer.
use super::*;

#[tokio::test]
#[ignore = "needs a real Docker-compatible engine"]
async fn an_engine_without_memory_sleep_refuses_before_freezing_and_keeps_files_after_stop() {
    let socket = engine::selected_socket().await;
    let runtime = Arc::new(DockerRuntime::new(&socket).unwrap());
    let Some(limits) = engine::limits_or_mapping_refusal(&runtime, &socket).await else { return };
    let dir = tempfile::tempdir().unwrap();
    let dep = deployment();
    let _cleanup = engine::EngineCleanup { socket, deployment: dep.clone() };
    let manager = manager_on(dir.path(), runtime, &engine::configuration(&dep, limits.rootless));
    let report = manager.reconcile(Records::FromDisk).await;
    assert!(report.error.is_none(), "{report:?}");
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
