// SPDX-License-Identifier: FSL-1.1-ALv2
//! Process-owned network adoption, incomplete launches and checkpoint observations. Kernel packet
//! parsing and route-only policy fixtures belong to `network/guard_tests.rs`.
use super::{
    Entry, ProcessRuntime, Served,
    network::{Network, Record, error, fixture_spec},
    observe,
};
use crate::{
    bpf::{
        host::{self, Host},
        object,
    },
    runtime::ContainerRuntime,
};
use std::process::Command;

/// These checkpoint tests launch only a bounded sleeper, and retire it even after a failed
/// assertion. The runtime's actual PID checkpoint remains the source of its process identity.
struct Children(ProcessRuntime);

impl Drop for Children {
    fn drop(&mut self) {
        let mut children = self.0.state.children.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for child in children.values_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        children.clear();
    }
}

#[test]
fn an_unverified_cgroup_stays_pending_without_a_supervisor_or_object_record() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = ProcessRuntime::new(root.path(), crate::config::ProcessConfig::default()).unwrap();
    let cgroups = root.path().join("cgroups");
    std::fs::create_dir(&cgroups).unwrap();
    let network = std::sync::Arc::new(Network::new(
        Host { cgroups: cgroups.clone(), pins: root.path().join("pins"), delegated: false },
        Served { ports: 42000..=42999, api_port: 7443 },
        &runtime.state.root,
    ));
    runtime.network = Some(network.clone());
    runtime.network_capability = Some(crate::capability::Capability::on("fixture host proof"));
    let spec = fixture_spec(root.path());
    let entry: Entry = serde_json::from_value(serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(), "spec": spec, "pid": null, "ticks": null,
        "code": 0, "started": null, "finished": null
    }))
    .unwrap();
    let (key, cgroup) = network.fixture_paths(&entry.spec.name);
    for record in
        [None, Some(Record { key: key.clone(), attachments: Vec::new(), object_sha256: "previous-object".into() })]
    {
        let mut generation = entry.clone();
        generation.network = record;
        runtime.save(generation).unwrap();
        assert!(!network.pending(&entry.spec.name));
        assert_eq!(runtime.unpoliced(), 0);
        std::fs::create_dir(&cgroup).unwrap();
        // No members are required: a paused pre-exec child might still join this directory.
        assert!(network.pending(&entry.spec.name));
        assert_eq!(runtime.unpoliced(), 1);
        assert_eq!(runtime.kernel_network_policy().unwrap().state, crate::capability::State::Off);
        std::fs::remove_dir(&cgroup).unwrap();
        assert_eq!(runtime.unpoliced(), 0);
    }
    // ENOTDIR, like EACCES, is uncertainty rather than a successful absence proof.
    std::fs::remove_dir(&cgroups).unwrap();
    std::fs::write(&cgroups, b"not a cgroup directory").unwrap();
    assert!(network.pending(&entry.spec.name));
    assert_eq!(runtime.unpoliced(), 1);
}

#[tokio::test]
async fn unavailable_cleanup_cannot_delete_a_recorded_cgroup_owner() {
    let root = tempfile::tempdir().unwrap();
    let runtime = ProcessRuntime::new(root.path(), crate::config::ProcessConfig::default()).unwrap();
    let entry: Entry = serde_json::from_value(serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(), "spec": fixture_spec(root.path()),
        "pid": null, "ticks": null, "code": 0, "started": null, "finished": null,
        "network": { "key": "previous-key", "attachments": [], "object_sha256": "previous-object" }
    }))
    .unwrap();
    runtime.save(entry.clone()).unwrap();
    let refused = runtime.remove(&entry.spec.name).await.unwrap_err();
    assert!(refused.to_string().contains("cleanup cannot be proved"), "{refused}");
    assert!(runtime.entry(&entry.spec.name).unwrap().network.is_some());
    assert!(runtime.state.root.join(format!("{}.json", entry.id)).is_file());
}

#[test]
fn a_plain_launch_intent_survives_pid_loss_restart_and_record_deletion() {
    let root = tempfile::tempdir().unwrap();
    let policy = crate::config::ProcessConfig::default();
    let runtime = ProcessRuntime::new(root.path(), policy.clone()).unwrap();
    let mut entry: Entry = serde_json::from_value(serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(), "spec": fixture_spec(root.path()),
        "pid": null, "ticks": null, "code": 0, "started": null, "finished": null
    }))
    .unwrap();
    // A genuinely never-attempted record is still exempt, including after reload.
    assert!(!runtime.legacy_pending(&entry));
    runtime.save(entry.clone()).unwrap();
    drop(runtime);
    let runtime = ProcessRuntime::new(root.path(), policy.clone()).unwrap();
    runtime.legacy_history().unwrap();
    assert!(!runtime.legacy_boot_uncertain());
    // The plain-mode pre-fork checkpoint exists but its daemon never saved a PID. No cgroup
    // or shared-UID scan can attribute a possible detached child to this lost supervisor.
    entry.network_boot = crate::host::boot_id();
    entry.network_launch_pending = true;
    runtime.save(entry.clone()).unwrap();
    drop(runtime);
    let mut runtime = ProcessRuntime::new(root.path(), policy.clone()).unwrap();
    let lost = runtime.entry(&entry.spec.name).unwrap();
    assert!(lost.network.is_none() && lost.pid.is_none() && lost.started.is_none());
    assert!(runtime.legacy_pending(&lost));
    runtime.legacy_history().unwrap();
    runtime.network_capability = Some(crate::capability::Capability::on("fixture later host proof"));
    assert_eq!(runtime.kernel_network_policy().unwrap().state, crate::capability::State::Off);
    runtime.remove_entry(&entry.spec.name).unwrap();
    drop(runtime);
    let mut runtime = ProcessRuntime::new(root.path(), policy).unwrap();
    assert!(runtime.names().is_empty());
    runtime.network_capability = Some(crate::capability::Capability::on("fixture later host proof"));
    assert_eq!(runtime.kernel_network_policy().unwrap().state, crate::capability::State::Off);
}

#[test]
#[ignore = "requires a dedicated UID and an unfiltered host process table"]
fn an_empty_dedicated_uid_cannot_retire_a_child_before_its_uid_drop() {
    if !rustix::process::geteuid().is_root() {
        eprintln!("root proves dedicated-UID retirement of incomplete launch intents");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let runtime = ProcessRuntime::new(root.path(), crate::config::ProcessConfig::default()).unwrap();
    assert!(!runtime.shared_uid());
    let (uid, _) = runtime.owner("route-guard").unwrap();
    assert!(observe::uid_processes_proven(uid).unwrap().is_empty());
    let mut entry: Entry = serde_json::from_value(serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(), "spec": fixture_spec(root.path()),
        "pid": null, "ticks": null, "code": 0, "started": null, "finished": null
    }))
    .unwrap();
    entry.spec.agent = Some(crate::runtime::AgentBoot {
        public_origin: None,
        binary: root.path().join("not-executed"),
        sandbox: "route-guard".into(),
        epoch: 1,
        secret_env: "FIXTURE_ONLY".into(),
        domain: "pryx.test".into(),
        agent_port: pryx_agent::contract::AGENT_PORT,
        spare_ports: Vec::new(),
        keep_alive: true,
    });
    assert!(!runtime.legacy_pending(&entry), "a never-attempted record remains exempt");
    entry.network_boot = crate::host::boot_id();
    entry.network_launch_pending = true;
    assert!(runtime.legacy_pending(&entry), "the child can still be paused before uid(owner)");
    entry.network_launch_pending = false;
    assert!(!runtime.legacy_pending(&entry), "completed generations retain the dedicated-UID empty proof");
    runtime.release_owner("route-guard").unwrap();
}

#[test]
fn observing_plain_cleanup_does_not_relock_the_entry_mutex() {
    let root = tempfile::tempdir().unwrap();
    let runtime = ProcessRuntime::new(root.path(), crate::config::ProcessConfig::default()).unwrap();
    let entry: Entry = serde_json::from_value(serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(), "spec": fixture_spec(root.path()),
        "pid": null, "ticks": null, "code": 0, "started": null, "finished": null,
        "cleaned": false
    }))
    .unwrap();
    runtime.save(entry.clone()).unwrap();
    let (sent, received) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sent.send(runtime.observe_entry(&entry.spec.name).map(|view| view.is_some())).unwrap();
    });
    let observed =
        received.recv_timeout(std::time::Duration::from_secs(2)).expect("plain cleanup must not deadlock").unwrap();
    assert!(observed);
    worker.join().unwrap();
}

#[test]
fn an_observer_waits_for_the_intent_fork_and_pid_checkpoint() {
    for policed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = ProcessRuntime::new(root.path(), crate::config::ProcessConfig::default()).unwrap();
        let mut entry: Entry = serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(), "spec": fixture_spec(root.path()),
            "pid": null, "ticks": null, "code": 0, "started": null, "finished": null,
            "cleaned": false
        }))
        .unwrap();
        let mut frozen = None;
        if policed {
            // A metadata fixture detects any attempted retirement; no BPF syscall is needed
            // to prove that an observer must not touch a newly prepared canonical cgroup.
            let network = std::sync::Arc::new(Network::new(
                Host { cgroups: root.path().join("cgroups"), pins: root.path().join("pins"), delegated: false },
                Served { ports: 42000..=42999, api_port: 7443 },
                &runtime.state.root,
            ));
            let (key, cgroup) = network.fixture_paths(&entry.spec.name);
            std::fs::create_dir_all(&cgroup).unwrap();
            let marker = cgroup.join("cgroup.freeze");
            std::fs::write(&marker, b"0").unwrap();
            frozen = Some(marker);
            entry.network = Some(Record { key, attachments: Vec::new(), object_sha256: object::digest() });
            runtime.network = Some(network);
        }
        runtime.save(entry.clone()).unwrap();
        let _children = Children(runtime.clone());
        let journal = runtime.state.root.join(format!("{}.json", entry.id));
        let observer = runtime.clone();
        let (begin, started) = std::sync::mpsc::channel();
        let (admitted, waiting) = std::sync::mpsc::channel();
        let (sent, observed) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started.recv().unwrap();
            assert!(matches!(observer.state.entries.try_lock(), Err(std::sync::TryLockError::WouldBlock)));
            admitted.send(()).unwrap();
            let running = observer.running("route-guard").unwrap();
            sent.send((running, observer.entry("route-guard").unwrap().pid)).unwrap();
        });
        let mut actual = None;
        runtime
            .spawn_checkpoint(entry, |_| {
                let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
                assert!(saved["network_boot"].is_string() && saved["pid"].is_null());
                assert_eq!(saved["network_launch_pending"], true);
                begin.send(()).unwrap();
                waiting.recv_timeout(std::time::Duration::from_secs(2)).expect("observer reached the locked intent");
                assert!(matches!(observed.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
                let child = Command::new("sleep").arg("60").spawn().map_err(error)?;
                actual = Some(child.id());
                Ok(child)
            })
            .unwrap();
        let outcome = observed.recv_timeout(std::time::Duration::from_secs(2)).expect("observer sees the new PID");
        assert_eq!(outcome, (true, actual));
        assert!(!runtime.entry("route-guard").unwrap().network_launch_pending);
        assert!(!runtime.legacy_boot_uncertain(), "a live in-progress intent was never a lost generation");
        if let Some(marker) = frozen {
            assert_eq!(std::fs::read(marker).unwrap(), b"0", "no premature cgroup retirement");
        }
        worker.join().unwrap();
    }
}

#[tokio::test]
#[ignore = "loads programs into the kernel"]
async fn a_failed_startup_proof_retires_an_orphaned_recorded_cgroup() {
    use std::os::unix::process::ExitStatusExt;
    let root = tempfile::tempdir().unwrap();
    let served = Served { ports: 42000..=42999, api_port: 7443 };
    let mut spec = fixture_spec(root.path());
    spec.name = "refresh".into();
    let sleeper = || {
        let mut command = Command::new("sleep");
        command.arg("60");
        host::drop_ambient_on_exec(&mut command);
        command
    };
    // An exited supervisor's journal can still own a live cgroup member in shared-UID
    // mode: no PID or UID sweep finds it, so a failed startup proof must fence the record.
    let runtime = ProcessRuntime::new(root.path(), crate::config::ProcessConfig::default()).unwrap();
    let host = Host::find().unwrap_or_else(|problem| panic!("{}", problem.evidence));
    let network = Network::new(host, served.clone(), &runtime.state.root);
    let (record, orphan_group) = network.prepare(&spec).await.unwrap();
    let mut orphan = network.spawn_verified(&spec.name, &mut sleeper()).unwrap();
    runtime
        .save(Entry {
            id: uuid::Uuid::new_v4().to_string(),
            spec: spec.clone(),
            pid: None,
            ticks: None,
            code: 0,
            started: None,
            finished: None,
            cleaned: false,
            native_browser: None,
            network: Some(record),
            network_boot: None,
            network_launch_pending: false,
            network_clean: false,
            network_stop_nonce: None,
            namespaced: None,
            limits: None,
        })
        .unwrap();
    drop(network);
    let rejected = aya::include_bytes_aligned!("../../../bpf/rejected.bpf.o");
    let _runtime = runtime.with_network_programs(served, rejected).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        if let Some(status) = orphan.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = orphan.kill();
            let _ = orphan.wait();
            panic!("an orphaned cgroup member outlived the failed startup proof");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    assert!(!orphan_group.exists());
}
