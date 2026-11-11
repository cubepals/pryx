// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Docker Engine API over the local unix socket (bollard). pryxd is the only client that
//! needs the socket, which is root-equivalent: whoever holds it can start a privileged container
//! with the host's filesystem mounted. pryxd never exposes it; it exposes the narrow
//! workload protocol instead, and fills every container's isolation from host policy.
//!
//! Parts (`docker/`):
//! - `egress.rs`: firewall-gated hostname bridges and gateways.
//! - `ownership.rs`: read-only proof of host data identity mapping.
//! - `podman.rs`: native Netavark isolation and hardened container creation.
//! - `network.rs`: engine bridge creation and isolation observation.
//! - `create.rs`: trusted container and agent creation.
//! - `container.rs`: container construction and observation types.
//! - `exec.rs`: command attachment and timeout containment.
//! - `native_tools.rs`: explicit host-only seccomp and FUSE device policy.
//! - `tests.rs`: container construction and host process ownership regressions.
//! - `policy_tests.rs`: transport regression for unsupported engine policy refusal.
//! - `relay.rs`: bounded private ingress to gateway-free internal networks.

mod container;
mod create;
pub(crate) mod egress;
mod exec;
mod native_tools;
mod network;
pub(crate) mod ownership;
mod podman;
#[cfg(test)]
mod policy_tests;
mod relay;
use container::{body_of, info_from};

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::errors::Error as BollardError;
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bollard::models::{
    ContainerUpdateBody, NetworkCreateRequest, RestartPolicy as DockerRestartPolicy, RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, EventsOptionsBuilder, InspectNetworkOptions,
    KillContainerOptionsBuilder, ListContainersOptionsBuilder, LogsOptionsBuilder, RemoveContainerOptionsBuilder,
    StatsOptionsBuilder, StopContainerOptionsBuilder,
};
use futures_util::stream::{BoxStream, StreamExt};
use hyper::StatusCode;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use super::{
    ContainerInfo, ContainerRuntime, ContainerSpec, ContainerStatus, ExecOutput, LogChunk, LogOptions, LogStream,
    NetworkReport, RawStats, RuntimeError, RuntimeEvent, RuntimeInfo,
};
use crate::protocol::MAX_STOP_TIMEOUT;

/// A rootless engine must map its workload identity to the host's protected data owner.
pub const DATA_OWNER_REQUIREMENT: &str = "data-owner-mapping";
/// Longer than the longest stop: Docker answers a stop only once the container exited.
const CLIENT_TIMEOUT_SECS: u64 = MAX_STOP_TIMEOUT as u64 + 60;
/// Docker Engine /info SecurityOptions uses this identifier for rootless mode.
const ROOTLESS_SECURITY_OPTION: &str = "name=rootless";
/// Podman v6.1.0 pkg/api/handlers/compat/version.go's component name.
const PODMAN_ENGINE_COMPONENT: &str = "Podman Engine";
pub(crate) const PODMAN_POLICY_PROBLEM: &str = "Podman needs its native API, Netavark and the default native-tools profile to preserve container isolation; use a supported engine configuration";
/// Quick calls get a much shorter leash of their own.
const QUICK: Duration = Duration::from_secs(20);
const ICC_OPTION: &str = "com.docker.network.bridge.enable_icc";
/// The bridge driver's option naming the network's interface on the host (Docker's documentation,
/// "Bridge network driver", options).
const BRIDGE_NAME_OPTION: &str = "com.docker.network.bridge.name";
/// What the bridge driver names an interface left unnamed: this, then the first 12 characters of
/// the network's id (libnetwork's bridge driver).
const DOCKER_BRIDGE_PREFIX: &str = "br-";
/// Linux's longest interface name: IFNAMSIZ, 16, less the closing NUL.
const IFNAME_MAX: usize = 15;
/// How long a timed-out exec's process is looked for. Docker answers the start before the process
/// exists, and reports pid 0 until it does.
const EXEC_PID_WAIT: Duration = Duration::from_secs(2);
const EXEC_PID_STEP: Duration = Duration::from_millis(50);

/// The interface pryxd names the bridge of the workloads' network `network`, named for the
/// network so the host's rules (isolation.rs) can name it before it exists, and two networks on one
/// host never share one.
pub fn bridge_name(network: &str) -> String {
    let prefix = "bly";
    let digest = hex::encode(Sha256::digest(network.as_bytes()));
    format!("{prefix}{}", &digest[..IFNAME_MAX - prefix.len()])
}

pub(crate) fn internal_network(network: &str) -> String {
    format!("{network}-internal")
}

pub struct DockerRuntime {
    socket: String,
    client: std::sync::Mutex<Option<Docker>>,
    info: std::sync::Mutex<Option<RuntimeInfo>>,
    relays: relay::Relays,
    egress: egress::Egress,
    native_tools: native_tools::Policy,
    pub(super) ownership: ownership::Policy,
}

impl DockerRuntime {
    /// Doesn't connect yet: pryxd starts even while Docker is down (bollard refuses to build a
    /// client for a socket that doesn't exist yet), and says so until it's back.
    pub fn new(socket: &Path) -> Result<Self, RuntimeError> {
        let socket = socket.to_str().ok_or_else(|| RuntimeError::Other("socket path is not UTF-8".into()))?.to_owned();
        Ok(Self {
            socket,
            client: std::sync::Mutex::new(None),
            info: std::sync::Mutex::new(None),
            relays: relay::Relays::default(),
            egress: Default::default(),
            native_tools: Default::default(),
            ownership: Default::default(),
        })
    }

    pub fn with_native_tools(
        mut self,
        profile: Option<&Path>,
        fuse: bool,
        apparmor: Option<&str>,
    ) -> Result<Self, RuntimeError> {
        self.native_tools = native_tools::Policy::load(profile, fuse, apparmor)?;
        Ok(self)
    }

    /// Prove the independent cgroup watchdog before enabling the optional host feature.
    pub async fn check_store_lease(&self) -> Result<(), RuntimeError> {
        let info = self.info().await?;
        if info.rootless
            || info.cgroup_version.as_deref() != Some("2")
            || info.cgroup_driver.as_deref() != Some("cgroupfs")
        {
            return Err(RuntimeError::Other("store-lease needs rootful cgroup v2 and Docker cgroupfs driver".into()));
        }
        crate::store_lease::guard::prepare("probe", 0)
            .map_err(|_| RuntimeError::Other("store-lease cgroups are not writable".into()))?;
        let mut probe = crate::store_lease::guard::Guard::start(
            "probe",
            0,
            crate::store_lease::guard::boot_ms().map_err(|_| RuntimeError::Other("Boot clock unavailable".into()))?
                + 5000,
        )
        .map_err(|_| RuntimeError::Other("Cannot protect independent lease watchdog".into()))?;
        probe.fence().map_err(|_| RuntimeError::Other("Cannot freeze lease cgroup".into()))?;
        Ok(())
    }
    fn docker(&self) -> Result<Docker, RuntimeError> {
        let mut client = self.client.lock().unwrap();
        if let Some(docker) = client.as_ref() {
            return Ok(docker.clone());
        }
        let docker = Docker::connect_with_unix(&self.socket, CLIENT_TIMEOUT_SECS, bollard::API_DEFAULT_VERSION)
            .map_err(map_err)?;
        *client = Some(docker.clone());
        Ok(docker)
    }

    /// The interface on the host of the network `name`: the one its options name, or else the one
    /// Docker named for its id. None if there is no such network.
    pub async fn bridge_of(&self, name: &str) -> Result<Option<String>, RuntimeError> {
        match quick("inspect network", self.docker()?.inspect_network(name, None::<InspectNetworkOptions>)).await {
            Ok(network) => Ok(network.options.unwrap_or_default().remove(BRIDGE_NAME_OPTION).or_else(|| {
                network.id.map(|id| {
                    format!("{DOCKER_BRIDGE_PREFIX}{}", &id[..id.len().min(IFNAME_MAX - DOCKER_BRIDGE_PREFIX.len())])
                })
            })),
            Err(RuntimeError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The host pid of an exec and its container's id. Pid 0 means not started yet, so it looks
    /// again for a short while; None if no process showed up, or the exec ended without one.
    async fn exec_pid(&self, exec_id: &str) -> Option<(i32, String)> {
        let deadline = tokio::time::Instant::now() + EXEC_PID_WAIT;
        loop {
            let inspect = quick("inspect exec", self.docker().ok()?.inspect_exec(exec_id)).await.ok()?;
            let pid = inspect.pid.unwrap_or(0);
            if pid > 0 {
                return Some((i32::try_from(pid).ok()?, inspect.container_id?));
            }
            if inspect.exit_code.is_some() || tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(EXEC_PID_STEP).await;
        }
    }
}

/// What Docker says when a start can't bind a host port, which it answers with no code of its own,
/// only a 500. Another container holds it: moby's port allocator (`ErrPortAlreadyAllocated`,
/// "Bind for 127.0.0.1:42000 failed: port is already allocated"). Another process holds it: Go's
/// text for EADDRINUSE ("failed to bind host port 127.0.0.1:42000/tcp: address already in use").
/// tests/docker.rs has Docker say both.
const PORT_TAKEN: [&str; 2] = ["port is already allocated", "address already in use"];

fn map_err(error: BollardError) -> RuntimeError {
    match error {
        BollardError::DockerResponseServerError { status_code, message } => match StatusCode::from_u16(status_code) {
            Ok(StatusCode::NOT_FOUND) => RuntimeError::NotFound(message),
            Ok(StatusCode::CONFLICT) => RuntimeError::Conflict(message),
            _ if PORT_TAKEN.iter().any(|said| message.contains(said)) => {
                RuntimeError::PortTaken { status: status_code, message }
            }
            _ => RuntimeError::Rejected { status: status_code, message },
        },
        BollardError::RequestTimeoutError => RuntimeError::Timeout("the Docker API didn't answer in time".into()),
        e @ (BollardError::IOError { .. }
        | BollardError::HyperResponseError { .. }
        | BollardError::HyperLegacyError { .. }
        | BollardError::SocketNotFoundError(_)) => RuntimeError::Unavailable(e.to_string()),
        other => RuntimeError::Other(other.to_string()),
    }
}

async fn native_request<T: serde::de::DeserializeOwned>(
    runtime: &DockerRuntime,
    version: &str,
    method: reqwest::Method,
    path: &[&str],
    body: Option<Vec<u8>>,
) -> Result<T, RuntimeError> {
    let client = reqwest::Client::builder()
        .unix_socket(runtime.socket.as_str())
        .no_proxy()
        .timeout(QUICK)
        .build()
        .map_err(|_| RuntimeError::Other("Cannot construct native API transport".into()))?;
    let mut url = reqwest::Url::parse("http://localhost").expect("literal native endpoint");
    url.path_segments_mut()
        .expect("literal HTTP URL has path segments")
        .push(&format!("v{version}"))
        .push("libpod")
        .extend(path);
    let response = client
        .request(method, url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.unwrap_or_default())
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                RuntimeError::Timeout("Podman native API timed out".into())
            } else {
                RuntimeError::Unavailable("Podman native API transport failed".into())
            }
        })?;
    if !response.status().is_success() {
        return Err(map_err(BollardError::DockerResponseServerError {
            status_code: response.status().as_u16(),
            message: "Podman rejected native container policy".into(),
        }));
    }
    let bytes =
        response.bytes().await.map_err(|_| RuntimeError::Unavailable("Podman native API response was lost".into()))?;
    serde_json::from_slice(&bytes).map_err(|_| RuntimeError::Other("Podman native API response was invalid".into()))
}

async fn quick<T>(
    what: &str,
    fut: impl std::future::Future<Output = Result<T, BollardError>>,
) -> Result<T, RuntimeError> {
    match tokio::time::timeout(QUICK, fut).await {
        Ok(result) => result.map_err(map_err),
        Err(_) => Err(RuntimeError::Timeout(format!("{what} took longer than {}s", QUICK.as_secs()))),
    }
}

fn filters(label_filters: &[String]) -> HashMap<String, Vec<String>> {
    HashMap::from([("label".to_owned(), label_filters.to_vec())])
}

/// The tag to pull a reference by: "latest" for a name with neither tag nor digest, which is what
/// inspect reads it as, where Docker would otherwise pull every tag. Only then: a tag passed with
/// one already named replaces it. A registry's port ("reg:5000/x") is no tag, so only the last
/// path segment is looked at.
fn pull_tag(image: &str) -> Option<&'static str> {
    let last = image.rsplit('/').next().unwrap_or(image);
    (!image.contains('@') && !last.contains(':')).then_some("latest")
}

#[async_trait]
impl ContainerRuntime for DockerRuntime {
    fn data_owner(&self, _id: &str) -> Result<Option<(u32, u32)>, RuntimeError> {
        Ok(self
            .info
            .lock()
            .unwrap()
            .as_ref()
            .filter(|info| info.rootless)
            .map(|_| (rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw())))
    }
    fn supports_agent(&self) -> bool {
        true
    }
    fn supports_native_fuse(&self) -> bool {
        self.native_tools.fuse()
            && self.info.lock().unwrap().as_ref().is_some_and(|info| !info.rootless && !info.podman)
    }
    fn supports_hostname_egress(&self) -> bool {
        self.info.lock().unwrap().as_ref().is_some_and(|info| !info.rootless && !info.podman) && egress::ready()
    }
    fn supports_network_policy(&self) -> bool {
        true
    }

    async fn ensure_internal_network(&self, name: &str, labels: &BTreeMap<String, String>) -> Result<(), RuntimeError> {
        network::internal(self, name, labels).await
    }
    async fn resource_limits(&self) -> Result<Option<super::limits::ResourceLimits>, RuntimeError> {
        let info = self.info().await?;
        if let Some(problem) = info.container_policy_problem {
            return Err(RuntimeError::Other(problem));
        }
        Ok(Some(super::limits::ResourceLimits {
            memory: info.memory_limit,
            cpu: info.cpu_limit,
            pids: info.pids_limit,
            rootless: info.rootless,
        }))
    }
    async fn info(&self) -> Result<RuntimeInfo, RuntimeError> {
        // The maintained client negotiates the compatible API; its clones share that version.
        // Independent metadata reads run together within the existing discovery deadline.
        let client = self.docker()?;
        crate::capability::probe_phase("engine-negotiate");
        crate::capability::probe_phase("engine-version");
        let (_, version) = tokio::try_join!(
            quick("negotiate version", client.clone().negotiate_version()),
            quick("version", client.version()),
        )?;
        let podman = version
            .components
            .as_ref()
            .is_some_and(|components| components.iter().any(|component| component.name == PODMAN_ENGINE_COMPONENT));
        let podman_version = version
            .components
            .as_ref()
            .and_then(|components| components.iter().find(|component| component.name == PODMAN_ENGINE_COMPONENT))
            .map(|component| component.version.clone())
            .filter(|version| {
                !version.is_empty()
                    && version.split('.').count() == 3
                    && version.split('.').all(|part| part.parse::<u32>().is_ok())
            });
        crate::capability::probe_phase("engine-compatible-info");
        crate::capability::probe_phase("engine-native-info");
        let native =
            async { if let Some(version) = &podman_version { podman::host(self, version).await } else { None } };
        let (info, native) = tokio::join!(quick("info", client.info()), native);
        let info = info?;
        let mut result = RuntimeInfo {
            podman,
            podman_version,
            container_policy_problem: (podman && native.is_none()).then(|| PODMAN_POLICY_PROBLEM.into()),
            rootless: info
                .security_options
                .as_ref()
                .is_some_and(|options| options.iter().any(|option| option == ROOTLESS_SECURITY_OPTION)),
            memory_limit: info.memory_limit == Some(true)
                && info.cgroup_driver.as_ref().is_some_and(|driver| driver.to_string() != "none"),
            cpu_limit: info.cpu_cfs_quota == Some(true)
                && info.cpu_shares == Some(true)
                && info.cgroup_driver.as_ref().is_some_and(|driver| driver.to_string() != "none"),
            pids_limit: info.pids_limit == Some(true)
                && info.cgroup_driver.as_ref().is_some_and(|driver| driver.to_string() != "none"),
            version: version.version,
            api_version: version.api_version,
            cgroup_version: info.cgroup_version.map(|v| v.to_string()),
            cgroup_driver: info.cgroup_driver.map(|v| v.to_string()),
            storage_driver: info.driver,
            security_options: info.security_options.unwrap_or_default(),
            live_restore: info.live_restore_enabled,
            log_driver: info.logging_driver,
        };
        podman::enforcement(&mut result, native.as_ref());
        crate::capability::probe_phase("engine-info-complete");
        if result.container_policy_problem.is_none() {
            result.container_policy_problem = ownership::problem(self, &result).await;
        }
        *self.info.lock().unwrap() = Some(result.clone());
        Ok(result)
    }

    async fn serving_info(&self) -> Result<RuntimeInfo, RuntimeError> {
        let info = self.info().await?;
        if let Some(problem) = &info.container_policy_problem {
            return Err(RuntimeError::Unavailable(problem.clone()));
        }
        Ok(info)
    }

    async fn ensure_network(
        &self,
        name: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<NetworkReport, RuntimeError> {
        network::ensure(self, name, labels).await
    }

    async fn network_isolated(&self, name: &str) -> Result<Option<bool>, RuntimeError> {
        network::isolated(self, name).await
    }

    async fn ensure_image(&self, image: &str, timeout: Duration) -> Result<bool, RuntimeError> {
        match quick("inspect image", self.docker()?.inspect_image(image)).await {
            Ok(_) => return Ok(false),
            Err(RuntimeError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        let mut options = CreateImageOptionsBuilder::new().from_image(image);
        if let Some(tag) = pull_tag(image) {
            options = options.tag(tag);
        }
        let options = options.build();
        let pull = async {
            let mut stream = self.docker()?.create_image(Some(options), None, None);
            while let Some(item) = stream.next().await {
                // bollard hands an errorDetail in the progress stream over as a DockerStreamError.
                item.map_err(|e| match e {
                    BollardError::DockerStreamError { error } => {
                        RuntimeError::Other(format!("pulling {image}: {error}"))
                    }
                    other => map_err(other),
                })?;
            }
            Ok(())
        };
        match tokio::time::timeout(timeout, pull).await {
            Ok(result) => result.map(|()| true),
            Err(_) => Err(RuntimeError::Timeout(format!("pulling {image} took longer than {}s", timeout.as_secs()))),
        }
    }

    async fn create(&self, spec: &ContainerSpec) -> Result<String, RuntimeError> {
        create::create(self, spec).await
    }

    async fn start(&self, name: &str) -> Result<(), RuntimeError> {
        if let Err(error) = quick("start", self.docker()?.start_container(name, None)).await {
            // A lost response does not undo Docker's mutation. Confirm its current state once;
            // never issue a second start, and preserve the timeout unless it is already running.
            if matches!(error, RuntimeError::Timeout(_))
                && matches!(self.inspect(name).await, Ok(Some(info)) if info.status == ContainerStatus::Running)
            {
                tracing::warn!("confirmed running after Docker start timeout without repeating start");
                return Ok(());
            }
            return Err(error);
        }
        self.inspect(name).await?;
        Ok(())
    }

    async fn stop(&self, name: &str, signal: &str, grace: Duration) -> Result<(), RuntimeError> {
        let options =
            StopContainerOptionsBuilder::new().signal(signal).t(grace.as_secs().min(i32::MAX as u64) as i32).build();
        // Docker answers once the container exited: the grace, plus a margin for the kill.
        let budget = grace + Duration::from_secs(30);
        match tokio::time::timeout(budget, self.docker()?.stop_container(name, Some(options))).await {
            Ok(result) => result.map_err(map_err),
            Err(_) => Err(RuntimeError::Timeout(format!("stopping {name} took longer than {}s", budget.as_secs()))),
        }
    }

    async fn kill(&self, name: &str) -> Result<(), RuntimeError> {
        let options = KillContainerOptionsBuilder::new().signal("SIGKILL").build();
        match quick("kill", self.docker()?.kill_container(name, Some(options))).await {
            // A conflict: not running. Nothing left to kill.
            Err(RuntimeError::Conflict(_)) => Ok(()),
            other => other,
        }
    }

    async fn pause(&self, name: &str) -> Result<(), RuntimeError> {
        quick("pause", self.docker()?.pause_container(name)).await
    }

    async fn unpause(&self, name: &str) -> Result<(), RuntimeError> {
        quick("unpause", self.docker()?.unpause_container(name)).await
    }

    async fn disable_restart(&self, name: &str) -> Result<(), RuntimeError> {
        let body = ContainerUpdateBody {
            restart_policy: Some(DockerRestartPolicy {
                name: Some(RestartPolicyNameEnum::NO),
                maximum_retry_count: None,
            }),
            ..Default::default()
        };
        quick("update", self.docker()?.update_container(name, body)).await
    }

    async fn remove(&self, name: &str) -> Result<(), RuntimeError> {
        self.relays.stop(name).await;
        let network = if self.info.lock().unwrap().as_ref().is_some_and(|info| info.podman) {
            self.inspect(name).await?.and_then(|info| info.labels.get(podman::NETWORK_LABEL).cloned())
        } else {
            None
        };
        // v: anonymous volumes an image declares go with the container instead of leaking; the
        // workload's data is a bind mount, which removal never touches.
        let options = RemoveContainerOptionsBuilder::new().force(true).v(true).build();
        match quick("remove", self.docker()?.remove_container(name, Some(options))).await {
            Err(RuntimeError::NotFound(_)) | Ok(()) => {
                self.egress.stop(self, name).await?;
                if let Some(network) = network {
                    match podman::delete_network(self, &network).await {
                        Ok(_) | Err(RuntimeError::NotFound(_)) => {}
                        Err(error) => return Err(error),
                    }
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    async fn inspect(&self, name: &str) -> Result<Option<ContainerInfo>, RuntimeError> {
        match quick("inspect", self.docker()?.inspect_container(name, None)).await {
            Ok(inspect) => {
                self.egress.restore(self, &inspect).await?;
                self.relays.ensure(&inspect).await?;
                Ok(Some(info_from(inspect)))
            }
            Err(RuntimeError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn list(&self, label_filters: &[String]) -> Result<Vec<ContainerInfo>, RuntimeError> {
        let options = ListContainersOptionsBuilder::new().all(true).filters(&filters(label_filters)).build();
        let summaries = quick("list", self.docker()?.list_containers(Some(options))).await?;
        let mut out = Vec::with_capacity(summaries.len());
        for summary in summaries {
            let Some(id) = summary.id else { continue };
            // The summary has no exit code or health; inspect is local and cheap.
            if let Some(info) = self.inspect(&id).await? {
                out.push(info);
            }
        }
        Ok(out)
    }

    async fn exec(
        &self,
        container: &str,
        command: &[String],
        timeout: Duration,
        max_output: usize,
    ) -> Result<ExecOutput, RuntimeError> {
        self.exec_command(container, command, timeout, max_output, None).await
    }

    async fn exec_stream(
        &self,
        container: &str,
        command: &[String],
        timeout: Duration,
        max_output: usize,
        output: tokio::sync::mpsc::Sender<LogChunk>,
    ) -> Result<ExecOutput, RuntimeError> {
        self.exec_command(container, command, timeout, max_output, Some(output)).await
    }

    fn logs(&self, name: &str, options: LogOptions) -> BoxStream<'static, Result<LogChunk, RuntimeError>> {
        let mut builder = LogsOptionsBuilder::new().stdout(true).stderr(true).timestamps(true).follow(options.follow);
        builder = builder.tail(&options.tail.map_or_else(|| "all".to_owned(), |n| n.to_string()));
        if let Some(since) = options.since_unix {
            builder = builder.since(since.clamp(0, i64::from(i32::MAX)) as i32);
        }
        let docker = match self.docker() {
            Ok(docker) => docker,
            Err(e) => return futures_util::stream::iter([Err(e)]).boxed(),
        };
        let stream = docker.logs(name, Some(builder.build()));
        stream
            .filter_map(|item| async move {
                match item {
                    Ok(LogOutput::StdOut { message }) => {
                        Some(Ok(LogChunk { stream: LogStream::Stdout, bytes: message }))
                    }
                    Ok(LogOutput::StdErr { message }) => {
                        Some(Ok(LogChunk { stream: LogStream::Stderr, bytes: message }))
                    }
                    Ok(LogOutput::Console { message }) => {
                        Some(Ok(LogChunk { stream: LogStream::Stdout, bytes: message }))
                    }
                    Ok(LogOutput::StdIn { .. }) => None,
                    Err(e) => Some(Err(map_err(e))),
                }
            })
            .boxed()
    }

    async fn stats(&self, name: &str) -> Result<Option<RawStats>, RuntimeError> {
        let options = StatsOptionsBuilder::new().stream(false).one_shot(true).build();
        let mut stream = self.docker()?.stats(name, Some(options));
        let first = match tokio::time::timeout(QUICK, stream.next()).await {
            Ok(Some(Ok(stats))) => stats,
            Ok(Some(Err(e))) => {
                return match map_err(e) {
                    RuntimeError::NotFound(_) => Ok(None),
                    other => Err(other),
                };
            }
            Ok(None) => return Ok(None),
            Err(_) => return Err(RuntimeError::Timeout("stats".into())),
        };
        let cpu = first.cpu_stats.unwrap_or_default();
        let memory = first.memory_stats.unwrap_or_default();
        let stats = memory.stats.unwrap_or_default();
        // cgroup v2 names it inactive_file, v1 total_inactive_file.
        let inactive = stats.get("inactive_file").or_else(|| stats.get("total_inactive_file")).copied();
        let (rx, tx) = first
            .networks
            .unwrap_or_default()
            .values()
            .fold((0u64, 0u64), |(rx, tx), n| (rx + n.rx_bytes.unwrap_or(0), tx + n.tx_bytes.unwrap_or(0)));
        let pids = first.pids_stats.unwrap_or_default();
        Ok(Some(RawStats {
            cpu_total_ns: cpu.cpu_usage.and_then(|u| u.total_usage),
            cpu_throttled_periods: cpu.throttling_data.and_then(|t| t.throttled_periods),
            memory_usage: memory.usage,
            memory_inactive_file: inactive,
            memory_limit: memory.limit,
            pids: pids.current,
            pids_limit: pids.limit,
            rx_bytes: Some(rx),
            tx_bytes: Some(tx),
        }))
    }

    fn events(&self, label_filters: &[String]) -> BoxStream<'static, Result<RuntimeEvent, RuntimeError>> {
        let mut filters = filters(label_filters);
        filters.insert("type".into(), vec!["container".into()]);
        let options = EventsOptionsBuilder::new().filters(&filters).build();
        let docker = match self.docker() {
            Ok(docker) => docker,
            Err(e) => return futures_util::stream::iter([Err(e)]).boxed(),
        };
        docker
            .events(Some(options))
            .map(|item| {
                let message = item.map_err(map_err)?;
                let actor = message.actor.unwrap_or_default();
                let at = message
                    .time_nano
                    .and_then(|ns| OffsetDateTime::from_unix_timestamp_nanos(i128::from(ns)).ok())
                    .unwrap_or_else(OffsetDateTime::now_utc);
                Ok(RuntimeEvent {
                    container_id: actor.id.unwrap_or_default(),
                    action: message.action.unwrap_or_default(),
                    exit_code: actor.attributes.as_ref().and_then(|a| a.get("exitCode")).and_then(|c| c.parse().ok()),
                    at,
                })
            })
            .boxed()
    }
}

/// SIGKILLs `pid` and every process descended from it, each only if its cgroup still names
/// `container` (a pid may have been reused since the exec began). Returns how many it killed.
fn kill_exec_tree(pid: i32, container: &str) -> usize {
    let mut killed = 0;
    for target in descendants(pid, &children_by_parent()) {
        let owned = std::fs::read_to_string(format!("/proc/{target}/cgroup")).is_ok_and(|c| c.contains(container));
        if owned
            && let Some(p) = rustix::process::Pid::from_raw(target)
            && rustix::process::kill_process(p, rustix::process::Signal::KILL).is_ok()
        {
            killed += 1;
        }
    }
    killed
}

/// Every process's children, keyed by parent pid, as /proc shows them now.
fn children_by_parent() -> HashMap<i32, Vec<i32>> {
    let mut parents: HashMap<i32, Vec<i32>> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.filter_map(Result::ok) {
            let Ok(child) = entry.file_name().to_string_lossy().parse::<i32>() else { continue };
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{child}/stat")) else { continue };
            // The command name is parenthesised and may hold spaces; fields resume after ')'.
            let Some(after) = stat.rfind(')').map(|i| &stat[i + 1..]) else { continue };
            let Some(ppid) = after.split_whitespace().nth(1).and_then(|p| p.parse::<i32>().ok()) else { continue };
            parents.entry(ppid).or_default().push(child);
        }
    }
    parents
}

/// `root` and everything below it, breadth first. Nothing for 0 or 1: the tree under either is
/// every process on the host, the container's own included, and the cgroup check would let
/// exactly those through. Docker reports pid 0 for an exec it hasn't started yet.
fn descendants(root: i32, children: &HashMap<i32, Vec<i32>>) -> Vec<i32> {
    if root <= 1 {
        return Vec::new();
    }
    let mut tree = vec![root];
    let mut i = 0;
    while i < tree.len() {
        if let Some(below) = children.get(&tree[i]) {
            tree.extend(below);
        }
        i += 1;
    }
    tree
}

#[cfg(test)]
mod tests;
