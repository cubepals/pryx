// SPDX-License-Identifier: FSL-1.1-ALv2
//! Podman's native network and create adapters. Bollard continues to own its compatible
//! lifecycle and streams. Host firewall, FUSE and custom native-tool profiles are not adapted.
use super::*;
use oci_spec::runtime::{Mount, MountBuilder};
use podman_api_stubs::models;
use serde::{Serialize, de::DeserializeOwned};

// containers/common/libnetwork/types/network.go exports IsolateOption with this value.
const ISOLATE_OPTION: &str = "isolate";
// containers/common/libnetwork/types/network.go's BridgeNetworkDriver and
// containers/common/pkg/config/config.go's SystemdCgroupsManager.
const BRIDGE_DRIVER: &str = "bridge";
const SYSTEMD_CGROUP_MANAGER: &str = "systemd";
// Podman libpod/info.go's native network backend name.
const NETAVARK_BACKEND: &str = "netavark";
// Podman pkg/specgen/specgen.go documents the cgroups mode and bounded file log driver.
const DISABLED_CGROUPS_MODE: &str = "disabled";
const FILE_LOG_DRIVER: &str = "k8s-file";
// libpod/define/container.go's RestartPolicyNo; --cap-drop's documented all-capabilities token.
const NEVER_RESTART: &str = "no";
const DROP_ALL_CAPABILITIES: &str = "ALL";
// Podman pkg/specgen/namespaces.go's NamespaceMode values.
const PRIVATE_NAMESPACE: &str = "private";
const BRIDGE_NAMESPACE: &str = "bridge";
const KEEP_ID_NAMESPACE: &str = "keep-id";
// Linux cgroup v1 cpu.cfs_period_us's documented default, also used with cgroup v2 cpu.max.
const CPU_PERIOD_MICROS: u64 = 100_000;
pub(super) const NETWORK_LABEL: &str = "blocklyd.podman-network";

pub(super) fn version(runtime: &DockerRuntime) -> Result<String, RuntimeError> {
    runtime
        .info
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|info| info.podman_version.clone())
        .ok_or_else(|| RuntimeError::Other("Podman native API version is unavailable".into()))
}

pub(super) async fn selected(runtime: &DockerRuntime) -> Result<bool, RuntimeError> {
    let cached = runtime.info.lock().unwrap().clone();
    let info = match cached {
        Some(info) => info,
        None => runtime.info().await?,
    };
    if let Some(problem) = info.container_policy_problem {
        return Err(RuntimeError::Other(problem));
    }
    Ok(info.podman)
}

pub(super) async fn host(runtime: &DockerRuntime, version: &str) -> Option<models::HostInfo> {
    if runtime.native_tools.custom() {
        return None;
    }
    let info: models::Info = native_request(runtime, version, reqwest::Method::GET, &["info"], None).await.ok()?;
    info.host.filter(|host| host.network_backend.as_deref() == Some(NETAVARK_BACKEND))
}

pub(super) fn enforcement(info: &mut RuntimeInfo, host: Option<&models::HostInfo>) {
    if !info.podman {
        return;
    }
    let managed =
        host.is_some_and(|host| !info.rootless || host.cgroup_manager.as_deref() == Some(SYSTEMD_CGROUP_MANAGER));
    let controllers = host.and_then(|host| host.cgroup_controllers.as_ref());
    let has = |controller: &str| {
        managed && controllers.is_some_and(|controllers| controllers.iter().any(|c| c == controller))
    };
    info.memory_limit &= has("memory");
    info.cpu_limit &= has("cpu");
    info.pids_limit &= has("pids");
}

pub(super) async fn network(
    runtime: &DockerRuntime,
    name: &str,
    labels: &BTreeMap<String, String>,
    internal: bool,
) -> Result<(), RuntimeError> {
    let version = version(runtime)?;
    let inspect: Result<models::Network, _> =
        native_request(runtime, &version, reqwest::Method::GET, &["networks", name, "json"], None).await;
    match inspect {
        Ok(network) => {
            let owned = network
                .labels
                .as_ref()
                .is_some_and(|actual| labels.iter().all(|(key, value)| actual.get(key) == Some(value)));
            if network.driver.as_deref() == Some(BRIDGE_DRIVER)
                && network.internal == Some(internal)
                && network
                    .options
                    .as_ref()
                    .is_some_and(|options| options.get(ISOLATE_OPTION).map(String::as_str) == Some("true"))
                && owned
            {
                return Ok(());
            }
            return Err(RuntimeError::Other("Podman network does not enforce the owned isolation policy".into()));
        }
        Err(RuntimeError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    let model = models::Network {
        name: Some(name.into()),
        driver: Some(BRIDGE_DRIVER.into()),
        internal: Some(internal),
        dns_enabled: Some(!internal),
        labels: Some(labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        options: Some(HashMap::from([(ISOLATE_OPTION.into(), "true".into())])),
        ..empty()?
    };
    let body =
        serde_json::to_vec(&model).map_err(|_| RuntimeError::Other("Cannot encode native network policy".into()))?;
    let _: models::Network =
        native_request(runtime, &version, reqwest::Method::POST, &["networks", "create"], Some(body)).await?;
    let observed: models::Network =
        native_request(runtime, &version, reqwest::Method::GET, &["networks", name, "json"], None).await?;
    if observed.options.as_ref().and_then(|options| options.get(ISOLATE_OPTION)).map(String::as_str) != Some("true") {
        return Err(RuntimeError::Other("Podman did not retain its bridge isolation policy".into()));
    }
    Ok(())
}

/// The SDK's SpecGenerator predates read_write_tmpfs and has a conflicting Docker Mount
/// definition. Keep every other field in its generated owner type and import OCI's mount type.
#[derive(Serialize)]
pub(super) struct Create {
    #[serde(flatten)]
    spec: models::SpecGenerator,
    read_write_tmpfs: bool,
    mounts: Vec<Mount>,
}

fn empty<T: DeserializeOwned>() -> Result<T, RuntimeError> {
    serde_json::from_value(serde_json::json!({}))
        .map_err(|_| RuntimeError::Other("Cannot construct native API model".into()))
}

fn namespace(mode: &str, value: Option<String>) -> models::Namespace {
    models::Namespace { nsmode: Some(mode.into()), value }
}

pub(super) fn network_name(name: &str) -> String {
    format!("pryx-c-{}", &hex::encode(Sha256::digest(name.as_bytes()))[..24])
}

pub(super) fn body(spec: &ContainerSpec, info: &RuntimeInfo) -> Result<Create, RuntimeError> {
    let mut model: models::SpecGenerator = empty()?;
    model.name = Some(spec.name.clone());
    model.image = Some(spec.image.clone());
    model.entrypoint = spec.entrypoint.clone();
    if spec.entrypoint.is_some() {
        model.command = Some(Vec::new());
    }
    model.env = Some(
        spec.env
            .iter()
            .filter(|(key, _)| key != pryx_agent::contract::FUSE_ENV)
            .cloned()
            .chain([(pryx_agent::contract::FUSE_ENV.into(), "0".into())])
            .collect(),
    );
    model.env_host = Some(false);
    model.httpproxy = Some(false);
    model.user = Some(spec.user.clone());
    model.work_dir = spec.workdir.clone();
    model.read_only_filesystem = Some(spec.read_only_rootfs);
    model.cap_drop = Some(vec![DROP_ALL_CAPABILITIES.into()]);
    model.no_new_privileges = Some(true);
    model.privileged = Some(false);
    model.ipcns = Some(namespace(PRIVATE_NAMESPACE, None));
    model.pidns = Some(namespace(PRIVATE_NAMESPACE, None));
    model.utsns = Some(namespace(PRIVATE_NAMESPACE, None));
    model.netns = Some(namespace(BRIDGE_NAMESPACE, None));
    let network = network_name(&spec.name);
    model.networks = Some(HashMap::from([(network.clone(), empty()?)]));
    model.labels = Some(
        spec.labels
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .chain([(NETWORK_LABEL.into(), network)])
            .filter(|(key, _)| key != crate::labels::LABEL_INTERNAL_BINDINGS)
            .collect(),
    );
    if info.rootless {
        let (uid, gid) = spec
            .user
            .split_once(':')
            .ok_or_else(|| RuntimeError::Other("Podman needs numeric workload UID:GID".into()))?;
        model.userns = Some(namespace(KEEP_ID_NAMESPACE, Some(format!("uid={uid},gid={gid}"))));
    }
    model.restart_policy = Some(NEVER_RESTART.into());
    model.oom_score_adj = Some(i64::from(spec.oom_score_adj));
    model.stop_timeout = Some(u64::from(spec.stop_timeout_secs));
    let signal: crate::protocol::StopSignal = serde_json::from_value(spec.stop_signal.clone().into())
        .map_err(|_| RuntimeError::Other("Unsupported stop signal".into()))?;
    model.stop_signal = Some(i64::from(match signal {
        crate::protocol::StopSignal::SIGTERM => libc::SIGTERM,
        crate::protocol::StopSignal::SIGINT => libc::SIGINT,
    }));
    model.log_configuration = Some(models::LogConfigLibpod {
        driver: Some(FILE_LOG_DRIVER.into()),
        size: Some(i64::from(spec.log_max_size_mb) * 1024 * 1024),
        options: None,
        path: None,
    });
    model.resource_limits = Some(resources(spec, info)?);
    if info.rootless && !info.memory_limit && !info.cpu_limit && !info.pids_limit {
        model.cgroups_mode = Some(DISABLED_CGROUPS_MODE.into());
    }
    model.portmappings = Some(
        spec.ports
            .iter()
            .flat_map(|port| {
                port.host_ips.iter().map(move |ip| models::PortMapping {
                    container_port: Some(port.container_port),
                    host_ip: Some(ip.to_string()),
                    host_port: Some(port.host_port),
                    protocol: Some(port.protocol.as_str().into()),
                    range: Some(1),
                })
            })
            .collect(),
    );
    Ok(Create { spec: model, read_write_tmpfs: false, mounts: mounts(spec)? })
}

fn resources(spec: &ContainerSpec, info: &RuntimeInfo) -> Result<models::LinuxResources, RuntimeError> {
    let mut limits: models::LinuxResources = empty()?;
    if info.memory_limit {
        limits.memory =
            Some(models::LinuxMemory { limit: Some(spec.memory_bytes), swap: Some(spec.memory_bytes), ..empty()? });
    }
    if info.cpu_limit {
        limits.cpu = Some(models::LinuxCpu {
            shares: Some(u64::try_from(spec.cpu_shares).map_err(|_| RuntimeError::Other("Invalid CPU shares".into()))?),
            period: spec.nano_cpus.map(|_| CPU_PERIOD_MICROS),
            quota: spec
                .nano_cpus
                .map(|nanos| nanos * CPU_PERIOD_MICROS as i64 / Duration::from_secs(1).as_nanos() as i64),
            ..empty()?
        });
    }
    if info.pids_limit {
        limits.pids = Some(models::LinuxPids { limit: Some(spec.pids_limit) });
    }
    Ok(limits)
}

fn mount(source: &Path, destination: &str, read_only: bool) -> Result<Mount, RuntimeError> {
    MountBuilder::default()
        .typ("bind")
        .source(source)
        .destination(destination)
        .options(vec!["rbind".into(), if read_only { "ro" } else { "rw" }.into(), "nosuid".into(), "nodev".into()])
        .build()
        .map_err(|_| RuntimeError::Other("Cannot construct OCI bind mount".into()))
}

fn mounts(spec: &ContainerSpec) -> Result<Vec<Mount>, RuntimeError> {
    let mut mounts = vec![mount(&spec.data_dir, &spec.mount_path, false)?];
    if let Some(agent) = &spec.agent {
        mounts.push(mount(&agent.binary, pryx_agent::contract::BINARY_PATH, true)?);
    }
    if spec.read_only_rootfs {
        mounts.push(
            MountBuilder::default()
                .typ("tmpfs")
                .source("tmpfs")
                .destination("/tmp")
                .options(vec![
                    "rw".into(),
                    "exec".into(),
                    "nosuid".into(),
                    "nodev".into(),
                    format!("size={}m", spec.tmp_size_mb),
                ])
                .build()
                .map_err(|_| RuntimeError::Other("Cannot construct OCI temporary mount".into()))?,
        );
    }
    Ok(mounts)
}

pub(super) async fn create(
    runtime: &DockerRuntime,
    spec: &ContainerSpec,
    info: &RuntimeInfo,
) -> Result<String, RuntimeError> {
    network(runtime, &network_name(&spec.name), &spec.labels, spec.internal_network).await?;
    let body = serde_json::to_vec(&body(spec, info)?)
        .map_err(|_| RuntimeError::Other("Cannot encode native container policy".into()))?;
    native_create(runtime, body).await
}

pub(super) async fn isolated(runtime: &DockerRuntime, name: &str) -> Result<Option<bool>, RuntimeError> {
    match native_request::<models::Network>(
        runtime,
        &version(runtime)?,
        reqwest::Method::GET,
        &["networks", name, "json"],
        None,
    )
    .await
    {
        Ok(network) => Ok(Some(
            network.driver.as_deref() == Some(BRIDGE_DRIVER)
                && network.options.as_ref().and_then(|options| options.get(ISOLATE_OPTION)).map(String::as_str)
                    == Some("true"),
        )),
        Err(RuntimeError::NotFound(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) async fn delete_network(runtime: &DockerRuntime, name: &str) -> Result<(), RuntimeError> {
    let reports: Vec<models::NetworkRmReport> =
        native_request(runtime, &version(runtime)?, reqwest::Method::DELETE, &["networks", name], None).await?;
    if reports.iter().any(|report| report.err.is_some()) {
        return Err(RuntimeError::Other("Podman did not remove the unused workload network".into()));
    }
    Ok(())
}

async fn native_create(runtime: &DockerRuntime, body: Vec<u8>) -> Result<String, RuntimeError> {
    let created: podman_api_stubs::models::ContainerCreateResponse = native_request(
        runtime,
        &podman::version(runtime)?,
        reqwest::Method::POST,
        &["containers", "create"],
        Some(body),
    )
    .await?;
    if created.id.is_empty() {
        return Err(RuntimeError::Other("Podman native create returned no identity".into()));
    }
    Ok(created.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_policy_preserves_read_only_mounts_nonzero_ids_and_available_limits() {
        let mut spec = super::super::tests::spec();
        spec.env.push((pryx_agent::contract::FUSE_ENV.into(), "1".into()));
        let info = RuntimeInfo {
            podman: true,
            rootless: true,
            memory_limit: true,
            cpu_limit: true,
            pids_limit: true,
            ..Default::default()
        };
        let value = serde_json::to_value(body(&spec, &info).unwrap()).unwrap();
        assert_eq!(value["user"], spec.user);
        assert_eq!(value["userns"]["nsmode"], KEEP_ID_NAMESPACE);
        assert_eq!(
            value["userns"]["value"],
            format!("uid={},gid={}", spec.user.split(':').next().unwrap(), spec.user.split(':').nth(1).unwrap())
        );
        assert_eq!(value["read_only_filesystem"], true);
        assert_eq!(value["read_write_tmpfs"], false);
        assert_eq!(value["cap_drop"], serde_json::json!(["ALL"]));
        assert_eq!(value["no_new_privileges"], true);
        assert_eq!(value["privileged"], false);
        assert_eq!(value["env"][pryx_agent::contract::FUSE_ENV], "0");
        assert_eq!(value["resource_limits"]["memory"]["limit"], spec.memory_bytes);
        assert_eq!(value["resource_limits"]["memory"]["swap"], spec.memory_bytes);
        let mounts = value["mounts"].as_array().unwrap();
        assert_eq!(mounts.iter().filter(|mount| mount["destination"] == spec.mount_path).count(), 1);
        assert!(mounts.iter().all(|mount| mount.get("Target").is_none()));
        assert!(mounts.iter().all(|mount| mount["destination"] != "/run" && mount["destination"] != "/var/tmp"));
        assert!(mounts.iter().any(|mount| mount["destination"] == "/tmp" && mount["type"] == "tmpfs"));
        let without = RuntimeInfo { podman: true, rootless: true, ..Default::default() };
        let value = serde_json::to_value(body(&spec, &without).unwrap()).unwrap();
        assert!(value["resource_limits"]["memory"].is_null());
        assert!(value["resource_limits"]["cpu"].is_null());
        assert!(value["resource_limits"]["pids"].is_null());
    }

    #[tokio::test]
    async fn a_native_create_rejection_never_echoes_request_secrets_or_repeats_the_post() {
        use axum::{Json, Router, http::StatusCode, routing::post};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("native.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let app = Router::new().route(
            "/{*path}",
            post(move |Json(body): Json<serde_json::Value>| {
                seen.fetch_add(1, Ordering::SeqCst);
                async move { (StatusCode::BAD_REQUEST, Json(body)) }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let runtime = DockerRuntime::new(&path).unwrap();
        let versions: serde_json::Value =
            serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/.github/engine-versions.json")))
                .unwrap();
        let native_version = versions["podman"]["version"].as_str().unwrap().split('+').next().unwrap().into();
        *runtime.info.lock().unwrap() =
            Some(RuntimeInfo { podman: true, podman_version: Some(native_version), ..Default::default() });
        let secret = "native-create-secret-must-stay-private";
        let error =
            native_create(&runtime, serde_json::to_vec(&serde_json::json!({"env": {"SECRET": secret}})).unwrap())
                .await
                .unwrap_err();
        assert!(matches!(error, RuntimeError::Rejected { status, .. } if status == StatusCode::BAD_REQUEST.as_u16()));
        assert!(!error.to_string().contains(secret));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        server.abort();
    }
}
