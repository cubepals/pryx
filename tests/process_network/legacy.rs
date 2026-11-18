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
async fn a_previous_object_record_is_stopped_before_new_policy_is_advertised() {
    for identity in [None, Some("another-object")] {
        let dir = tempfile::tempdir().unwrap();
        let before = kernel_state();
        {
            let manager = node(dir.path(), 23800).await;
            start(&manager, "old-object", serde_json::json!({})).await;
        }
        // Keep the real pins and their recorded kernel IDs. Only the private code identity is
        // older, so adopting the unchanged IDs alone would wrongly accept the previous policy.
        let history = config(dir.path(), &extra(23800, None)).state_dir.join("process-runtime");
        let records: Vec<_> = std::fs::read_dir(history)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension().is_some_and(|suffix| suffix == "json")
                    && path.file_name().is_some_and(|name| name != "owners.json")
            })
            .collect();
        assert_eq!(records.len(), 1);
        let path = &records[0];
        let mut record: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let network = record["network"].as_object_mut().unwrap();
        assert!(network["object_sha256"].as_str().is_some_and(|digest| digest.len() == 64));
        match identity {
            Some(identity) => {
                network.insert("object_sha256".into(), identity.into());
            }
            None => {
                network.remove("object_sha256");
            }
        }
        std::fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
        let manager = node(dir.path(), 23800).await;
        let stopped = until(&manager, "old-object", "the previous object supervisor stopped", |view| {
            view.state != WorkloadState::Running
        })
        .await;
        assert!(!stopped.guarantees[&Guarantee::HostFirewall]);
        manager.start(id("old-object"), None).await.unwrap();
        let fresh =
            until(&manager, "old-object", "a fresh current-object start", |view| view.state == WorkloadState::Running)
                .await;
        assert!(fresh.guarantees[&Guarantee::HostFirewall]);
        remove(&manager, &["old-object"]).await;
        nothing_left(&before);
    }
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
    let orphan = {
        // A daemon from before this host had the capability: the process runtime without it.
        let config = config(dir.path(), &extra);
        let runtime = pryxd::runtime::process::ProcessRuntime::new(&config.state_dir, config.process).unwrap();
        let manager = daemon(Arc::new(runtime));
        manager.refresh_host_probe().await;
        assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
        manager.ensure(id("legacy"), sandbox(serde_json::json!({})), Precondition::None, None).await.unwrap();
        manager.start(id("legacy"), None).await.unwrap();
        until(&manager, "legacy", "a running supervisor", |view| view.state == WorkloadState::Running).await;
        Orphan::start(&manager, "legacy").await
    };
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
    manager.suspend(id("legacy"), None).await.unwrap();
    manager.stop(id("legacy"), None, None).await.unwrap();
    until(&manager, "legacy", "the stopped supervisor", |view| view.state != WorkloadState::Running).await;
    orphan.gone().await;
    assert!(manager.features().iter().any(|feature| feature == "host-firewall"), "{:?}", manager.features());
    // Started again, it gets its programs as any sandbox does.
    manager.start(id("legacy"), None).await.unwrap();
    let view = until(&manager, "legacy", "the restarted supervisor", |view| view.state == WorkloadState::Running).await;
    assert!(view.guarantees[&Guarantee::HostFirewall], "{view:?}");
    remove(&manager, &["legacy"]).await;
    nothing_left(&before);
}

/// A detached fixture that does not accept TERM; its PID generation bounds test cleanup.
struct Orphan {
    pid: u32,
    ticks: u64,
}

fn ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let mut fields = stat.rsplit_once(") ")?.1.split_whitespace();
    if matches!(fields.next(), Some("Z" | "X")) {
        return None;
    }
    // proc_pid_stat(5): starttime is field 22, after state and 18 intervening fields.
    fields.nth(18)?.parse().ok()
}

impl Orphan {
    async fn start(manager: &Arc<Manager>, name: &str) -> Self {
        let code = "import os,signal,time\nr,w=os.pipe()\np=os.fork()\nif p:\n os.close(w);os.read(r,1);os.close(r);print(p);raise SystemExit(0)\nos.close(r)\nos.setsid()\nsignal.signal(signal.SIGTERM,signal.SIG_IGN)\nf=os.open('/dev/null',os.O_RDWR)\nfor target in (0,1,2): os.dup2(f,target)\nos.write(w,b'1');os.close(w)\nwhile True: time.sleep(1)";
        let pid = run(manager, name, &["python3", "-c", code]).await.parse().unwrap();
        Self { pid, ticks: ticks(pid).expect("the detached fixture lives") }
    }
    fn alive(&self) -> bool {
        ticks(self.pid) == Some(self.ticks)
    }
    fn kill(&self) {
        if let Some(pid) = rustix::process::Pid::from_raw(self.pid as i32)
            && let Ok(fd) = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())
            && self.alive()
        {
            let _ = rustix::process::pidfd_send_signal(fd, rustix::process::Signal::KILL);
        }
    }
    async fn gone(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.alive() {
            assert!(std::time::Instant::now() < deadline, "detached fixture still runs after its stop");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}

impl Drop for Orphan {
    fn drop(&mut self) {
        self.kill();
    }
}

async fn legacy_node(dir: &std::path::Path, ports: u16) -> Arc<Manager> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o711)).unwrap();
    let config = Arc::new(config(dir, &extra(ports, None)));
    let store = pryxd::store::Store::open(&config.state_dir).unwrap();
    let runtime = pryxd::runtime::process::ProcessRuntime::new(&config.state_dir, config.process.clone()).unwrap();
    let probe = pryxd::ports::bind_probe(config.network.edge_ips.clone());
    let manager = Manager::new(config, Arc::new(runtime), store, Arc::new(pryxd::metrics::Metrics::new()), probe);
    manager.refresh_host_probe().await;
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    manager
}

/// Reads only the test fixture's PID generation from its private runtime journal.
fn supervisor(dir: &std::path::Path, name: &str) -> Orphan {
    for path in std::fs::read_dir(dir.join("process-runtime")).unwrap().flatten().map(|entry| entry.path()) {
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        if value["spec"]["agent"]["sandbox"] == name {
            return Orphan { pid: value["pid"].as_u64().unwrap() as u32, ticks: value["ticks"].as_u64().unwrap() };
        }
    }
    panic!("the fixture supervisor has a journal");
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn a_lost_legacy_supervisor_keeps_the_tier_off_when_orphans_cannot_be_attributed() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    let orphan = {
        let manager = legacy_node(dir.path(), 22300).await;
        manager.ensure(id("lost"), sandbox(serde_json::json!({})), Precondition::None, None).await.unwrap();
        manager.start(id("lost"), None).await.unwrap();
        until(&manager, "lost", "the legacy supervisor", |view| view.state == WorkloadState::Running).await;
        let orphan = Orphan::start(&manager, "lost").await;
        let supervisor = supervisor(&manager.config.state_dir, "lost");
        supervisor.kill();
        supervisor.gone().await;
        orphan
    };
    // An unexpected supervisor loss supplies no retirement checkpoint. The shared-UID case
    // cannot attribute its reparented child; root's dedicated UID can prove and perform cleanup.
    let configured = config(dir.path(), &extra(22300, None));
    let runtime = pryxd::runtime::configured::create(&configured).await.unwrap();
    let shared = !rustix::process::geteuid().is_root();
    let capability = runtime.kernel_network_policy().unwrap();
    assert_eq!(capability.state == pryxd::probe::State::Off, shared, "{capability:?}");
    let manager = manager_on(dir.path(), runtime, &extra(22300, None));
    manager.refresh_host_probe().await;
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    if shared {
        assert!(orphan.alive(), "the live unpoliced descendant is the reason for suppression");
        assert!(!manager.features().iter().any(|feature| feature == "host-firewall"));
        remove(&manager, &["lost"]).await;
        assert!(!manager.features().iter().any(|feature| feature == "host-firewall"), "record deletion is not cleanup");
        drop(manager);
        let runtime = pryxd::runtime::configured::create(&configured).await.unwrap();
        assert_eq!(runtime.kernel_network_policy().unwrap().state, pryxd::probe::State::Off);
    } else {
        orphan.gone().await;
        remove(&manager, &["lost"]).await;
    }
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn migration_cannot_treat_a_previously_deleted_legacy_record_as_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    let configured = config(dir.path(), &extra(22400, None));
    let orphan = {
        let manager = legacy_node(dir.path(), 22400).await;
        manager.ensure(id("deleted"), sandbox(serde_json::json!({})), Precondition::None, None).await.unwrap();
        manager.start(id("deleted"), None).await.unwrap();
        until(&manager, "deleted", "the legacy supervisor", |view| view.state == WorkloadState::Running).await;
        let orphan = Orphan::start(&manager, "deleted").await;
        let supervisor = supervisor(&manager.config.state_dir, "deleted");
        supervisor.kill();
        supervisor.gone().await;
        orphan
    };
    let history = configured.state_dir.join("process-runtime");
    // Emulate an earlier daemon that removed its record without tracking escaped descendants.
    for entry in std::fs::read_dir(&history).unwrap().flatten() {
        if entry.path().extension().is_some_and(|extension| extension == "json") {
            std::fs::remove_file(entry.path()).unwrap();
        }
    }
    std::fs::remove_file(history.join("network-history-origin")).unwrap();
    let runtime = pryxd::runtime::configured::create(&configured).await.unwrap();
    assert!(orphan.alive(), "no surviving journal can identify the escaped fixture");
    assert_eq!(runtime.kernel_network_policy().unwrap().state, pryxd::probe::State::Off);
    drop(runtime);
    let runtime = pryxd::runtime::configured::create(&configured).await.unwrap();
    assert_eq!(runtime.kernel_network_policy().unwrap().state, pryxd::probe::State::Off);
    orphan.kill();
    orphan.gone().await;
    nothing_left(&before);
}
