// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The process tier's two modes against the real agent and the real helper. CI runs it twice:
//! on a hosted runner that allows user namespaces (`PRYX_TEST_NAMESPACES=on`), where each
//! sandbox must start in its own namespaces, and inside a default `docker run`
//! (`PRYX_TEST_NAMESPACES=off`), whose seccomp profile refuses them, where the tier must be
//! exactly the plain mode and say so. `PRYX_TEST_PROCESS_AGENT` names an installed agent with the
//! helper staged beside it (deploy/namespace-helper.py). With `PRYX_TEST_NAMESPACE_PROFILE` set,
//! a host where the probe fails gets the AppArmor profile the doctor prints written there.
mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pryxd::manager::{Manager, Precondition, Records};
use pryxd::protocol::{DataDisposition, ExecRequest, Guarantee, WorkloadSpec, WorkloadState};
use pryxd::runtime::ContainerRuntime;
use pryxd::runtime::process::ProcessRuntime;
use support::id;

fn agent() -> PathBuf {
    PathBuf::from(std::env::var_os("PRYX_TEST_PROCESS_AGENT").expect("PRYX_TEST_PROCESS_AGENT names the agent"))
}

fn sandbox(script: &str, network: &str) -> WorkloadSpec {
    serde_json::from_value(serde_json::json!({
        "image": "",
        "entrypoint": ["/bin/sh", "-c", script],
        "resources": { "memoryMb": 256 },
        "storage": { "mountPath": "/data", "sizeGb": 1 },
        "network": network,
        "stop": { "timeoutSeconds": 10 }
    }))
    .unwrap()
}

async fn run(m: &Arc<Manager>, w: &str, script: &str) -> String {
    let request = ExecRequest { command: vec!["/bin/sh".into(), "-c".into(), script.into()], timeout_seconds: 20 };
    let out = m.exec(id(w), request, None, None).await.unwrap();
    assert_eq!(out.exit_code, Some(0), "{w}: {script}: {out:?}");
    out.stdout
}

/// The host PID of the first process whose arguments, from the host's view, satisfy `matches`.
fn host_pid(matches: impl Fn(&[String]) -> bool) -> u32 {
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(bytes) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let argv: Vec<String> = bytes
            .split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect();
        if matches(&argv) {
            return pid;
        }
    }
    panic!("no such process on the host");
}

/// The supervisor of sandbox `w` itself, not the helper whose arguments name it too.
fn supervisor(w: &str) -> u32 {
    host_pid(|argv| {
        argv.first().is_some_and(|zero| zero.ends_with("pryx-agent")) && argv.windows(2).any(|p| p == ["--sandbox", w])
    })
}

/// Which namespace of `kind` the host process `pid` is in, as the kernel names it.
fn namespace(pid: u32, kind: &str) -> PathBuf {
    std::fs::read_link(format!("/proc/{pid}/ns/{kind}")).unwrap()
}

#[tokio::test]
#[ignore = "needs Landlock, the built agent and the staged helper; CI runs it in both modes"]
async fn process_sandboxes_get_namespaces_exactly_where_the_host_proves_them() {
    let expected = std::env::var("PRYX_TEST_NAMESPACES").unwrap_or_default();
    let agent = agent();
    let dir = tempfile::tempdir().unwrap();
    let extra =
        format!("runtime = 'process'\n[workloads]\nagent_binary = '{}'\ndefault_spare_ports = 0\n", agent.display());
    let config = support::config(dir.path(), &extra);
    std::fs::create_dir_all(&config.state_dir).unwrap();
    let process = ProcessRuntime::new(&config.state_dir, config.process.clone()).unwrap();
    let probed = process.process_namespaces(&agent).await;
    if let Some(path) = std::env::var_os("PRYX_TEST_NAMESPACE_PROFILE") {
        // Only the probe: what the doctor would tell root to install, where the helper is refused.
        if let Err(problem) = &probed {
            let helper = agent.parent().unwrap().join(pryxd::runtime::process::namespaces::HELPER);
            let profile = pryxd::runtime::process::namespaces::profile(&helper);
            assert!(pryxd::runtime::process::namespaces::action(&agent, true).ends_with(&profile));
            std::fs::write(path, profile).unwrap();
            eprintln!("NAMESPACES_OFF {problem}");
        }
        return;
    }
    let namespaced = match expected.as_str() {
        "on" => {
            probed.as_ref().expect("this runner allows user namespaces");
            true
        }
        "off" => {
            assert!(probed.is_err(), "a default docker run refuses user namespaces");
            false
        }
        other => panic!("PRYX_TEST_NAMESPACES must be on or off, not {other:?}"),
    };
    let runtime: Arc<dyn ContainerRuntime> = Arc::new(process);
    let m = support::manager_on(dir.path(), runtime, &extra);
    assert!(m.reconcile(Records::FromDisk).await.error.is_none());
    m.refresh_host_probe().await;
    let features = m.features();
    assert_eq!(features.iter().any(|f| f == "tier-process:private-pids"), namespaced, "{features:?}");

    let neighbour = "echo neighbour-secret > /tmp/neighbour-marker 2>/dev/null; exec sleep 600";
    m.ensure(id("a"), sandbox(neighbour, "internet"), Precondition::None, None).await.unwrap();
    m.ensure(id("b"), sandbox("exec sleep 601", "none"), Precondition::None, None).await.unwrap();
    m.start(id("a"), None).await.unwrap();
    m.start(id("b"), None).await.unwrap();
    let (a, b) = (m.view(&id("a")).unwrap(), m.view(&id("b")).unwrap());
    assert_eq!(a.guarantees[&Guarantee::PrivatePids], namespaced);
    assert!(!a.guarantees[&Guarantee::PrivateNetwork], "an internet sandbox shares the host's network");
    assert_eq!(b.guarantees[&Guarantee::PrivatePids], namespaced);
    assert_eq!(b.guarantees[&Guarantee::PrivateNetwork], namespaced, "network none has its own only when namespaced");

    // From the host's own view: each supervisor's namespaces, and b's network, are its own.
    let me = std::process::id();
    let (agent_a, agent_b) = (supervisor("a"), supervisor("b"));
    for kind in ["pid", "mnt", "ipc", "user"] {
        assert_eq!(namespace(agent_a, kind) != namespace(me, kind), namespaced, "a's {kind}");
        assert_eq!(namespace(agent_b, kind) != namespace(me, kind), namespaced, "b's {kind}");
        if namespaced {
            assert_ne!(namespace(agent_a, kind), namespace(agent_b, kind), "{kind} is per sandbox");
        }
    }
    assert_eq!(namespace(agent_a, "net"), namespace(me, "net"), "internet shares the host's network");
    assert_eq!(namespace(agent_b, "net") != namespace(me, "net"), namespaced, "network none's own network");
    // From inside b: whether a's process exists at all, by the path any process could stat.
    let sleeper = host_pid(|argv| argv == ["sleep", "600"]);
    let seen = run(&m, "b", &format!("test -e /proc/{sleeper} && echo visible || echo hidden")).await;
    assert_eq!(seen.trim(), if namespaced { "hidden" } else { "visible" }, "a's process from b");
    if namespaced {
        assert_eq!(run(&m, "a", "cat /tmp/neighbour-marker").await.trim(), "neighbour-secret");
        let listing = run(&m, "b", "ls -A /tmp; test -e /tmp/neighbour-marker && echo seen || echo unseen").await;
        assert_eq!(listing.trim(), "unseen", "b's /tmp is its own");
        assert!(!std::path::Path::new("/tmp/neighbour-marker").exists(), "and a's is not the host's");
    }

    // Sleep keeps the processes, wake resumes them, and a stop still reaches the supervisor.
    let before = m.view(&id("a")).unwrap().started_at;
    assert_eq!(m.suspend(id("a"), None).await.unwrap().workload.state, WorkloadState::Suspended);
    let woke = m.start(id("a"), None).await.unwrap();
    assert_eq!(woke.resumed, Some(true));
    assert_eq!(woke.workload.started_at, before, "the same processes, never started again");
    let started = std::time::Instant::now();
    let stopped = m.stop(id("b"), None, None).await.unwrap();
    assert!(!stopped.forced, "the supervisor handled its stop: {stopped:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(stopped.workload.state, WorkloadState::Stopped);
    for w in ["a", "b"] {
        m.delete(id(w), DataDisposition::Delete, Some(w.to_owned()), None).await.unwrap();
    }
    eprintln!("PROCESS_NAMESPACES mode={}", if namespaced { "namespaced" } else { "plain" });
}

/// Nearest-rank p50 and p90 of `samples`, in milliseconds.
fn percentiles(mut samples: Vec<f64>) -> (f64, f64) {
    samples.sort_by(f64::total_cmp);
    let rank = |p: f64| samples[((p * samples.len() as f64).ceil() as usize).clamp(1, samples.len()) - 1];
    (rank(0.5), rank(0.9))
}

/// K26's process figures for docs/tiers.md, in whichever mode this host gives: thirty samples each
/// of create to running, an agent exec of `true` and a syscall-heavy command, as `K26_MEASURE`.
#[tokio::test]
#[ignore = "needs Landlock, the built agent and the staged helper; CI runs it in both modes"]
async fn process_figures_in_the_mode_the_host_gives() {
    if std::env::var_os("PRYX_TEST_NAMESPACE_PROFILE").is_some() {
        return;
    }
    let agent = agent();
    let dir = tempfile::tempdir().unwrap();
    let extra =
        format!("runtime = 'process'\n[workloads]\nagent_binary = '{}'\ndefault_spare_ports = 0\n", agent.display());
    let config = support::config(dir.path(), &extra);
    std::fs::create_dir_all(&config.state_dir).unwrap();
    let process = ProcessRuntime::new(&config.state_dir, config.process.clone()).unwrap();
    let mode = if process.process_namespaces(&agent).await.is_ok() { "namespaced" } else { "plain" };
    let m = support::manager_on(dir.path(), Arc::new(process), &extra);
    assert!(m.reconcile(Records::FromDisk).await.error.is_none());
    let (mut create, mut exec, mut syscalls) = (Vec::new(), Vec::new(), Vec::new());
    for sample in 0..30 {
        let w = format!("m{sample}");
        let began = std::time::Instant::now();
        m.ensure(id(&w), sandbox("exec sleep 600", "none"), Precondition::None, None).await.unwrap();
        m.start(id(&w), None).await.unwrap();
        create.push(began.elapsed().as_secs_f64() * 1000.0);
        let began = std::time::Instant::now();
        let out = m.exec(id(&w), ExecRequest { command: vec!["true".into()], timeout_seconds: 20 }, None, None);
        assert_eq!(out.await.unwrap().exit_code, Some(0));
        exec.push(began.elapsed().as_secs_f64() * 1000.0);
        let began = std::time::Instant::now();
        run(&m, &w, "dd if=/dev/zero of=/dev/null bs=1 count=200000 2>/dev/null").await;
        syscalls.push(began.elapsed().as_secs_f64() * 1000.0);
        m.delete(id(&w), DataDisposition::Delete, Some(w.clone()), None).await.unwrap();
    }
    for (metric, samples) in [("create", create), ("exec-true", exec), ("syscall-heavy", syscalls)] {
        let (p50, p90) = percentiles(samples);
        eprintln!("K26_MEASURE tier=process mode={mode} metric={metric} n=30 p50={p50:.2} p90={p90:.2}");
    }
}
