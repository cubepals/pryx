// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The gvisor tier beside the container tier on one real engine, the selected one (a rootless
//! Docker or Podman in the engine runs). Whether that engine can serve gVisor is read from the
//! engine itself, never assumed: where it can't, the tier is refused by name and left off; where it
//! can, a named tier is never served by the other backend.
use super::*;
use pryxd::protocol::Guarantee;
use pryxd::runtime::router::Router;

/// Whether this run of the suite is the gvisor tier's (`PRYX_TEST_GVISOR`): each case that makes
/// its own runtime then makes the gvisor one, on the same engine, which CI registers runsc with.
pub(super) fn on_gvisor() -> bool {
    std::env::var_os("PRYX_TEST_GVISOR").is_some()
}

/// The backend a case runs on: the container tier, or the gvisor tier on the same engine.
pub(super) fn engine_runtime(socket: &std::path::Path) -> DockerRuntime {
    let runtime = DockerRuntime::new(socket).unwrap();
    if on_gvisor() { runtime.gvisor() } else { runtime }
}

/// A node exec past its deadline: killed in a container, refused by name on gvisor.
pub(super) fn timed_out(result: Result<pryxd::protocol::ExecResponse, NodeError>) {
    if on_gvisor() {
        let said = format!("{:?}", result.expect_err("gvisor names exec-timeout as unavailable"));
        assert!(said.contains(pryxd::runtime::docker::EXEC_TIMEOUT_PROBLEM), "{said}");
    } else {
        let out = result.unwrap();
        assert!(out.timed_out && out.killed, "{out:?}");
    }
}

/// How many timed-out sleeps remain: none in a container; on gvisor, whose refusal stops none,
/// they are still there, as the refusal says.
pub(super) fn left_running(count: &str, why: &str) {
    if on_gvisor() {
        assert!(count.trim().parse::<u32>().unwrap() > 0, "refused, so left running: {count}");
    } else {
        assert_eq!(count.trim(), "0", "{why}");
    }
}

/// The pids limit: a container's own cgroup shows it; gVisor's kernel shows its own cgroups, so
/// on gvisor the engine's record of the limit it applied is read instead.
pub(super) fn pids_limited(inside: &str, container: &str) {
    if !on_gvisor() {
        return assert!(inside.contains("4096"), "pids limit");
    }
    let out = std::process::Command::new("docker")
        .args(["inspect", "--format", "{{.HostConfig.PidsLimit}}", container])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "4096", "the engine applied the pids limit");
}

/// Hostname egress on gvisor: never announced, and a create that names a policy is refused.
pub(super) async fn hostname_egress_refused() {
    let dir = tempfile::tempdir().unwrap();
    let dep = deployment();
    let _cleanup = Cleanup(dep.clone());
    let m = daemon(dir.path(), &dep);
    assert!(m.reconcile(Records::FromDisk).await.error.is_none());
    assert!(!m.features().iter().any(|f| f == "egress-hostnames" || f.ends_with(":egress-hostnames")));
    let spec: WorkloadSpec = serde_json::from_value(serde_json::json!({
        "image": "alpine:3.22",
        "entrypoint": ["/bin/sh", "-c", SERVER],
        "egress": { "allow": ["example.com"] },
        "resources": { "memoryMb": 128, "cpuMillis": 500 },
        "storage": { "mountPath": "/data", "sizeGb": 1 },
        "stop": { "timeoutSeconds": 10 }
    }))
    .unwrap();
    let made = m.ensure(id("dns"), spec, Precondition::None, None).await;
    let said = format!("{made:?}");
    assert!(made.is_err() && said.contains("egress"), "a hostname policy is refused on gvisor: {said}");
}

/// Evidence of how the engine shows an OOM kill: the highest `oom_kill` count seen in the
/// container's own cgroup while it runs, as the kernel keeps it (cgroup v2 memory.events).
pub(super) async fn cgroup_oom_kills(api: &bollard::Docker, container: &str, highest: &mut Option<u64>) {
    let Ok(inspect) = api.inspect_container(container, None).await else { return };
    let Some(pid) = inspect.state.and_then(|state| state.pid).filter(|pid| *pid > 0) else { return };
    let Ok(groups) = std::fs::read_to_string(format!("/proc/{pid}/cgroup")) else { return };
    let Some(path) = groups.lines().find_map(|line| line.strip_prefix("0::")) else { return };
    let Ok(events) = std::fs::read_to_string(format!("/sys/fs/cgroup{path}/memory.events")) else { return };
    let count = events.lines().find_map(|line| line.strip_prefix("oom_kill ")).and_then(|c| c.trim().parse().ok());
    if let Some(count) = count {
        *highest = Some(highest.map_or(count, |seen: u64| seen.max(count)));
    }
}

/// The engine's own events for `container` since `since` (Unix seconds): whether an `oom` came,
/// and where it fell beside the `die`.
pub(super) fn engine_events(container: &str, since: u64) -> String {
    let until = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 1;
    let filter = format!("container={container}");
    let (since, until) = (since.to_string(), until.to_string());
    let out = std::process::Command::new("docker")
        .args([
            "events",
            "--since",
            &since,
            "--until",
            &until,
            "--filter",
            &filter,
            "--format",
            "{{.TimeNano}} {{.Action}}",
        ])
        .output();
    out.map(|out| String::from_utf8_lossy(&out.stdout).split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

/// Why the engine at `socket` can't serve the gvisor tier, read from the engine's own API, or None
/// where it can. gVisor runs through the container tier, so that tier's own refusal comes first;
/// then Podman, which pryx doesn't start gVisor on; then `runsc` missing from the engine's runtimes.
async fn gvisor_refusal(socket: &std::path::Path) -> Option<String> {
    let engine = engine_api(socket).await;
    let version = engine.version().await.expect("the engine answers version");
    match DockerRuntime::new(socket).unwrap().serving_info().await {
        Err(pryxd::runtime::RuntimeError::Unavailable(problem)) => return Some(problem),
        Err(error) => panic!("the container tier's engine answers: {error}"),
        Ok(_) => {}
    }
    let components = version.components.unwrap_or_default();
    if components.iter().any(|component| component.name == pryxd::runtime::docker::PODMAN_ENGINE_COMPONENT) {
        return Some(pryxd::runtime::docker::gvisor::GVISOR_PODMAN_PROBLEM.into());
    }
    let info = engine.info().await.expect("the engine answers info");
    let registered = info.runtimes.unwrap_or_default().contains_key(pryxd::runtime::docker::GVISOR_RUNTIME);
    (!registered).then(|| pryxd::runtime::docker::GVISOR_MISSING_PROBLEM.into())
}

/// The engine's own API at `socket`, at the version it negotiates (Podman's is older than Docker's).
async fn engine_api(socket: &std::path::Path) -> bollard::Docker {
    bollard::Docker::connect_with_unix(socket.to_str().unwrap(), 60, bollard::API_DEFAULT_VERSION)
        .unwrap()
        .negotiate_version()
        .await
        .expect("the engine negotiates its API")
}

/// The engine's own record of the runtime a container was made with.
fn engine_runtime_of(container: &str) -> String {
    let out = std::process::Command::new("docker")
        .args(["inspect", "--format", "{{.HostConfig.Runtime}}", container])
        .output()
        .unwrap();
    assert!(out.status.success(), "docker inspect: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn tiered(isolation: &str) -> WorkloadSpec {
    serde_json::from_value(serde_json::json!({
        "isolation": isolation,
        "image": "alpine:3.22",
        "entrypoint": ["/bin/sh", "-c", SERVER],
        "resources": { "memoryMb": 256, "cpuMillis": 500 },
        "storage": { "mountPath": "/data", "sizeGb": 1 },
        "stop": { "timeoutSeconds": 10 }
    }))
    .unwrap()
}

async fn kernel(m: &Arc<Manager>, w: &str) -> String {
    let out = m
        .exec(id(w), ExecRequest { command: vec!["uname".into(), "-r".into()], timeout_seconds: 20 }, None, None)
        .await
        .unwrap();
    assert_eq!(out.exit_code, Some(0), "{out:?}");
    out.stdout.trim().to_owned()
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn gvisor_is_served_only_where_the_engine_registers_runsc() {
    let socket = engine::selected_socket().await;
    let refusal = gvisor_refusal(&socket).await;
    if on_gvisor() {
        assert_eq!(refusal, None, "the gvisor run of the suite needs runsc registered with Docker");
    }
    let gvisor = DockerRuntime::new(&socket).unwrap().gvisor();
    assert_eq!(gvisor.isolation(), "gvisor");
    let served = gvisor.serving_info().await;
    match &refusal {
        None => assert!(served.is_ok(), "runsc is registered, yet gvisor was refused: {served:?}"),
        Some(why) => {
            let error = served.expect_err("gvisor served where the engine can't serve it");
            assert!(error.to_string().contains(why.as_str()), "the refusal names what is missing: {error}");
        }
    }
    // The auto configuration, which picks a host's tiers, leaves gvisor off for the same reason.
    let dir = tempfile::tempdir().unwrap();
    let auto = format!("runtime = \"auto\"\n[docker]\nsocket = \"{}\"\n", socket.display());
    let available = pryxd::runtime::configured::availability(&support::config(dir.path(), &auto)).await;
    assert_eq!(available[pryxd::protocol::Isolation::Gvisor.as_str()], refusal, "{available:?}");
}

/// Two backends on one engine: each workload keeps the tier it named, the engine records the
/// runtime that tier means, and the gvisor sandbox answers from a kernel that isn't the host's.
#[tokio::test]
#[ignore = "needs Docker"]
async fn a_named_tier_is_never_served_by_the_other_engine_backend() {
    let socket = engine::selected_socket().await;
    let container: Arc<dyn ContainerRuntime> = Arc::new(DockerRuntime::new(&socket).unwrap());
    let dir = tempfile::tempdir().unwrap();
    let dep = deployment();
    if let Some(why) = gvisor_refusal(&socket).await {
        refused_where_unserved(&socket, container, dir.path(), &dep, &why).await;
        return;
    }
    let _cleanup = Cleanup(dep.clone());
    let gvisor: Arc<dyn ContainerRuntime> = Arc::new(DockerRuntime::new(&socket).unwrap().gvisor());
    let router: Arc<dyn ContainerRuntime> = Arc::new(Router::new(vec![container.clone(), gvisor.clone()]).unwrap());
    let probe = pryxd::ports::bind_probe(vec!["127.0.0.1".parse().unwrap()]);
    let m = manager_probed(dir.path(), router, &extra(&dep), probe);
    let report = m.reconcile(Records::FromDisk).await;
    assert!(report.error.is_none(), "{report:?}");
    let features = m.features();
    assert!(features.iter().any(|f| f == "tier-gvisor"), "{features:?}");
    for tier in ["tier-gvisor:private-network", "tier-gvisor:private-pids"] {
        assert!(features.iter().any(|f| f == tier), "{tier}: {features:?}");
    }
    assert!(!features.iter().any(|f| f == "tier-gvisor:own-kernel"), "{features:?}");
    m.ensure(id("c"), tiered("container"), Precondition::None, None).await.unwrap();
    m.ensure(id("g"), tiered("gvisor"), Precondition::None, None).await.unwrap();
    m.start(id("c"), None).await.unwrap();
    m.start(id("g"), None).await.unwrap();
    let (c, g) = (m.view(&id("c")).unwrap(), m.view(&id("g")).unwrap());
    assert_eq!(c.isolation, None, "containers keep their unnamed view");
    assert_eq!(g.isolation.as_deref(), Some("gvisor"));
    assert!(g.guarantees[&Guarantee::PrivatePids] && g.guarantees[&Guarantee::PrivateNetwork]);
    assert!(!g.guarantees[&Guarantee::OwnKernel], "gVisor's kernel is not claimed as a guest kernel");
    assert_eq!(engine_runtime_of(&g.locate.container_name), pryxd::runtime::docker::GVISOR_RUNTIME);
    assert_ne!(engine_runtime_of(&c.locate.container_name), pryxd::runtime::docker::GVISOR_RUNTIME);
    let filter = [format!("{LABEL_DEPLOYMENT}={dep}")];
    assert_eq!(container.list(&filter).await.unwrap().len(), 1, "the container backend sees only its own");
    assert_eq!(gvisor.list(&filter).await.unwrap().len(), 1, "the gvisor backend sees only its own");
    let host = String::from_utf8(std::process::Command::new("uname").arg("-r").output().unwrap().stdout).unwrap();
    let host = host.trim();
    assert_eq!(kernel(&m, "c").await, host, "a container shares the host's kernel");
    let guest = kernel(&m, "g").await;
    assert_ne!(guest, host, "the gvisor sandbox answers from gVisor's own kernel");
    eprintln!("GVISOR_KERNEL host={host} sandbox={guest}");
    // A restarted daemon routes each again by the engine's own record, without ambiguity.
    let report = m.reconcile(Records::FromDisk).await;
    assert!(report.error.is_none(), "{report:?}");
    assert_eq!(m.view(&id("g")).unwrap().locate.container_id, g.locate.container_id);
    cleanup(&m, &["c", "g"]).await;
}

/// An engine that can't serve gvisor: the node serves its containers alone, as the auto
/// configuration makes it there, and refuses a workload that names gvisor before making anything.
async fn refused_where_unserved(
    socket: &std::path::Path,
    container: Arc<dyn ContainerRuntime>,
    dir: &std::path::Path,
    dep: &str,
    why: &str,
) {
    let router: Arc<dyn ContainerRuntime> = Arc::new(Router::new(vec![container]).unwrap());
    let m = manager_on(dir, router, &extra(dep).replace(HOST_OWNERSHIP, "[workloads]\n"));
    let features = m.features();
    assert!(!features.iter().any(|f| f.starts_with("tier-gvisor")), "{features:?}");
    let refused = m.ensure(id("g"), tiered("gvisor"), Precondition::None, None).await;
    let Err(NodeError::Invalid(problems)) = refused else {
        panic!("gvisor where the engine can't serve it: {refused:?}")
    };
    let said = format!("{problems:?}");
    assert!(said.contains("isolation") && said.contains("does not serve the gvisor tier"), "{said}");
    assert!(!m.store.data_dir(&id("g")).exists(), "a refused tier makes no data");
    let engine = engine_api(socket).await;
    let options = bollard::query_parameters::ListContainersOptionsBuilder::new()
        .all(true)
        .filters(&std::collections::HashMap::from([("label".to_owned(), vec![format!("{LABEL_DEPLOYMENT}={dep}")])]))
        .build();
    assert!(engine.list_containers(Some(options)).await.unwrap().is_empty(), "a refused tier makes no compute");
    eprintln!("gvisor refused on this engine: {why}");
}

/// Nearest-rank p50 and p90 of `samples`, in milliseconds.
fn percentiles(mut samples: Vec<f64>) -> (f64, f64) {
    samples.sort_by(f64::total_cmp);
    let rank = |p: f64| samples[((p * samples.len() as f64).ceil() as usize).clamp(1, samples.len()) - 1];
    (rank(0.5), rank(0.9))
}

/// K26's figures for docs/tiers.md: thirty samples each of create to running, an exec of `true`,
/// a syscall-heavy command and a 16 MiB transfer from the published port, on both engine tiers
/// of the one runner. Printed as `K26_MEASURE` lines for the job log.
#[tokio::test]
#[ignore = "needs Docker with runsc registered; CI's gvisor run measures"]
async fn container_and_gvisor_figures() {
    if !on_gvisor() {
        // A measurement, not a guarantee: the gvisor run of the suite, with runsc, takes it.
        return;
    }
    let socket = DockerConfig::default().socket;
    for (tier, runtime) in [
        ("container", Arc::new(DockerRuntime::new(&socket).unwrap()) as Arc<dyn ContainerRuntime>),
        ("gvisor", Arc::new(DockerRuntime::new(&socket).unwrap().gvisor())),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let dep = deployment();
        let _cleanup = Cleanup(dep.clone());
        let m = manager_probed(
            dir.path(),
            runtime,
            &extra(&dep),
            pryxd::ports::bind_probe(vec!["127.0.0.1".parse().unwrap()]),
        );
        assert!(m.reconcile(Records::FromDisk).await.error.is_none());
        let serve = format!(
            "head -c 16777216 /dev/zero > /data/blob; nc -lk -p {} -e cat /data/blob & while true; do sleep 1; done",
            pryxd::examples::HTTP_PORT
        );
        let (mut create, mut exec, mut syscalls, mut transfer) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut stream = Vec::new();
        for sample in 0..30 {
            let w = format!("m{sample}");
            let began = Instant::now();
            m.ensure(id(&w), super::shell(&serve, 256), Precondition::None, None).await.unwrap();
            m.start(id(&w), None).await.unwrap();
            create.push(began.elapsed().as_secs_f64() * 1000.0);
            let run = |script: &str| ExecRequest {
                command: vec!["sh".into(), "-c".into(), script.into()],
                timeout_seconds: 60,
            };
            let began = Instant::now();
            assert_eq!(
                m.exec(id(&w), ExecRequest { command: vec!["true".into()], timeout_seconds: 20 }, None, None)
                    .await
                    .unwrap()
                    .exit_code,
                Some(0)
            );
            exec.push(began.elapsed().as_secs_f64() * 1000.0);
            let began = Instant::now();
            let out = m
                .exec(id(&w), run("dd if=/dev/zero of=/dev/null bs=1 count=200000 2>/dev/null"), None, None)
                .await
                .unwrap();
            assert_eq!(out.exit_code, Some(0), "{out:?}");
            syscalls.push(began.elapsed().as_secs_f64() * 1000.0);
            let began = Instant::now();
            let out = m.exec(id(&w), run("head -c 262144 /dev/zero"), None, None).await.unwrap();
            assert_eq!(out.stdout.len(), 256 * 1024);
            stream.push(began.elapsed().as_secs_f64() * 1000.0);
            let port = m.view(&id(&w)).unwrap().ports[0].host_port;
            let received = loop {
                let began = Instant::now();
                let bytes = tokio::task::spawn_blocking(move || {
                    use std::io::Read;
                    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
                    let mut all = Vec::new();
                    stream.read_to_end(&mut all).ok()?;
                    Some(all.len())
                })
                .await
                .unwrap();
                if bytes == Some(16 * 1024 * 1024) {
                    break began.elapsed().as_secs_f64() * 1000.0;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            transfer.push(received);
            m.delete(id(&w), DataDisposition::Delete, Some(w.clone()), None).await.unwrap();
        }
        for (metric, samples) in [
            ("create", create),
            ("exec-true", exec),
            ("syscall-heavy", syscalls),
            ("exec-output-256KiB", stream),
            ("transfer-16MiB", transfer),
        ] {
            let (p50, p90) = percentiles(samples);
            eprintln!("K26_MEASURE tier={tier} metric={metric} n=30 p50={p50:.2} p90={p90:.2}");
        }
    }
}
