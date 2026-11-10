// SPDX-License-Identifier: FSL-1.1-ALv2
//! Manager identity, format and policy checks with independently observable fake backends.
//! Real Docker/process/Wasm coexistence is deployment acceptance, not established by this fixture.
mod support;
use pryxd::manager::{Precondition, Records};
use pryxd::protocol::Isolation;
use pryxd::runtime::{ContainerRuntime, fake::FakeRuntime, router::Router};
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
