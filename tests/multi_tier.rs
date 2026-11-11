// SPDX-License-Identifier: FSL-1.1-ALv2
//! Manager identity, format and policy checks with independently observable fake backends.
//! Real Docker/process/Wasm coexistence is deployment acceptance, not established by this fixture.
mod support;
use pryxd::manager::{Precondition, Records};
use pryxd::protocol::Isolation;
use pryxd::runtime::{ContainerRuntime, RuntimeError, fake::FakeRuntime, router::Router};
use std::sync::Arc;

fn spec(tier: Option<&str>) -> pryxd::protocol::WorkloadSpec {
    let mut value = support::spec_json();
    if let Some(tier) = tier {
        value["isolation"] = tier.into();
        if tier != "container" {
            value["image"] = "".into();
        }
    }
    value["resources"] = serde_json::json!({"memoryMb": 128});
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn tiers_share_one_manager_and_keep_their_identity_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let container = Arc::new(FakeRuntime::new());
    let process = Arc::new(FakeRuntime::new().with_isolation(Isolation::Process));
    let wasm = Arc::new(FakeRuntime::new().with_isolation(Isolation::Wasm));
    let router = Arc::new(Router::new(vec![container.clone(), process.clone(), wasm.clone()]).unwrap());
    let manager = support::manager_on(dir.path(), router, "runtime = 'auto'");
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    let legacy = spec(None);
    let digest = legacy.record().digest();
    manager.ensure(support::id("legacy"), legacy, Precondition::None, None).await.unwrap();
    for tier in ["container", "process", "wasm"] {
        let id = support::id(tier);
        manager.ensure(id.clone(), spec(Some(tier)), Precondition::None, None).await.unwrap();
        manager.start(id.clone(), None).await.unwrap();
        let view = serde_json::to_value(manager.view(&id).unwrap()).unwrap();
        assert_eq!(view["isolation"].as_str().unwrap_or("container"), tier);
        assert_eq!(view["guarantees"]["private-network"], tier == "container");
        assert_eq!(view["guarantees"]["cpu-limit"], false);
    }
    assert_eq!(manager.view(&support::id("legacy")).unwrap().spec_digest, digest);
    assert_eq!(container.count_calls("create "), 2);
    assert_eq!(process.count_calls("create "), 1);
    assert_eq!(wasm.count_calls("create "), 1);
    let refused = manager.ensure(support::id("process"), spec(Some("wasm")), Precondition::None, None).await;
    tier_refusal(refused.unwrap_err(), "process");
    assert_eq!(wasm.count_calls("create "), 1);
    drop(manager);
    let router = Arc::new(Router::new(vec![wasm.clone(), process.clone(), container.clone()]).unwrap());
    let restarted = support::manager_on(dir.path(), router, "runtime = 'auto'");
    assert!(restarted.reconcile(Records::FromDisk).await.error.is_none());
    let mut no_choice = serde_json::to_value(spec(Some("process")).record()).unwrap();
    no_choice.as_object_mut().unwrap().remove("isolation");
    no_choice.as_object_mut().unwrap().remove("secretNames");
    let no_choice = serde_json::from_value(no_choice).unwrap();
    restarted.ensure(support::id("process"), no_choice, Precondition::None, None).await.unwrap();
    assert_eq!(serde_json::to_value(restarted.view(&support::id("process")).unwrap()).unwrap()["isolation"], "process");
    assert_eq!(restarted.view(&support::id("legacy")).unwrap().spec_digest, digest);
}

#[tokio::test]
async fn a_missing_recorded_backend_is_refused_without_using_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let process = Arc::new(FakeRuntime::new().with_isolation(Isolation::Process));
    let first = support::manager_on(dir.path(), process.clone(), "runtime = 'process'");
    first.reconcile(Records::FromDisk).await;
    first.ensure(support::id("bound"), spec(Some("process")), Precondition::None, None).await.unwrap();
    drop(first);
    let container = Arc::new(FakeRuntime::new());
    let router = Arc::new(Router::new(vec![container.clone()]).unwrap());
    let second = support::manager_on(dir.path(), router, "runtime = 'auto'");
    second.reconcile(Records::FromDisk).await;
    tier_refusal(
        second.ensure(support::id("bound"), spec(None), Precondition::None, None).await.unwrap_err(),
        "process",
    );
    assert_eq!(container.count_calls("create "), 0);
    assert_eq!(container.count_calls("start "), 0);
}

fn tier_refusal(error: pryxd::manager::NodeError, tier: &str) {
    let pryxd::manager::NodeError::Invalid(fields) = error else {
        panic!("unexpected refusal: {error:?}");
    };
    let fields = serde_json::to_value(fields).unwrap();
    assert_eq!(fields[0]["field"], "isolation");
    assert!(fields[0]["problem"].as_str().unwrap().contains(tier));
}

#[tokio::test]
async fn explicit_process_and_container_configuration_stays_single_tier() {
    for tier in ["process", "container"] {
        let dir = tempfile::tempdir().unwrap();
        let config = support::config(dir.path(), &format!("runtime = '{tier}'"));
        std::fs::create_dir_all(&config.state_dir).unwrap();
        let runtime = pryxd::runtime::configured::create(&config).await.unwrap();
        assert_eq!(runtime.isolation(), tier);
        assert_eq!(runtime.tiers(), vec![tier]);
        assert!(!runtime.multi_tier());
    }
}

#[tokio::test]
async fn a_backend_outage_and_identity_collision_cannot_redirect_a_mutation() {
    let container = Arc::new(FakeRuntime::new());
    let process = Arc::new(FakeRuntime::new().with_isolation(Isolation::Process));
    let router = Router::new(vec![container.clone(), process.clone()]).unwrap();
    assert!(router.info().await.is_ok());
    router.bind("bound", Some("runtime-id"), "container").unwrap();
    assert!(router.bind("other", Some("runtime-id"), "process").is_err());
    assert!(router.kill("other").await.is_err());
    assert_eq!(process.count_calls("kill "), 0);
    process.set_available(false);
    assert!(router.info().await.unwrap_err().to_string().contains("process"));
    assert!(router.kill("runtime-id").await.is_err());
    assert_eq!(container.count_calls("kill "), 1);
    assert_eq!(process.count_calls("kill "), 0);
}

#[tokio::test]
async fn changing_to_a_single_backend_cannot_redirect_saved_workloads() {
    let dir = tempfile::tempdir().unwrap();
    let process = Arc::new(FakeRuntime::new().with_isolation(Isolation::Process));
    let first = support::manager_on(dir.path(), process.clone(), "runtime = 'process'");
    first.reconcile(Records::FromDisk).await;
    let id = support::id("saved-process");
    first.ensure(id.clone(), spec(Some("process")), Precondition::None, None).await.unwrap();
    first.start(id.clone(), None).await.unwrap();
    drop(first);
    let container = Arc::new(FakeRuntime::new());
    let second = support::manager_on(dir.path(), container.clone(), "runtime = 'container'");
    let report = second.reconcile(Records::FromDisk).await;
    assert!(report.error.as_deref().unwrap().contains("process"));
    assert!(second.start(id.clone(), None).await.is_err());
    assert!(second.kill(id.clone(), None).await.is_err());
    assert!(
        second.delete(id.clone(), pryxd::protocol::DataDisposition::Delete, Some(id.to_string()), None).await.is_err()
    );
    tier_refusal(second.ensure(id, spec(None), Precondition::None, None).await.unwrap_err(), "process");
    for verb in ["create ", "start ", "kill ", "remove "] {
        assert_eq!(container.count_calls(verb), 0);
    }
    assert_eq!(process.count_calls("kill "), 0);
    assert_eq!(process.count_calls("remove "), 0);
}

#[tokio::test]
async fn legacy_records_use_their_actual_backend_and_refuse_ambiguous_resting_state() {
    let dir = tempfile::tempdir().unwrap();
    let process = Arc::new(FakeRuntime::new().with_isolation(Isolation::Process));
    let first = support::manager_on(dir.path(), process.clone(), "runtime = 'process'");
    first.reconcile(Records::FromDisk).await;
    let mut value = support::spec_json();
    value["image"] = "".into();
    value["resources"] = serde_json::json!({"memoryMb": 128});
    let legacy: pryxd::protocol::WorkloadSpec = serde_json::from_value(value).unwrap();
    for name in ["legacy-running", "legacy-resting"] {
        let id = support::id(name);
        first.ensure(id.clone(), legacy.clone(), Precondition::None, None).await.unwrap();
        if name == "legacy-running" {
            first.start(id.clone(), None).await.unwrap();
        } else {
            first.delete(id.clone(), pryxd::protocol::DataDisposition::Keep, None, None).await.unwrap();
        }
        let path = first.store.workload_dir(&id).join("workload.json");
        let mut record: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        record.as_object_mut().unwrap().remove("isolation");
        std::fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    }
    drop(first);
    let container = Arc::new(FakeRuntime::new());
    let router = Arc::new(Router::new(vec![container.clone(), process.clone()]).unwrap());
    let second = support::manager_on(dir.path(), router, "runtime = 'auto'");
    let report = second.reconcile(Records::FromDisk).await;
    assert!(report.error.as_deref().unwrap().contains("unresolved"));
    let running = support::id("legacy-running");
    assert_eq!(serde_json::to_value(second.view(&running).unwrap()).unwrap()["isolation"], "process");
    second.start(running, None).await.unwrap();
    let resting = support::id("legacy-resting");
    tier_refusal(second.ensure(resting.clone(), legacy, Precondition::None, None).await.unwrap_err(), "unresolved");
    assert!(second.start(resting, None).await.is_err());
    assert_eq!(container.count_calls("create "), 0);
    assert_eq!(container.count_calls("start "), 0);
}

struct SameName {
    _directories: [tempfile::TempDir; 2],
    container: Arc<FakeRuntime>,
    process: Arc<FakeRuntime>,
    record: pryxd::store::WorkloadRecord,
}

async fn same_name_backends() -> SameName {
    let directories = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let container = Arc::new(FakeRuntime::new());
    let process = Arc::new(FakeRuntime::new().with_isolation(Isolation::Process));
    let first = support::manager_on(directories[0].path(), container.clone(), "runtime = 'container'");
    let second = support::manager_on(directories[1].path(), process.clone(), "runtime = 'process'");
    first.reconcile(Records::FromDisk).await;
    second.reconcile(Records::FromDisk).await;
    first.ensure(support::id("same-name"), spec(Some("container")), Precondition::None, None).await.unwrap();
    second.ensure(support::id("same-name"), spec(Some("process")), Precondition::None, None).await.unwrap();
    let record = first.store.load_all().unwrap().0.pop().unwrap();
    assert!(process.container(&record.container_name).is_some());
    SameName { _directories: directories, container, process, record }
}

#[tokio::test]
async fn inspection_discovery_failure_cannot_publish_a_partial_route() {
    let f = same_name_backends().await;
    let router = Router::new(vec![f.container.clone(), f.process.clone()]).unwrap();
    let inspected = router.inspect(&f.record.container_name).await;
    let _ = router.start(&f.record.container_name).await;
    assert_eq!(f.container.count_calls("start "), 0);
    assert_eq!(f.process.count_calls("start "), 0);
    assert!(matches!(inspected, Err(RuntimeError::Conflict(_))));
}

#[tokio::test]
async fn list_discovery_failure_cannot_publish_a_partial_route() {
    let f = same_name_backends().await;
    let router = Router::new(vec![f.container.clone(), f.process.clone()]).unwrap();
    let listed = router.list(&[]).await;
    let _ = router.start(&f.record.container_name).await;
    assert_eq!(f.container.count_calls("start "), 0);
    assert_eq!(f.process.count_calls("start "), 0);
    assert!(matches!(listed, Err(RuntimeError::Conflict(_))));
}

#[tokio::test]
async fn list_discovery_cannot_redirect_a_recorded_tier_to_another_backend() {
    let f = same_name_backends().await;
    f.container.vanish(&f.record.container_name);
    let router = Router::new(vec![f.container.clone(), f.process.clone()]).unwrap();
    router.bind(&f.record.container_name, f.record.container_id.as_deref(), "container").unwrap();
    let listed = router.list(&[]).await;
    let _ = router.start(&f.record.container_name).await;
    assert_eq!(f.process.count_calls("start "), 0);
    assert!(matches!(listed, Err(RuntimeError::Conflict(_))));
    assert_eq!(router.bound_isolation(&f.record.container_name), Some("container"));
}

#[tokio::test]
async fn an_engine_outage_leaves_other_tiers_serving_and_preserves_saved_compute() {
    use pryxd::protocol::WorkloadState;
    let dir = tempfile::tempdir().unwrap();
    let container = Arc::new(FakeRuntime::new());
    let process = Arc::new(FakeRuntime::new().with_isolation(Isolation::Process));
    let wasm = Arc::new(FakeRuntime::new().with_isolation(Isolation::Wasm));
    let router = Arc::new(Router::new(vec![container.clone(), process.clone(), wasm.clone()]).unwrap());
    let manager = support::manager_on(dir.path(), router.clone(), "runtime = 'auto'");
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    for tier in ["container", "process", "wasm"] {
        manager.ensure(support::id(tier), spec(Some(tier)), Precondition::None, None).await.unwrap();
        manager.start(support::id(tier), None).await.unwrap();
    }
    let before = manager.view(&support::id("container")).unwrap();
    container.set_available(false);
    let health = manager.health().await;
    assert_eq!(health.status, "ok");
    assert!(!manager.features().contains(&"tier-container".into()));
    assert!(manager.features().contains(&"tier-process".into()));
    assert!(manager.features().contains(&"tier-wasm".into()));
    assert_eq!(manager.view(&support::id("container")).unwrap().state, WorkloadState::Unknown);
    assert!(manager.reconcile(Records::InMemory).await.error.is_none());
    assert!(manager.start(support::id("container"), None).await.is_err());
    for tier in ["process", "wasm"] {
        assert_eq!(manager.view(&support::id(tier)).unwrap().state, WorkloadState::Running);
        manager.stop(support::id(tier), None, None).await.unwrap();
        manager.start(support::id(tier), None).await.unwrap();
        assert_eq!(manager.view(&support::id(tier)).unwrap().state, WorkloadState::Running);
    }
    let during = manager.view(&support::id("container")).unwrap();
    assert_eq!(during.locate.container_id, before.locate.container_id);
    assert!(during.guarantees.values().all(|on| !on));
    assert_eq!(during.generation, before.generation);
    assert_eq!(during.spec_digest, before.spec_digest);
    assert!(!during.issues.iter().any(|issue| issue.code == pryxd::protocol::IssueCode::ContainerMissing));
    container.set_available(true);
    assert!(!manager.features().contains(&"tier-container".into()), "health alone cannot publish stale observations");
    assert_eq!(manager.view(&support::id("container")).unwrap().state, WorkloadState::Unknown);
    assert!(manager.reconcile(Records::InMemory).await.error.is_none());
    assert_eq!(manager.view(&support::id("container")).unwrap().state, WorkloadState::Running);
    assert!(manager.features().contains(&"tier-container".into()));
    assert_eq!(container.count_calls("create "), 1);
    assert_eq!(process.count_calls("create "), 1);
    assert_eq!(wasm.count_calls("create "), 1);
}

#[tokio::test]
async fn all_tiers_down_degrades_the_node_and_one_recovered_tier_can_reconcile() {
    let dir = tempfile::tempdir().unwrap();
    let container = Arc::new(FakeRuntime::new());
    let process = Arc::new(FakeRuntime::new().with_isolation(Isolation::Process));
    let router = Arc::new(Router::new(vec![container.clone(), process.clone()]).unwrap());
    let manager = support::manager_on(dir.path(), router, "runtime = 'auto'");
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    container.set_available(false);
    process.set_available(false);
    assert_eq!(manager.health().await.status, "degraded");
    assert!(manager.reconcile(Records::InMemory).await.error.is_some());
    process.set_available(true);
    assert!(manager.reconcile(Records::InMemory).await.error.is_none());
    assert_eq!(manager.health().await.status, "ok");
    manager.ensure(support::id("new-process"), spec(Some("process")), Precondition::None, None).await.unwrap();
    manager.start(support::id("new-process"), None).await.unwrap();
    assert_eq!(container.count_calls("create "), 0);
    assert_eq!(process.count_calls("create "), 1);
}

#[tokio::test]
async fn restarting_with_an_unreadable_engine_keeps_its_record_bytes_and_route() {
    let dir = tempfile::tempdir().unwrap();
    let container = Arc::new(FakeRuntime::new());
    let process = Arc::new(FakeRuntime::new().with_isolation(Isolation::Process));
    let router = Arc::new(Router::new(vec![container.clone(), process.clone()]).unwrap());
    let manager = support::manager_on(dir.path(), router, "runtime = 'auto'");
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    let id = support::id("saved");
    manager.ensure(id.clone(), spec(Some("container")), Precondition::None, None).await.unwrap();
    manager.start(id.clone(), None).await.unwrap();
    let path = manager.store.workload_dir(&id).join("workload.json");
    let before = std::fs::read(&path).unwrap();
    drop(manager);
    container.set_available(false);
    let router = Arc::new(Router::new(vec![process.clone(), container.clone()]).unwrap());
    let manager = support::manager_on(dir.path(), router, "runtime = 'auto'");
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert_eq!(manager.view(&id).unwrap().state, pryxd::protocol::WorkloadState::Unknown);
    assert!(manager.start(id.clone(), None).await.is_err());
    assert_eq!(process.count_calls("start "), 0);
    assert_eq!(process.count_calls("create "), 0);
    container.set_available(true);
    assert!(manager.reconcile(Records::InMemory).await.error.is_none());
    assert_eq!(manager.view(&id).unwrap().state, pryxd::protocol::WorkloadState::Running);
    assert_eq!(container.count_calls("create "), 1);
}

#[tokio::test]
async fn a_crash_discovered_after_an_engine_outage_is_not_a_watched_failure() {
    let dir = tempfile::tempdir().unwrap();
    let container = Arc::new(FakeRuntime::new());
    let process = Arc::new(FakeRuntime::new().with_isolation(Isolation::Process));
    let router = Arc::new(Router::new(vec![container.clone(), process]).unwrap());
    let manager = support::manager_on(dir.path(), router, "runtime = 'auto'");
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    let mut value = support::spec_json();
    value["isolation"] = "container".into();
    value["restart"] = serde_json::json!({"policy": "on-failure", "maxRetries": 3});
    let id = support::id("unwatched");
    manager.ensure(id.clone(), serde_json::from_value(value).unwrap(), Precondition::None, None).await.unwrap();
    manager.start(id.clone(), None).await.unwrap();
    let name = manager.view(&id).unwrap().locate.container_name;
    container.set_available(false);
    assert_eq!(manager.health().await.status, "ok");
    container.crash(&name, 1, false);
    container.set_available(true);
    assert!(manager.reconcile(Records::InMemory).await.error.is_none());
    let view = manager.view(&id).unwrap();
    assert_eq!(view.state, pryxd::protocol::WorkloadState::Crashed);
    assert_eq!(view.last_failure_at, None);
    assert_eq!(view.restart_count, 0);
}
