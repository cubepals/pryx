// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real KVM acceptance for memory sleep through the runtime seam: the start test that turns it on,
//! a process that keeps running across thirty timed sleeps and wakes, the guest's clock stepped to
//! the host's, a restore made to fail and a wake on another host class, each followed by the cold
//! boot a node makes then, with the data written before the sleep intact; two forks with memory
//! that run the source's process under their own identities and random state, its workload frozen
//! from before the sleep's flush until each resume has reseeded it; and a guest with a
//! network, public or under a hostname policy, whose DNS and allowed egress work after its wake
//! and in its fork.
use super::tests::{artifact_server, exec, spec};
use super::*;
use crate::runtime::{ContainerRuntime, ContainerStatus};

const SAMPLES: usize = 30;
// Counts in a tmpfs file, which only the guest's memory keeps; its shell's PID says it's the same.
const COUNTER: &str = "echo $$ > /tmp/pid; i=0; while :; do i=$((i+1)); echo $i > /tmp/counter; sleep 0.05; done";
// Appends 16 random bytes a line once /tmp/sample exists, which a cold boot doesn't keep. Capture
// the request before drawing: a snapshot may retain bytes already drawn but not yet appended.
const SAMPLER: &str = r#"answered=
while :; do
    if [ -e /tmp/sample ]; then
        request=$(cat /tmp/sample-request 2>/dev/null || true)
        r=$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')
        echo "$r" >> /data/random
        if [ -n "$request" ] && [ "$request" != "$answered" ]; then
            printf '%s %s\n' "$request" "$r" > /tmp/sample-response-next
            mv /tmp/sample-response-next /tmp/sample-response
            answered=$request
        fi
    fi
    sleep 0.01
done"#;

/// Only a complete acknowledgement for this request proves its sample was drawn after the request.
fn requested_sample<'a>(response: &'a str, request: &str) -> Option<&'a str> {
    let (seen, value) = response.trim().split_once(' ')?;
    (seen == request && value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(value)
}

/// Ask only after wake readiness, so matching bytes cannot have been drawn before the restore.
async fn fresh_sample(runtime: &VmRuntime, id: &str) -> String {
    let request = uuid::Uuid::new_v4().simple().to_string();
    exec(runtime, id, &format!("printf '%s' {request} > /tmp/sample-request")).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let response = read(runtime, id, "cat /tmp/sample-response 2>/dev/null || true").await;
            if let Some(value) = requested_sample(&response, &request) {
                return value.to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("The sampler must acknowledge a post-wake draw")
}

/// The fixture's complete process group is retired even if the gate-order assertion panics.
struct SamplerProcess(std::process::Child);

impl Drop for SamplerProcess {
    fn drop(&mut self) {
        let _ = rustix::process::kill_process_group(
            rustix::process::Pid::from_child(&self.0),
            rustix::process::Signal::KILL,
        );
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn sampler_reached(path: &Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("The controlled sampler must reach its gate");
}

#[tokio::test]
async fn a_buffered_sampler_draw_cannot_answer_a_later_request() {
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    for directory in ["tmp", "data", "bin"] {
        fs::create_dir(root.join(directory)).unwrap();
    }
    for flag in ["tmp/sample", "old-held", "fresh-held"] {
        fs::write(root.join(flag), b"").unwrap();
    }
    fs::write(root.join("tmp/sample-request"), b"before-request").unwrap();
    // The real sampler runs unchanged apart from fixture paths. Its head input blocks after
    // providing the old bytes, then blocks the next draw so the old acknowledgement is observable.
    let head = root.join("bin/head");
    fs::write(
        &head,
        r#"#!/bin/sh
if [ ! -e "$SAMPLER_FIXTURE/old-drawn" ]; then
    printf 0123456789abcdef
    touch "$SAMPLER_FIXTURE/old-drawn"
    while [ -e "$SAMPLER_FIXTURE/old-held" ]; do sleep 0.01; done
else
    touch "$SAMPLER_FIXTURE/fresh-drawn"
    while [ -e "$SAMPLER_FIXTURE/fresh-held" ]; do sleep 0.01; done
    printf fedcba9876543210
fi
"#,
    )
    .unwrap();
    fs::set_permissions(&head, fs::Permissions::from_mode(0o700)).unwrap();
    let paths = std::env::join_paths(
        std::iter::once(root.join("bin")).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())),
    )
    .unwrap();
    let script = SAMPLER
        .replace("/tmp/", &format!("{}/tmp/", root.display()))
        .replace("/data/", &format!("{}/data/", root.display()));
    let _sampler = SamplerProcess(
        std::process::Command::new("sh")
            .args(["-c", &script])
            .env("PATH", paths)
            .env("SAMPLER_FIXTURE", root)
            .process_group(0)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    sampler_reached(&root.join("old-drawn")).await;
    let request = uuid::Uuid::new_v4().simple().to_string();
    fs::write(root.join("tmp/sample-request"), &request).unwrap();
    fs::remove_file(root.join("old-held")).unwrap();
    sampler_reached(&root.join("fresh-drawn")).await;
    let response = fs::read_to_string(root.join("tmp/sample-response")).unwrap();
    assert!(
        requested_sample(&response, &request).is_none(),
        "A buffered draw must not answer a request received after that draw began"
    );
    let buffered = hex::encode(b"0123456789abcdef");
    assert_eq!(requested_sample(&response, "before-request"), Some(buffered.as_str()));
    assert_eq!(fs::read_to_string(root.join("data/random")).unwrap(), format!("{buffered}\n"));
    fs::remove_file(root.join("fresh-held")).unwrap();
    let fresh = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let response = fs::read_to_string(root.join("tmp/sample-response")).unwrap();
            if let Some(value) = requested_sample(&response, &request) {
                break value.to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("A new draw must acknowledge the new request");
    assert_eq!(fresh, hex::encode(b"fedcba9876543210"));
}

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

    bootstrap_mismatches_boot_the_disk(&runtime, &id).await;

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

/// Missing legacy bootstrap identity and a changed bootstrap both boot the disk.
async fn bootstrap_mismatches_boot_the_disk(runtime: &VmRuntime, id: &str) {
    for bootstrap in ["", "another-bootstrap"] {
        exec(runtime, id, "printf kept > /tmp/marker").await;
        runtime.pause(id).await.unwrap();
        {
            let mut entries = runtime.state.entries.lock().unwrap();
            let image = entries.get_mut(id).unwrap().memory.as_mut().unwrap();
            assert!(!image.bootstrap_sha256.is_empty(), "A new sleep records its actual bootstrap");
            image.bootstrap_sha256 = bootstrap.into();
        }
        let refused = runtime.unpause(id).await;
        cold_boot_after(runtime, id, refused).await;
    }
}

/// One shell line's output in `id`'s guest, trimmed.
async fn read(runtime: &VmRuntime, id: &str, script: &str) -> String {
    String::from_utf8(exec(runtime, id, script).await).unwrap().trim().to_owned()
}

/// A source with a counting process and a sampling one, asleep with its memory, and a fork made
/// from it for each of `names`.
async fn source_and_forks(
    runtime: &VmRuntime,
    root: &Path,
    image: &str,
    names: &[&str],
) -> ((String, ContainerSpec), Vec<(String, ContainerSpec)>, String) {
    let with_counter = |mut spec: ContainerSpec| {
        spec.agent.as_mut().unwrap().keep_alive = false;
        spec.entrypoint = Some(vec!["/bin/sh".into(), "-c".into(), format!("({SAMPLER}) & {COUNTER}")]);
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
    exec(runtime, &source, "printf kept > /tmp/marker; touch /tmp/sample").await;
    while sampled(runtime, &source).await.len() < 5 {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let running = process(runtime, &source).await;
    runtime.pause(&source).await.unwrap();
    let mut forks = Vec::new();
    for name in names {
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

/// The lines a guest's sampler appended to its disk, as that guest sees them.
async fn sampled(runtime: &VmRuntime, id: &str) -> Vec<String> {
    read(runtime, id, "cat /data/random 2>/dev/null || true").await.lines().map(str::to_owned).collect()
}

/// Every copy restored from one memory draws its own bytes on a request issued after wake readiness.
/// The disk the sleep flushed holds every line that memory had: the workload was frozen before
/// the flush, and thawed in each copy only once its resume had reseeded it. `cold`, a fork refused
/// its restore (another host class here), boots that disk.
async fn frozen_across_the_restore(runtime: &VmRuntime, woken: &[String], cold: &str, cold_spec: &ContainerSpec) {
    let mut copies = Vec::new();
    let mut fresh = Vec::new();
    for id in woken {
        fresh.push(fresh_sample(runtime, id).await);
        copies.push(sampled(runtime, id).await);
    }
    runtime.state.entries.lock().unwrap().get_mut(cold).unwrap().memory.as_mut().unwrap().class.kernel.push_str("-x");
    let refused = runtime.unpause(cold).await;
    assert!(refused.is_err_and(|problem| !problem.is_unavailable()), "Another class's memory is never restored");
    runtime.kill(cold).await.unwrap();
    runtime.start(cold).await.unwrap();
    let flushed = sampled(runtime, cold).await;
    eprintln!("VM sleep flushed {} sampled lines; the copies hold {:?}", flushed.len(), copies.iter().map(Vec::len));
    assert!(!flushed.is_empty());
    for copy in &copies {
        assert!(copy.len() > flushed.len(), "Each copy sampled again after its wake");
        assert_eq!(copy[..flushed.len()], flushed[..], "The flushed disk holds every line the memory had");
    }
    for (index, value) in fresh.iter().enumerate() {
        for other in &fresh[index + 1..] {
            assert_ne!(value, other, "Samples requested after each restore draw different bytes");
        }
    }
    runtime.stop(cold, &cold_spec.stop_signal, Duration::from_secs(5)).await.unwrap();
    runtime.remove(cold).await.unwrap();
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
    let since = std::time::SystemTime::now();
    let ((source, source_spec), mut forks, running) =
        source_and_forks(&runtime, temporary.path(), image, &["one", "two", "three"]).await;
    let (cold, cold_spec) = forks.pop().unwrap();
    let mut woken = Vec::new();
    for (fork, _) in &forks {
        let started = std::time::Instant::now();
        runtime.unpause(fork).await.inspect_err(|_| monitor_output(since)).unwrap();
        eprintln!("VM fork {fork} woke in {:.2} ms", started.elapsed().as_secs_f64() * 1000.0);
        woken.push(fork.clone());
    }
    runtime.unpause(&source).await.unwrap();
    woken.push(source.clone());
    own_identities(&runtime, &woken, &running).await;
    frozen_across_the_restore(&runtime, &woken, &cold, &cold_spec).await;
    for (fork, fork_spec) in &forks {
        retire_fork(&runtime, fork, fork_spec, &source_spec).await;
    }
    runtime.stop(&source, &source_spec.stop_signal, Duration::from_secs(5)).await.unwrap();
    runtime.remove(&source).await.unwrap();
    assert!(fs::read_dir(runtime.state.root.join("memory")).unwrap().next().is_none());
    server.abort();
}

/// A guest with the production network keeps its DNS and allowed egress across a wake with memory,
/// and its fork with memory has them too: each wake's gateways listen on new ports, which the
/// resume hands the guest's redirects.
#[tokio::test]
#[ignore = "requires real KVM, the verified VM archive and public DNS; the VM experiments workflow runs it"]
async fn actual_memory_wake_keeps_a_networked_guests_dns_and_egress() {
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
    let since = std::time::SystemTime::now();
    let host = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    for unrestricted in [true, false] {
        let (id, source_spec) =
            super::network_acceptance::start(&runtime, temporary.path(), image, 0, unrestricted, &host).await;
        super::network_acceptance::probe(&runtime, &id, unrestricted, &host).await;
        exec(&runtime, &id, "printf kept > /tmp/marker").await;
        // A managed process without an exec deadline: its owner and socket survive the sleeps.
        let retained = pryx_agent::process::Start {
            command: pryx_agent::process::Command::Shell(
                "exec /data/network-probe --retained-dns /data/network-fixture.json --ready-file /tmp/retained-dns >/tmp/retained-dns.log 2>&1".into(),
            ),
            env: Default::default(),
            cwd: None,
            stdin: false,
            pty: false,
            cols: None,
            rows: None,
            timeout: None,
        };
        super::agent::raw(
            &source_spec,
            hyper::Method::POST,
            "/_pryx/processes",
            serde_json::to_value(retained).unwrap(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        let socket = retained_dns(&runtime, &id, 0).await;
        runtime.pause(&id).await.unwrap();
        runtime.unpause(&id).await.inspect_err(|_| monitor_output(since)).unwrap();
        assert_eq!(read(&runtime, &id, "cat /tmp/marker").await, "kept", "The wake restored memory");
        assert_eq!(retained_dns(&runtime, &id, 1).await, socket, "The same UDP socket resolves after its wake");
        super::network_acceptance::probe(&runtime, &id, unrestricted, &host).await;
        runtime.pause(&id).await.unwrap();
        let data = temporary.path().join(format!("network-fork-{unrestricted}"));
        let fork_spec = super::network_acceptance::networked(spec(&data, image), unrestricted);
        let fork = runtime.fork_with_memory(&id, &fork_spec).await.unwrap();
        runtime.unpause(&fork).await.inspect_err(|_| monitor_output(since)).unwrap();
        assert_eq!(read(&runtime, &fork, "cat /tmp/marker").await, "kept", "The fork woke with memory");
        assert_eq!(retained_dns(&runtime, &fork, 2).await, socket, "The fork's retained UDP socket resolves");
        super::network_acceptance::probe(&runtime, &fork, unrestricted, &host).await;
        runtime.unpause(&id).await.unwrap();
        assert_eq!(retained_dns(&runtime, &id, 2).await, socket, "The source's retained UDP socket resolves");
        super::network_acceptance::probe(&runtime, &id, unrestricted, &host).await;
        for (sandbox, stop) in [(&fork, &fork_spec.stop_signal), (&id, &source_spec.stop_signal)] {
            runtime.stop(sandbox, stop, Duration::from_secs(15)).await.unwrap();
            runtime.remove(sandbox).await.unwrap();
        }
    }
    server.abort();
}

/// Continues the same guest process and waits for its next DNS answer through the same UDP socket.
async fn retained_dns(runtime: &VmRuntime, id: &str, phase: usize) -> String {
    if phase != 0 {
        exec(runtime, id, &format!("touch /tmp/retained-dns.continue{phase}")).await;
    }
    read(
        runtime,
        id,
        &format!(
            "while ! test -f /tmp/retained-dns{phase}; do if test -s /tmp/retained-dns.log; then cat /tmp/retained-dns.log; exit 1; fi; sleep 0.01; done; cat /tmp/retained-dns{phase}"
        ),
    )
    .await
}

/// The directory the monitor of `id`'s current boot runs in, where a sleep writes its snapshot.
async fn boot_directory_of(runtime: &VmRuntime, id: &str) -> PathBuf {
    let live = runtime.state.live.lock().unwrap()[id].clone();
    let live = live.lock().await;
    live._directory.path().to_owned()
}

/// A snapshot that fails after the monitor saved the devices' state leaves the guest running where
/// it was, its network included, and its next sleep works; one whose monitor never answers again
/// (its memory file a FIFO nobody reads, so the create blocks) is retired, and its next start
/// boots the disk with everything written before the sleep.
#[tokio::test]
#[ignore = "requires real KVM, the verified VM archive and public DNS; the VM experiments workflow runs it"]
async fn actual_failed_snapshots_resume_or_retire_the_guest() {
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
    let host = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let (id, source_spec) = super::network_acceptance::start(&runtime, temporary.path(), image, 1, true, &host).await;
    exec(&runtime, &id, "printf kept > /tmp/marker; printf 'written before sleeping' > /data/durable").await;

    // The memory file can't be made: the create fails after the devices' state was saved.
    let unwritable = boot_directory_of(&runtime, &id).await.join("memory.bin");
    fs::create_dir(&unwritable).unwrap();
    let refused = runtime.pause(&id).await.expect_err("A snapshot without its memory file fails");
    eprintln!("VM sleep with an unwritable memory file: {refused}");
    assert_eq!(runtime.inspect(&id).await.unwrap().unwrap().status, ContainerStatus::Running);
    eprintln!("VM failed snapshot marker phase: first after recovery");
    assert_eq!(read(&runtime, &id, "cat /tmp/marker").await, "kept", "The guest went on where it was");
    super::network_acceptance::probe(&runtime, &id, true, &host).await;
    fs::remove_dir(&unwritable).unwrap();
    runtime.pause(&id).await.unwrap();
    runtime.unpause(&id).await.unwrap();
    eprintln!("VM failed snapshot marker phase: second after pause and unpause");
    assert_eq!(read(&runtime, &id, "cat /tmp/marker").await, "kept");
    super::network_acceptance::probe(&runtime, &id, true, &host).await;

    // The create blocks on a FIFO and the monitor answers nothing more, a resume included.
    let blocked = boot_directory_of(&runtime, &id).await.join("memory.bin");
    let made = std::process::Command::new("mkfifo").arg(&blocked).status().unwrap();
    assert!(made.success());
    let started = std::time::Instant::now();
    let retired = runtime.pause(&id).await.expect_err("A snapshot that never finishes fails");
    eprintln!("VM sleep with a blocked create, after {:.1} s: {retired}", started.elapsed().as_secs_f64());
    let after = runtime.inspect(&id).await.unwrap().unwrap();
    assert!(!matches!(after.status, ContainerStatus::Running | ContainerStatus::Paused), "{:?}", after.status);
    assert_eq!(after.exit_code, crate::runtime::exit_by(libc::SIGKILL));
    runtime.start(&id).await.unwrap();
    assert_eq!(exec(&runtime, &id, "cat /data/durable").await, b"written before sleeping");
    assert_eq!(read(&runtime, &id, "test -e /tmp/marker && echo memory || echo cold").await, "cold");
    runtime.stop(&id, &source_spec.stop_signal, Duration::from_secs(15)).await.unwrap();
    runtime.remove(&id).await.unwrap();
    assert!(fs::read_dir(runtime.state.root.join("memory")).unwrap().next().is_none());
    server.abort();
}

/// A removal takes a guest's memory files with it, even ones its record lost.
#[tokio::test]
#[ignore = "requires real KVM and the verified VM archive; the VM experiments workflow runs it"]
async fn actual_memory_goes_with_its_sandbox() {
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
    let data = temporary.path().join("data");
    directory(&data).unwrap();
    let id = runtime.create(&spec(&data, inputs[0]["reference"].as_str().unwrap())).await.unwrap();
    runtime.start(&id).await.unwrap();
    runtime.pause(&id).await.unwrap();
    let memory = runtime.state.root.join("memory");
    assert!(fs::read_dir(&memory).unwrap().next().is_some());
    runtime.state.entries.lock().unwrap().get_mut(&id).unwrap().memory = None;
    runtime.remove(&id).await.unwrap();
    assert!(fs::read_dir(&memory).unwrap().next().is_none(), "Guest memory never outlives its sandbox");
    server.abort();
}

/// A memory image is used once even across a crash: consumed before its restore, it is never
/// restored by a pryxd that starts after a crash between that restore and its removal, which sweeps
/// its files and boots the disk.
#[tokio::test]
#[ignore = "requires real KVM and the verified VM archive; the VM experiments workflow runs it"]
async fn actual_memory_is_used_once_across_a_crash() {
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
    let runtime = VmRuntime::new(temporary.path(), settings.clone()).await.unwrap();
    let inputs: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../../templates/vm-tools/image-inputs.json")).unwrap();
    let data = temporary.path().join("data");
    directory(&data).unwrap();
    let original = spec(&data, inputs[0]["reference"].as_str().unwrap());
    let id = runtime.create(&original).await.unwrap();
    runtime.start(&id).await.unwrap();
    exec(&runtime, &id, "printf kept > /tmp/marker; printf 'written before sleeping' > /data/durable").await;
    runtime.pause(&id).await.unwrap();
    let memory = runtime.state.root.join("memory");

    // pryxd dies after consuming the image for a restore, before removing it.
    let mut entry = runtime.entry(&id).unwrap();
    let consumed = runtime.consume(&mut entry).unwrap();
    assert!(consumed.join(super::memory::MEMORY).exists());
    drop(runtime);
    // The runtime's state lease is free once the last of its tasks has ended.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let runtime = loop {
        match VmRuntime::new(temporary.path(), settings.clone()).await {
            Ok(runtime) => break runtime,
            Err(problem) if tokio::time::Instant::now() < deadline => {
                eprintln!("VM runtime not yet free: {problem}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(problem) => panic!("{problem}"),
        }
    };
    assert!(fs::read_dir(&memory).unwrap().next().is_none(), "The next start swept the consumed image");
    let after = runtime.inspect(&id).await.unwrap().unwrap();
    assert_ne!(after.status, ContainerStatus::Paused, "A consumed image is never asleep again");
    let refused = runtime.unpause(&id).await.expect_err("A consumed image is never restored");
    eprintln!("VM wake after a crash mid-restore: {refused}");
    runtime.start(&id).await.unwrap();
    assert_eq!(exec(&runtime, &id, "cat /data/durable").await, b"written before sleeping");
    assert_eq!(read(&runtime, &id, "test -e /tmp/marker && echo memory || echo cold").await, "cold");

    runtime.remove(&id).await.unwrap();
    server.abort();
}
