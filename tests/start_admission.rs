// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Async compute boundaries prove atomic node admission and ownership after caller cancellation.
//! The in-memory runtime supplies scheduling gates, not VM/Docker enforcement evidence; the
//! optional real-engine workflows remain responsible for boot, disk and isolation guarantees.
mod support;
use pryxd::manager::{NodeError, Precondition, Records};
use std::time::Duration;
use support::{eventually, fixture, id, spec, spec_with};

#[tokio::test]
async fn cancelled_start_keeps_memory_and_fencing_authority_until_observed() {
    let f = fixture("[capacity]\nallocatable_memory_mb = 1536\nreserved_memory_mb = 0\n").await;
    for name in ["first", "other"] {
        f.manager.ensure(id(name), spec(), Precondition::None, Some(7)).await.unwrap();
    }
    let gate = f.fake.start_gate.write().await;
    let manager = f.manager.clone();
    let caller = tokio::spawn(async move { manager.start(id("first"), Some(7)).await });
    eventually("start reaches compute", async || {
        let calls = f.fake.count_calls("start ");
        (calls == 1, calls)
    })
    .await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert_eq!(f.manager.capacity().running_memory_mb, 1024);
    assert!(f.manager.reconcile(Records::InMemory).await.error.is_none());
    assert_eq!(f.manager.capacity().running_memory_mb, 1024, "Old Created inventory cannot release an active start");
    let refused = tokio::time::timeout(Duration::from_secs(1), f.manager.start(id("other"), Some(7))).await.unwrap();
    assert!(matches!(refused, Err(NodeError::InsufficientCapacity(_))));
    assert_eq!(f.fake.count_calls("start "), 1);
    let manager = f.manager.clone();
    let mut fence = tokio::spawn(async move { manager.fence(id("first"), 8).await });
    assert!(tokio::time::timeout(Duration::from_millis(50), &mut fence).await.is_err());
    drop(gate);
    let fenced = tokio::time::timeout(Duration::from_secs(5), fence).await.unwrap().unwrap().unwrap();
    assert!(fenced.stopped);
    assert_eq!(f.manager.capacity().running_memory_mb, 0);
    assert!(f.manager.start(id("first"), Some(7)).await.is_err());
    assert!(f.manager.start(id("other"), Some(7)).await.unwrap().changed);
    assert_eq!(f.fake.count_calls("start blockly-test-first"), 1);
}

#[tokio::test]
async fn cancelled_replacement_reserves_the_new_size_and_keeps_its_lock() {
    let f = fixture("[capacity]\nallocatable_memory_mb = 2560\nreserved_memory_mb = 0\n").await;
    for name in ["first", "other"] {
        f.manager.ensure(id(name), spec(), Precondition::None, Some(7)).await.unwrap();
    }
    f.manager.start(id("first"), Some(7)).await.unwrap();
    let gate = f.fake.start_gate.write().await;
    let manager = f.manager.clone();
    let caller = tokio::spawn(async move {
        manager
            .ensure(
                id("first"),
                spec_with(|value| value["resources"]["memoryMb"] = 2048.into()),
                Precondition::None,
                Some(7),
            )
            .await
    });
    eventually("replacement reaches compute", async || {
        let calls = f.fake.count_calls("start ");
        (calls == 2, calls)
    })
    .await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert_eq!(f.manager.capacity().running_memory_mb, 2048);
    let refused = tokio::time::timeout(Duration::from_secs(1), f.manager.start(id("other"), Some(7))).await.unwrap();
    assert!(matches!(refused, Err(NodeError::InsufficientCapacity(_))));
    let manager = f.manager.clone();
    let mut fence = tokio::spawn(async move { manager.fence(id("first"), 8).await });
    assert!(tokio::time::timeout(Duration::from_millis(50), &mut fence).await.is_err());
    drop(gate);
    assert!(tokio::time::timeout(Duration::from_secs(5), fence).await.unwrap().unwrap().unwrap().stopped);
    assert_eq!(f.manager.capacity().running_memory_mb, 0);
    assert_eq!(f.fake.count_calls("start blockly-test-first"), 2);
}

#[tokio::test]
async fn an_unobserved_start_stays_charged_through_a_runtime_outage() {
    let f = fixture("").await;
    f.manager.ensure(id("first"), spec(), Precondition::None, None).await.unwrap();
    let gate = f.fake.start_gate.write().await;
    let manager = f.manager.clone();
    let caller = tokio::spawn(async move { manager.start(id("first"), None).await });
    eventually("start reaches compute", async || {
        let calls = f.fake.count_calls("start ");
        (calls == 1, calls)
    })
    .await;
    f.fake.set_available(false);
    drop(gate);
    assert!(matches!(caller.await.unwrap(), Err(NodeError::RuntimeUnavailable(_))));
    assert_eq!(f.manager.capacity().running_memory_mb, 1024);
    assert!(f.manager.reconcile(Records::InMemory).await.error.is_some());
    assert_eq!(f.manager.capacity().running_memory_mb, 1024);
    f.fake.set_available(true);
    assert!(f.manager.reconcile(Records::InMemory).await.error.is_none());
    assert_eq!(f.manager.capacity().running_memory_mb, 1024, "A running outcome remains charged once observed");
    f.manager.stop(id("first"), None, None).await.unwrap();
    assert_eq!(f.manager.capacity().running_memory_mb, 0);
}
