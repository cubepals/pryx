// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real KVM acceptance for memory sleep through the runtime seam: the start test that turns it on,
//! a process that keeps running across thirty timed sleeps and wakes, the guest's clock stepped to
//! the host's, a restore made to fail and a wake on another host class, each followed by the cold
//! boot a node makes then, with the data written before the sleep intact; and two forks with
//! memory that run the source's process under their own identities and random state.
use super::tests::{artifact_server, exec, spec};
use super::*;
use crate::runtime::{ContainerRuntime, ContainerStatus};

const SAMPLES: usize = 30;
// Counts in a tmpfs file, which only the guest's memory keeps; its shell's PID says it's the same.
const COUNTER: &str = "echo $$ > /tmp/pid; i=0; while :; do i=$((i+1)); echo $i > /tmp/counter; sleep 0.05; done";

fn percentile(samples: &mut [f64], fraction: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[((samples.len() - 1) as f64 * fraction).round() as usize]
}

async fn counter(runtime: &VmRuntime, id: &str) -> u64 {
    String::from_utf8(exec(runtime, id, "cat /tmp/counter").await).unwrap().trim().parse().unwrap()
}

async fn process(runtime: &VmRuntime, id: &str) -> String {
    // proc_pid_stat(5): field 22 is the process's start time since boot.
    String::from_utf8(exec(runtime, id, "p=$(cat /tmp/pid); echo $p $(cut -d' ' -f22 /proc/$p/stat)").await).unwrap()
}

/// The redacted monitor output this test's guests left (diagnostics.rs), into the job's log.
fn monitor_output(since: std::time::SystemTime) {
    let Some(directory) = std::env::var_os("PRYX_VM_RUNTIME_DIAGNOSTICS") else { return };
    for entry in fs::read_dir(directory).into_iter().flatten().flatten() {
        if entry.metadata().and_then(|info| info.modified()).is_ok_and(|modified| modified >= since) {
            eprintln!(
                "VM monitor output {}:\n{}",
                entry.path().display(),
                fs::read_to_string(entry.path()).unwrap_or_default()
            );
        }
    }
}

/// A wake the runtime refused, followed by what a node does then: a kill and a cold boot.
async fn cold_boot_after(runtime: &VmRuntime, id: &str, refused: Result<(), RuntimeError>) {
    let problem = refused.expect_err("This wake must not restore memory");
    eprintln!("VM memory wake refused as expected: {problem}");
    assert!(!problem.is_unavailable(), "A refused restore must leave the node free to boot the disk");
    assert_ne!(runtime.inspect(id).await.unwrap().unwrap().status, ContainerStatus::Paused);
    runtime.kill(id).await.unwrap();
    runtime.start(id).await.unwrap();
    assert_eq!(runtime.inspect(id).await.unwrap().unwrap().status, ContainerStatus::Running);
    assert_eq!(exec(runtime, id, "cat /data/durable").await, b"written before sleeping");
    assert_eq!(exec(runtime, id, "test -e /tmp/marker && echo memory || echo cold").await, b"cold\n");
}

#[tokio::test]
#[ignore = "requires real KVM and the verified VM archive; the VM experiments workflow runs it"]
async fn actual_memory_sleep_keeps_a_running_process_and_falls_back_to_its_disk() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let temporary = tempfile::tempdir().unwrap();
    let (artifact, server) = artifact_server().await;
    let settings = Settings {
        monitor_owner: None,
        artifact,
        ingress_connections: 8,
        free_floor_bytes: 0,
        image_timeout: Duration::from_secs(120),
    };
    let runtime = VmRuntime::new(temporary.path(), settings).await.unwrap();
    assert!(!runtime.info().await.unwrap().memory_sleep, "Memory sleep is off until its start test passes");
    let since = std::time::SystemTime::now();
    runtime.probe_memory_sleep_in_background();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    while !runtime.info().await.unwrap().memory_sleep {
        let outcome = runtime.state.memory_sleep.lock().unwrap().clone();
        if matches!(outcome, Some(Err(_))) {
            monitor_output(since);
            panic!("The restore test failed: {outcome:?}");
        }
        assert!(tokio::time::Instant::now() < deadline, "The restore test did not finish");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let inputs: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../../templates/vm-tools/image-inputs.json")).unwrap();
    let data = temporary.path().join("data");
    directory(&data).unwrap();
    let mut original = spec(&data, inputs[0]["reference"].as_str().unwrap());
    original.agent.as_mut().unwrap().keep_alive = false;
    original.entrypoint = Some(vec!["/bin/sh".into(), "-c".into(), COUNTER.into()]);
    let id = runtime.create(&original).await.unwrap();
    runtime.start(&id).await.unwrap();
    while exec(&runtime, &id, "test -s /tmp/counter && echo yes || echo no").await != b"yes\n" {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    exec(&runtime, &id, "printf kept > /tmp/marker").await;
    let running = process(&runtime, &id).await;
    let (mut sleeps, mut wakes) = (Vec::new(), Vec::new());
    for sample in 0..SAMPLES {
        let before = counter(&runtime, &id).await;
        let started = std::time::Instant::now();
        runtime.pause(&id).await.unwrap();
        sleeps.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(runtime.inspect(&id).await.unwrap().unwrap().status, ContainerStatus::Paused);
        let started = std::time::Instant::now();
        runtime.unpause(&id).await.unwrap();
        wakes.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(runtime.inspect(&id).await.unwrap().unwrap().status, ContainerStatus::Running);
        assert_eq!(process(&runtime, &id).await, running, "Sample {sample}: the same process runs on");
        assert_eq!(exec(&runtime, &id, "cat /tmp/marker").await, b"kept");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let after = counter(&runtime, &id).await;
        assert!(after > before, "Sample {sample}: the counter went on from {before} to {after}");
    }
    let memory = runtime.state.root.join("memory");
    assert!(fs::read_dir(&memory).unwrap().next().is_none(), "A woken guest's memory image is gone");
    let (sleep_p50, sleep_p90) = (percentile(&mut sleeps, 0.5), percentile(&mut sleeps, 0.9));
    let (wake_p50, wake_p90) = (percentile(&mut wakes, 0.5), percentile(&mut wakes, 0.9));
    println!(
        "VM memory sleep samples ({SAMPLES}, 256 MiB, Alpine, a running process): sleep p50 {sleep_p50:.2} / p90 {sleep_p90:.2} ms, wake p50 {wake_p50:.2} / p90 {wake_p90:.2} ms"
    );

    // A wake steps the guest's wall clock to the host's, though its memory says otherwise.
    runtime.pause(&id).await.unwrap();
    tokio::time::sleep(Duration::from_secs(5)).await;
    runtime.unpause(&id).await.unwrap();
    let guest: i64 = String::from_utf8(exec(&runtime, &id, "date +%s").await).unwrap().trim().parse().unwrap();
    let host = crate::clock::now().unix_timestamp();
    assert!((guest - host).abs() <= 2, "The guest's clock {guest} is the host's {host}");

    // Written and not synced: the guest's own flush before its snapshot puts it on the disk.
    exec(&runtime, &id, "printf 'written before sleeping' > /data/durable").await;
    runtime.pause(&id).await.unwrap();
    let state = fs::read_dir(&memory).unwrap().next().unwrap().unwrap().path().join("state");
    fs::write(&state, b"not a monitor state").unwrap();
    let refused = runtime.unpause(&id).await;
    cold_boot_after(&runtime, &id, refused).await;
    assert!(fs::read_dir(&memory).unwrap().next().is_none(), "A refused image is discarded");

    // Memory taken on a host of another class (here, another kernel) is never restored.
    exec(&runtime, &id, "printf kept > /tmp/marker").await;
    runtime.pause(&id).await.unwrap();
    {
        let mut entries = runtime.state.entries.lock().unwrap();
        let entry = entries.get_mut(&id).unwrap();
        entry.memory.as_mut().unwrap().class.kernel.push_str("-another");
    }
    let refused = runtime.unpause(&id).await;
    cold_boot_after(&runtime, &id, refused).await;

    // Stopping a sleeping guest ends its memory as a kill does, and keeps its disk.
    runtime.pause(&id).await.unwrap();
    runtime.stop(&id, &original.stop_signal, Duration::from_secs(5)).await.unwrap();
    let stopped = runtime.inspect(&id).await.unwrap().unwrap();
    assert_eq!(stopped.status, ContainerStatus::Exited);
    assert_eq!(stopped.exit_code, crate::runtime::exit_by(libc::SIGKILL));
    assert!(fs::read_dir(&memory).unwrap().next().is_none());
    runtime.start(&id).await.unwrap();
    assert_eq!(exec(&runtime, &id, "cat /data/durable").await, b"written before sleeping");
    runtime.remove(&id).await.unwrap();
    server.abort();
}

/// One shell line's output in `id`'s guest, trimmed.
async fn read(runtime: &VmRuntime, id: &str, script: &str) -> String {
    String::from_utf8(exec(runtime, id, script).await).unwrap().trim().to_owned()
}

/// A source with a counting process, asleep with its memory, and two forks made from it.
async fn source_and_forks(
    runtime: &VmRuntime,
    root: &Path,
    image: &str,
) -> ((String, ContainerSpec), Vec<(String, ContainerSpec)>, String) {
    let with_counter = |mut spec: ContainerSpec| {
        spec.agent.as_mut().unwrap().keep_alive = false;
        spec.entrypoint = Some(vec!["/bin/sh".into(), "-c".into(), COUNTER.into()]);
        spec
    };
    let data = root.join("source");
    directory(&data).unwrap();
    let source_spec = with_counter(spec(&data, image));
    let source = runtime.create(&source_spec).await.unwrap();
    runtime.start(&source).await.unwrap();
    while read(runtime, &source, "test -s /tmp/counter && echo yes || echo no").await != "yes" {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    exec(runtime, &source, "printf kept > /tmp/marker").await;
    let running = process(runtime, &source).await;
    runtime.pause(&source).await.unwrap();
    let mut forks = Vec::new();
    for name in ["one", "two"] {
        let fork_spec = with_counter(spec(&root.join(format!("fork-{name}")), image));
        let started = std::time::Instant::now();
        let fork = runtime.fork_with_memory(&source, &fork_spec).await.unwrap();
        eprintln!("VM fork with memory {name} made in {:.2} ms", started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(runtime.inspect(&fork).await.unwrap().unwrap().status, ContainerStatus::Paused);
        forks.push((fork, fork_spec));
    }
    ((source, source_spec), forks, running)
}

/// Each woken guest runs the source's very process and memory, under its own hostname and with its
/// own kernel random bytes, read first thing after its wake.
async fn own_identities(runtime: &VmRuntime, woken: &[String], running: &str) {
    let mut seen = Vec::new();
    for id in woken {
        let random = read(runtime, id, "head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \\n'").await;
        let hostname = read(runtime, id, "hostname").await;
        let generation = read(runtime, id, "ls /sys/bus/acpi/drivers 2>/dev/null | grep -ci vmgenid || true").await;
        eprintln!("VM guest {id}: hostname {hostname}, VM generation ID drivers {generation}, random {random}");
        assert_eq!(process(runtime, id).await, running, "{id} runs the source's very process");
        assert_eq!(read(runtime, id, "cat /tmp/marker").await, "kept");
        seen.push((hostname, random));
    }
    for (index, (hostname, random)) in seen.iter().enumerate() {
        for (other_hostname, other_random) in &seen[index + 1..] {
            assert_ne!(hostname, other_hostname, "Every fork has its own hostname");
            assert_ne!(random, other_random, "Every fork's kernel draws its own random bytes after its wake");
        }
    }
}

/// A fork answers its own agent key, refuses its source's, and stops and goes as any sandbox.
async fn retire_fork(runtime: &VmRuntime, fork: &str, fork_spec: &ContainerSpec, source_spec: &ContainerSpec) {
    let mut impostor = source_spec.clone();
    impostor.ports = fork_spec.ports.clone();
    let refused = super::agent::raw(
        &impostor,
        hyper::Method::GET,
        "/_pryx/health",
        serde_json::Value::Null,
        Duration::from_secs(5),
    )
    .await;
    assert!(refused.is_err(), "{fork} refuses its source's token");
    assert!(read(runtime, fork, "cat /tmp/counter").await.parse::<u64>().unwrap() > 0);
    runtime.stop(fork, &fork_spec.stop_signal, Duration::from_secs(5)).await.unwrap();
    assert_eq!(runtime.inspect(fork).await.unwrap().unwrap().status, ContainerStatus::Exited);
    runtime.remove(fork).await.unwrap();
}

#[tokio::test]
#[ignore = "requires real KVM and the verified VM archive; the VM experiments workflow runs it"]
async fn actual_forks_with_memory_wake_as_themselves() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let temporary = tempfile::tempdir().unwrap();
    let (artifact, server) = artifact_server().await;
    let settings = Settings {
        monitor_owner: None,
        artifact,
        ingress_connections: 8,
        free_floor_bytes: 0,
        image_timeout: Duration::from_secs(120),
    };
    let runtime = VmRuntime::new(temporary.path(), settings).await.unwrap();
    let inputs: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../../templates/vm-tools/image-inputs.json")).unwrap();
    let image = inputs[0]["reference"].as_str().unwrap();
    let ((source, source_spec), forks, running) = source_and_forks(&runtime, temporary.path(), image).await;
    let mut woken = Vec::new();
    for (fork, _) in &forks {
        let started = std::time::Instant::now();
        runtime.unpause(fork).await.unwrap();
        eprintln!("VM fork {fork} woke in {:.2} ms", started.elapsed().as_secs_f64() * 1000.0);
        woken.push(fork.clone());
    }
    runtime.unpause(&source).await.unwrap();
    woken.push(source.clone());
    own_identities(&runtime, &woken, &running).await;
    for (fork, fork_spec) in &forks {
        retire_fork(&runtime, fork, fork_spec, &source_spec).await;
    }
    runtime.stop(&source, &source_spec.stop_signal, Duration::from_secs(5)).await.unwrap();
    runtime.remove(&source).await.unwrap();
    assert!(fs::read_dir(runtime.state.root.join("memory")).unwrap().next().is_none());
    server.abort();
}
