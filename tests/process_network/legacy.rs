// SPDX-License-Identifier: FSL-1.1-ALv2
//! Process-network adoption and legacy generations across daemon restarts.
//! The parent owns the actual node, endpoint and kernel fixtures.
use super::*;

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn the_policy_outlives_a_daemon_restart_and_nothing_outlives_the_sandbox() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    let (_service, service) = host_service();
    {
        let manager = node(dir.path(), 21400).await;
        start(&manager, "kept", serde_json::json!({ "egress": { "allow": ["example.com"] } })).await;
        assert_refused(&manager, "kept", "127.0.0.1", service).await;
    }
    // A new daemon over the same state: the supervisor ran on, and its programs with it.
    let manager = node(dir.path(), 21400).await;
    let view = until(&manager, "kept", "the adopted supervisor", |view| view.state == WorkloadState::Running).await;
    assert!(view.guarantees[&Guarantee::HostFirewall], "adopted programs are verified again");
    assert_refused(&manager, "kept", "127.0.0.1", service).await;
    assert_eq!(curl(&manager, "kept", "https://example.com").await, "200", "the gateway is served again");
    assert_ne!(curl(&manager, "kept", "https://example.org").await, "200");
    let stats = serde_json::to_value(manager.stats(&id("kept")).await.unwrap()).unwrap();
    for counted in ["networkRxBytes", "networkTxBytes"] {
        assert!(stats[counted].as_u64().is_some_and(|bytes| bytes > 0), "the programs count traffic: {stats}");
    }
    remove(&manager, &["kept"]).await;
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn a_sandbox_whose_programs_are_gone_after_a_restart_is_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    {
        // Public internet and no policy of its own: its programs are all that keep it off the host.
        let manager = node(dir.path(), 22000).await;
        start(&manager, "plain", serde_json::json!({})).await;
    }
    // While no daemon runs, its pins go, and its programs with them: what a lost pin leaves.
    let pins = std::path::Path::new("/sys/fs/bpf/pryx");
    for directory in kernel_state().difference(&before).filter(|path| path.parent() == Some(pins)) {
        for pin in std::fs::read_dir(directory).unwrap() {
            std::fs::remove_file(pin.unwrap().path()).unwrap();
        }
    }
    let manager = node(dir.path(), 22000).await;
    let view =
        until(&manager, "plain", "the unverified supervisor stopped", |view| view.state != WorkloadState::Running)
            .await;
    assert!(!view.guarantees[&Guarantee::HostFirewall], "{view:?}");
    remove(&manager, &["plain"]).await;
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn a_failed_probe_on_restart_stops_recorded_sandboxes_and_refuses_a_plain_relaunch() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    {
        let manager = node(dir.path(), 22200).await;
        start(&manager, "recorded", serde_json::json!({})).await;
    }
    let extra = extra(22200, None);
    let config = config(dir.path(), &extra);
    let served = pryxd::runtime::process::Served { ports: 22200..=22299, api_port: config.api.listen.port() };
    let runtime = pryxd::runtime::process::ProcessRuntime::new(&config.state_dir, config.process.clone())
        .unwrap()
        .with_network_programs(served, REJECTED)
        .await;
    assert_eq!(runtime.kernel_network_policy().unwrap().state, pryxd::probe::State::Off);
    let manager = manager_on(dir.path(), Arc::new(runtime), &extra);
    manager.refresh_host_probe().await;
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    let view = until(&manager, "recorded", "the previously policed supervisor stopped", |view| {
        view.state != WorkloadState::Running
    })
    .await;
    assert!(!view.guarantees[&Guarantee::HostFirewall]);
    let refused = manager.start(id("recorded"), None).await.unwrap_err();
    assert!(format!("{refused:?}").contains("kernel-network-policy"), "{refused:?}");
    remove(&manager, &["recorded"]).await;
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn a_sandbox_started_without_programs_keeps_the_tier_guarantee_off_while_it_runs() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o711)).unwrap();
    let extra = extra(22100, None);
    let daemon = |runtime: Arc<dyn ContainerRuntime>| {
        let config = Arc::new(config(dir.path(), &extra));
        let store = pryxd::store::Store::open(&config.state_dir).unwrap();
        let probe = pryxd::ports::bind_probe(config.network.edge_ips.clone());
        Manager::new(config, runtime, store, Arc::new(pryxd::metrics::Metrics::new()), probe)
    };
    {
        // A daemon from before this host had the capability: the process runtime without it.
        let config = config(dir.path(), &extra);
        let runtime = pryxd::runtime::process::ProcessRuntime::new(&config.state_dir, config.process).unwrap();
        let manager = daemon(Arc::new(runtime));
        manager.refresh_host_probe().await;
        assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
        manager.ensure(id("legacy"), sandbox(serde_json::json!({})), Precondition::None, None).await.unwrap();
        manager.start(id("legacy"), None).await.unwrap();
        until(&manager, "legacy", "a running supervisor", |view| view.state == WorkloadState::Running).await;
    }
    // The same state under a daemon with the capability: the old supervisor ran on, with none.
    let runtime = pryxd::runtime::configured::create(&config(dir.path(), &extra)).await.unwrap();
    let capability = runtime.kernel_network_policy().unwrap();
    assert_eq!(capability.state, pryxd::probe::State::Off, "{capability:?}");
    assert!(capability.evidence.starts_with("1 process sandbox(es) run without"), "{capability:?}");
    let manager = daemon(runtime);
    manager.refresh_host_probe().await;
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    let view = until(&manager, "legacy", "the adopted supervisor", |view| view.state == WorkloadState::Running).await;
    assert!(!view.guarantees[&Guarantee::HostFirewall], "{view:?}");
    assert!(!manager.features().iter().any(|feature| feature == "host-firewall"), "{:?}", manager.features());
    // Once it stops, nothing runs unpoliced and the tier's guarantee is back.
    manager.stop(id("legacy"), None, None).await.unwrap();
    until(&manager, "legacy", "the stopped supervisor", |view| view.state != WorkloadState::Running).await;
    assert!(manager.features().iter().any(|feature| feature == "host-firewall"), "{:?}", manager.features());
    // Started again, it gets its programs as any sandbox does.
    manager.start(id("legacy"), None).await.unwrap();
    let view = until(&manager, "legacy", "the restarted supervisor", |view| view.state == WorkloadState::Running).await;
    assert!(view.guarantees[&Guarantee::HostFirewall], "{view:?}");
    remove(&manager, &["legacy"]).await;
    nothing_left(&before);
}
