// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The gvisor tier beside the container tier on one real engine. Whether the engine has gVisor's
//! `runsc` registered is read from the engine itself (`docker info`), never assumed: without it the
//! tier is refused by name, and with it a named tier is never served by the other backend.
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

/// What the engine itself says: `runsc` among the runtimes `docker info` lists.
fn engine_registers_runsc() -> bool {
    let out = std::process::Command::new("docker")
        .args(["info", "--format", "{{json .Runtimes}}"])
        .output()
        .expect("the docker CLI answers info");
    assert!(out.status.success(), "docker info: {}", String::from_utf8_lossy(&out.stderr));
    let runtimes: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    runtimes.get(pryxd::runtime::docker::GVISOR_RUNTIME).is_some()
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
    let socket = DockerConfig::default().socket;
    let registered = engine_registers_runsc();
    if on_gvisor() {
        assert!(registered, "the gvisor run of the suite needs runsc registered with the engine");
    }
    let gvisor = DockerRuntime::new(&socket).unwrap().gvisor();
    assert_eq!(gvisor.isolation(), "gvisor");
    match gvisor.serving_info().await {
        Ok(_) => assert!(registered, "gvisor served without runsc registered"),
        Err(error) => {
            assert!(!registered, "runsc is registered, yet gvisor was refused: {error}");
            assert!(
                error.to_string().contains(pryxd::runtime::docker::GVISOR_MISSING_PROBLEM),
                "the refusal names what is missing: {error}"
            );
        }
    }
}

/// Two backends on one engine: each workload keeps the tier it named, the engine records the
/// runtime that tier means, and the gvisor sandbox answers from a kernel that isn't the host's.
#[tokio::test]
#[ignore = "needs Docker"]
async fn a_named_tier_is_never_served_by_the_other_engine_backend() {
    let socket = DockerConfig::default().socket;
    let registered = engine_registers_runsc();
    let container: Arc<dyn ContainerRuntime> = Arc::new(DockerRuntime::new(&socket).unwrap());
    let gvisor: Arc<dyn ContainerRuntime> = Arc::new(DockerRuntime::new(&socket).unwrap().gvisor());
    let mut tiers = vec![container.clone()];
    if registered {
        tiers.push(gvisor.clone());
    }
    let router: Arc<dyn ContainerRuntime> = Arc::new(Router::new(tiers).unwrap());
    let dir = tempfile::tempdir().unwrap();
    let dep = deployment();
    let _cleanup = Cleanup(dep.clone());
    let probe = pryxd::ports::bind_probe(vec!["127.0.0.1".parse().unwrap()]);
    let m = manager_probed(dir.path(), router, &extra(&dep), probe);
    assert!(m.reconcile(Records::FromDisk).await.error.is_none());
    let features = m.features();
    assert_eq!(features.iter().any(|f| f == "tier-gvisor"), registered, "{features:?}");
    if !registered {
        let refused = m.ensure(id("g"), tiered("gvisor"), Precondition::None, None).await;
        let Err(NodeError::Invalid(problems)) = refused else { panic!("gvisor without runsc: {refused:?}") };
        let said = format!("{problems:?}");
        assert!(said.contains("isolation") && said.contains("does not serve the gvisor tier"), "{said}");
        return;
    }
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
