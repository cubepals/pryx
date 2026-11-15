// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real KVM acceptance for memory sleep through the runtime seam: the start test that turns it on,
//! a process that keeps running across thirty timed sleeps and wakes, the guest's clock stepped to
//! the host's, a restore made to fail and a wake on another host class, each followed by the cold
//! boot a node makes then, with the data written before the sleep intact.
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
    runtime.probe_memory_sleep_in_background();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    while !runtime.info().await.unwrap().memory_sleep {
        let outcome = runtime.state.memory_sleep.lock().unwrap().clone();
        assert!(!matches!(outcome, Some(Err(_))), "The restore test failed: {outcome:?}");
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
