// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Resource-policy acceptance on the actual selected engine. Rootful execution must retain hard
//! limits; an engine without controllers must refuse requested limits before creating data.
use super::*;

#[tokio::test]
#[ignore = "needs a real Docker-compatible engine"]
async fn explicit_memory_acceptance_never_removes_available_enforcement() {
    let socket = selected_socket().await;
    let runtime = Arc::new(DockerRuntime::new(&socket).unwrap());
    let Some(limits) = limits_or_mapping_refusal(&runtime, &socket).await else { return };
    let dir = tempfile::tempdir().unwrap();
    let dep = deployment();
    let _cleanup = EngineCleanup { socket: socket.clone(), deployment: dep.clone() };
    let configuration = configuration(&dep, limits.rootless);
    let manager = manager_on(dir.path(), runtime, &configuration);
    let report = manager.reconcile(Records::FromDisk).await;
    assert!(report.error.is_none(), "{report:?}");
    let mut value = support::spec_json();
    value["entrypoint"] = serde_json::json!(["/bin/sh", "-c", "trap 'exit 0' TERM; while :; do sleep 1; done"]);
    value["stop"]["timeoutSeconds"] = 5.into();
    value["resources"] = serde_json::json!({"memoryMb": 256});
    if !limits.memory {
        let refused = manager
            .ensure(id("limited"), serde_json::from_value(value.clone()).unwrap(), Precondition::None, None)
            .await
            .unwrap_err();
        refusal(refused, pryxd::protocol::Guarantee::MemoryLimit);
        assert!(!manager.store.data_dir(&id("limited")).exists());
    }
    value["resources"]["allowUnenforcedMemory"] = true.into();
    if !limits.cpu {
        value["resources"]["cpuMillis"] = pryxd::protocol::MIN_CPU_MILLIS.into();
        let refused = manager
            .ensure(id("cpu-limited"), serde_json::from_value(value.clone()).unwrap(), Precondition::None, None)
            .await
            .unwrap_err();
        refusal(refused, pryxd::protocol::Guarantee::CpuLimit);
        assert!(!manager.store.data_dir(&id("cpu-limited")).exists());
        value["resources"].as_object_mut().unwrap().remove("cpuMillis");
    }
    extra_limit_refusals(&manager, &limits, &value).await;
    let made =
        manager.ensure(id("accepted"), serde_json::from_value(value).unwrap(), Precondition::None, None).await.unwrap();
    let view = serde_json::to_value(&made.workload).unwrap();
    assert_eq!(view["guarantees"]["memory-limit"], limits.memory);
    assert_eq!(view["guarantees"]["cpu-limit"], limits.cpu);
    let backend =
        bollard::Docker::connect_with_unix(socket.to_str().unwrap(), 60, bollard::API_DEFAULT_VERSION).unwrap();
    let options = bollard::query_parameters::ListContainersOptionsBuilder::new()
        .all(true)
        .filters(&std::collections::HashMap::from([("label".to_owned(), vec![format!("{LABEL_DEPLOYMENT}={dep}")])]))
        .build();
    let containers = backend.list_containers(Some(options)).await.unwrap();
    assert_eq!(containers.len(), 1, "refusals create no compute");
    let inspected = backend.inspect_container(containers[0].id.as_ref().unwrap(), None).await.unwrap();
    let host = inspected.host_config.unwrap();
    assert_eq!(host.memory.unwrap_or_default(), if limits.memory { 256 * 1024 * 1024 } else { 0 });
    assert_eq!(host.readonly_rootfs, Some(true));
    assert_eq!(host.privileged, Some(false));
    let dropped = host.cap_drop.unwrap();
    if std::env::var("PRYX_TEST_ENGINE").as_deref() == Ok("podman") {
        assert!(!dropped.is_empty(), "the native API must retain capability drops");
    } else {
        assert!(dropped.iter().any(|cap| cap == "ALL"));
    }
    assert!(
        host.security_opt
            .unwrap()
            .iter()
            .any(|option| option == "no-new-privileges:true" || option == "no-new-privileges")
    );
    manager.start(id("accepted"), None).await.unwrap();
    assert_kernel_capabilities(&manager).await;
    let result = manager
        .exec(
            id("accepted"),
            ExecRequest {
                command: vec![
                    "sh".into(),
                    "-c".into(),
                    "printf retained > /data/engine-policy; cat /data/engine-policy".into(),
                ],
                timeout_seconds: 10,
            },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.stdout, "retained");
    cleanup(&manager, &["accepted"]).await;
}

async fn extra_limit_refusals(
    manager: &Arc<Manager>,
    limits: &pryxd::runtime::limits::ResourceLimits,
    value: &serde_json::Value,
) {
    for (name, field, enforced) in
        [("weight-limited", "cpuWeight", limits.cpu), ("pids-limited", "pidsLimit", limits.pids)]
    {
        if enforced {
            continue;
        }
        let mut value = value.clone();
        value["resources"][field] = 128.into();
        let error = manager
            .ensure(id(name), serde_json::from_value(value).unwrap(), Precondition::None, None)
            .await
            .unwrap_err();
        let NodeError::Invalid(errors) = error else { panic!("unexpected refusal: {error:?}") };
        let errors = serde_json::to_value(&errors).unwrap();
        assert!(errors.as_array().unwrap().iter().any(|error| error["field"] == format!("resources.{field}")));
        let named = if field == "cpuWeight" {
            serde_json::to_value(pryxd::protocol::Guarantee::CpuLimit).unwrap()
        } else {
            "pids-limit".into()
        };
        assert!(
            errors
                .as_array()
                .unwrap()
                .iter()
                .any(|error| error["problem"].as_str().unwrap().contains(named.as_str().unwrap()))
        );
        assert!(!manager.store.data_dir(&id(name)).exists());
    }
}

fn refusal(error: NodeError, guarantee: pryxd::protocol::Guarantee) {
    let NodeError::Invalid(errors) = error else {
        panic!("unexpected refusal: {error:?}");
    };
    let expected = serde_json::to_value(guarantee).unwrap();
    assert!(serde_json::to_string(&errors).unwrap().contains(expected.as_str().unwrap()));
}

pub(super) struct EngineCleanup {
    pub(super) socket: std::path::PathBuf,
    pub(super) deployment: String,
}
impl Drop for EngineCleanup {
    fn drop(&mut self) {
        let endpoint = format!("unix://{}", self.socket.display());
        let filter = format!("label={LABEL_DEPLOYMENT}={}", self.deployment);
        if let Ok(output) =
            std::process::Command::new("docker").args(["--host", &endpoint, "ps", "-aq", "--filter", &filter]).output()
        {
            let ids = String::from_utf8_lossy(&output.stdout).split_whitespace().map(str::to_owned).collect::<Vec<_>>();
            if output.status.success() && !ids.is_empty() {
                let _ = std::process::Command::new("docker").args(["--host", &endpoint, "rm", "-f"]).args(ids).output();
            }
        }
        for network in [format!("engine-{}", self.deployment), format!("engine-{}-internal", self.deployment)] {
            let _ =
                std::process::Command::new("docker").args(["--host", &endpoint, "network", "rm", &network]).output();
        }
    }
}

pub(super) fn configuration(deployment: &str, rootless: bool) -> String {
    let owner = if rootless { "[workloads]\n" } else { HOST_OWNERSHIP };
    format!("{}\n[docker]\nnetwork = \"engine-{deployment}\"\n", extra(deployment).replace(HOST_OWNERSHIP, owner))
}

pub(super) async fn selected_socket() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("PRYX_TEST_ENGINE_SOCKET") {
        let path = std::path::PathBuf::from(path);
        assert!(path.is_absolute());
        return path;
    }
    pryxd::runtime::engine::socket(&DockerConfig::default()).await.unwrap()
}

#[tokio::test]
#[ignore = "needs the actual selected engine"]
async fn discovery_reaches_the_selected_environment_engine() {
    let expected = selected_socket().await;
    let selected = pryxd::runtime::engine::socket(&DockerConfig::default()).await.unwrap();
    assert_eq!(
        selected,
        expected,
        "selected endpoint metadata: {:?}",
        pryxd::runtime::engine::inspect(&expected).await
    );
}

/// Unsupported rootless ownership is an asserted pre-compute refusal, never a skipped case.
/// The existing rootful and supported rootless branches retain all positive assertions above.
pub(super) async fn limits_or_mapping_refusal(
    runtime: &Arc<DockerRuntime>,
    socket: &std::path::Path,
) -> Option<pryxd::runtime::limits::ResourceLimits> {
    let error = match runtime.resource_limits().await {
        Ok(Some(limits)) => return Some(limits),
        result => result.unwrap_err(),
    };
    let requirement = pryxd::runtime::docker::DATA_OWNER_REQUIREMENT;
    assert!(error.to_string().contains(requirement), "unexpected engine policy: {error}");
    assert_eq!(
        std::env::var("PRYX_TEST_ENGINE").as_deref(),
        Ok("docker"),
        "only the measured unsupported Docker mapping is parked"
    );
    let dir = tempfile::tempdir().unwrap();
    let dep = deployment();
    let manager = manager_on(dir.path(), runtime.clone(), &extra(&dep).replace(HOST_OWNERSHIP, "[workloads]\n"));
    let mut value = support::spec_json();
    value["resources"]["allowUnenforcedMemory"] = true.into();
    let error = manager
        .ensure(id("unmapped"), serde_json::from_value(value).unwrap(), Precondition::None, None)
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains(requirement));
    assert!(!manager.store.data_dir(&id("unmapped")).exists());
    let backend =
        bollard::Docker::connect_with_unix(socket.to_str().unwrap(), 60, bollard::API_DEFAULT_VERSION).unwrap();
    let options = bollard::query_parameters::ListContainersOptionsBuilder::new()
        .all(true)
        .filters(&std::collections::HashMap::from([("label".to_owned(), vec![format!("{LABEL_DEPLOYMENT}={dep}")])]))
        .build();
    assert!(backend.list_containers(Some(options)).await.unwrap().is_empty(), "a refused mapping creates no compute");
    eprintln!("asserted refusal: {error:?}");
    None
}

async fn assert_kernel_capabilities(manager: &Arc<Manager>) {
    let result = manager
        .exec(
            id("accepted"),
            ExecRequest { command: vec!["cat".into(), "/proc/self/status".into()], timeout_seconds: 10 },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.exit_code, Some(0));
    let fields =
        result.stdout.lines().filter_map(|line| line.split_once(':')).collect::<std::collections::BTreeMap<_, _>>();
    for name in ["CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"] {
        assert_eq!(u64::from_str_radix(fields[name].trim(), 16).unwrap(), 0, "{name} must be empty");
    }
    assert_eq!(fields["NoNewPrivs"].trim(), "1");
}

#[tokio::test]
#[ignore = "needs the actual selected engine"]
async fn engine_doctor_reads_local_process_and_low_port_prerequisites() {
    let socket = selected_socket().await;
    let directory = tempfile::tempdir().unwrap();
    let pki = support::pki(directory.path());
    let config = directory.path().join("doctor.toml");
    let path = |value: &std::path::Path| serde_json::to_string(value.to_str().unwrap()).unwrap();
    std::fs::write(
        &config,
        format!(
            r#"
node_id = "engine-doctor"
deployment_id = "engine-doctor"
state_dir = {}
[api]
listen = "127.0.0.1:0"
[api.tls]
cert = {}
key = {}
client_ca = {}
allowed_clients = [{}]
[docker]
socket = {}
"#,
            path(&directory.path().join("state")),
            path(&pki.dir.join("node.pem")),
            path(&pki.dir.join("node.key")),
            path(&pki.dir.join("ca.pem")),
            serde_json::to_string(support::CLIENT).unwrap(),
            path(&socket)
        ),
    )
    .unwrap();
    let report = pryxd::doctor::run(&pryxd::doctor::Options { config, preflight: false, fix: false }).await;
    let report = serde_json::to_value(report).unwrap();
    let checks = report["checks"].as_array().unwrap();
    let named = |name| checks.iter().find(|check| check["name"] == name);
    if std::env::var_os("PRYX_TEST_ENGINE").is_some() {
        assert!(named("docker.exec-pid-visibility").is_some(), "{report}");
        assert_eq!(named("docker.exec-pid-visibility").unwrap()["status"], "pass");
        let first: u16 =
            std::fs::read_to_string("/proc/sys/net/ipv4/ip_unprivileged_port_start").unwrap().trim().parse().unwrap();
        let ports = named("docker.low-ports").unwrap();
        assert_eq!(ports["status"], if first <= 1 { "pass" } else { "warn" });
        if first > 1 {
            assert!(!ports["fix"].as_str().unwrap().is_empty());
        }
    } else {
        assert!(named("docker.exec-pid-visibility").is_none());
        assert!(named("docker.low-ports").is_none());
    }
}
