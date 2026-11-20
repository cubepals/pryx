// SPDX-License-Identifier: FSL-1.1-ALv2
//! Construct the host-configured backend. Fleet enrollment and server setup stay in main.
use super::{ContainerRuntime, docker::DockerRuntime};
use crate::capability::{Capability, State};
use crate::config::{Config, RuntimeKind};
use std::collections::BTreeMap;
use std::sync::Arc;

/// One construction's backends and the original reasons it omitted other tiers. These are
/// construction observations, not a new probe or a claim about a backend's later availability.
pub(crate) struct Construction {
    pub(crate) runtime: Arc<dyn ContainerRuntime>,
    pub(crate) unavailable: BTreeMap<&'static str, String>,
}

pub async fn create(config: &Config) -> anyhow::Result<Arc<dyn ContainerRuntime>> {
    if config.runtime == RuntimeKind::Auto {
        return Ok(auto(config).await?.runtime);
    }
    let runtime: Arc<dyn ContainerRuntime> = if config.runtime == RuntimeKind::Vm {
        if rustix::process::geteuid().is_root() {
            // Root's VM authority is the separate worker's; this daemon holds only its lifetime.
            Arc::new(super::vm::worker::frontend::Frontend::installed(&config.state_dir).await?)
        } else {
            let runtime =
                super::vm::instance::VmRuntime::new(&config.state_dir, super::vm::policy::settings(config)?).await?;
            // The tier announces memory sleep only once this host's restore test has passed.
            runtime.probe_memory_sleep_in_background();
            Arc::new(runtime)
        }
    } else if config.runtime == RuntimeKind::Wasm {
        Arc::new(crate::runtime::wasm::WasmRuntime::new(&config.state_dir, &config.wasm).await?)
    } else if config.runtime == RuntimeKind::Process {
        let served = crate::runtime::process::Served {
            ports: config.network.port_range[0]..=config.network.port_range[1],
            api_port: config.api.listen.port(),
        };
        Arc::new(
            // Limits first: the daemon may move into its own leaf only before it starts any child.
            crate::runtime::process::ProcessRuntime::new(&config.state_dir, config.process.clone())?
                .with_configured_agent(config.workloads.agent_binary.clone())
                .with_resources()
                .with_network(served)
                .await,
        )
    } else {
        anyhow::ensure!(config.docker.enabled, "the container tier is disabled by this host preset");
        let socket = super::engine::socket(&config.docker).await?;
        let docker = container(config, &socket)?;
        if let Ok(Ok(info)) = tokio::time::timeout(super::engine::PROBE_TIMEOUT, docker.info()).await
            && let Some(problem) = info.container_policy_problem
        {
            anyhow::bail!(problem);
        }
        if config.store_lease.enabled {
            docker.check_store_lease().await?;
            crate::store_lease::check_policy(&config.state_dir, true)?;
        }
        Arc::new(docker)
    };
    Ok(runtime)
}

/// The same construction used by `create`, keeping its initial off reasons for a refusal's
/// diagnosis. A second probe could replace the evidence that caused this construction to omit it.
pub(crate) async fn create_with_reasons(config: &Config) -> anyhow::Result<Construction> {
    if config.runtime == RuntimeKind::Auto {
        auto(config).await
    } else {
        Ok(Construction { runtime: create(config).await?, unavailable: BTreeMap::new() })
    }
}

/// Probe prerequisites without starting compute or mutating a host.
pub async fn availability(config: &Config) -> std::collections::BTreeMap<&'static str, Option<String>> {
    let mut available = host_availability(config).await;
    if available["process"].is_none()
        && let Err(reason) =
            super::agent_metadata::require_configured_process_agent(&config.workloads.agent_binary).await
    {
        available.insert("process", Some(reason.into()));
    }
    available
}

async fn host_availability(config: &Config) -> std::collections::BTreeMap<&'static str, Option<String>> {
    let process = if matches!(config.runtime, RuntimeKind::Auto | RuntimeKind::Process) {
        process_available(None)
    } else {
        Capability::new(State::Off, "configuration selects another tier", "Set runtime = auto")
    };
    let docker = async {
        let socket = super::engine::socket(&config.docker).await?;
        let docker = container(config, &socket).map_err(|error| super::RuntimeError::Other(error.to_string()))?;
        tokio::time::timeout(super::engine::PROBE_TIMEOUT, docker.info())
            .await
            .map_err(|_| super::RuntimeError::Unavailable("container policy probe timed out".into()))?
    };
    let engine = if !matches!(config.runtime, RuntimeKind::Auto | RuntimeKind::Container) {
        Err("configuration selects another tier".to_string())
    } else if !config.docker.enabled {
        Err("disabled by this host preset; use the server preset for host engine access".into())
    } else {
        docker.await.map_err(|error| error.to_string())
    };
    let container =
        engine.as_ref().map_or_else(|error| Some(error.clone()), |info| info.container_policy_problem.clone());
    // gVisor is the container backend with another engine runtime, so it needs that tier first.
    let gvisor = if config.runtime != RuntimeKind::Auto {
        Some("configuration selects another tier".into())
    } else {
        container.clone().or_else(|| engine.as_ref().ok().and_then(super::docker::gvisor_problem))
    };
    std::collections::BTreeMap::from([
        ("container", container),
        (crate::protocol::Isolation::Gvisor.as_str(), gvisor),
        ("process", (process.state != State::On).then_some(process.evidence)),
        ("vm", {
            if !matches!(config.runtime, RuntimeKind::Auto | RuntimeKind::Vm) {
                Some("configuration selects another tier".into())
            } else if rustix::process::geteuid().is_root() {
                super::vm::worker::frontend::Frontend::available(&config.state_dir)
                    .err()
                    .map(|problem| problem.to_string())
            } else {
                super::vm::policy::settings(config).err().map(|problem| problem.to_string())
            }
        }),
        ("wasm", {
            if matches!(config.runtime, RuntimeKind::Auto | RuntimeKind::Wasm) {
                let wasm = wasm_available(&config.wasm);
                (wasm.state != State::On).then_some(wasm.evidence)
            } else {
                Some("configuration selects another tier".into())
            }
        }),
    ])
}

async fn auto(config: &Config) -> anyhow::Result<Construction> {
    let mut selected = config.clone();
    let mut engine_error = None;
    if config.docker.enabled {
        match super::engine::socket(&config.docker).await {
            Ok(socket) => selected.docker.socket = socket,
            Err(error) => {
                engine_error = Some(error.to_string());
                selected.docker.enabled = false;
            }
        }
        selected.docker.discover = false;
    }
    let config = &selected;
    // Keep process journals available for existing lifecycle operations when the installed
    // supervisor cannot start new compute. Its runtime reports that admission failure in info().
    let mut available = host_availability(config).await;
    if let Some(reason) = engine_error {
        available.insert("container", Some(reason));
    }
    let mut backends = Vec::new();
    let mut reasons = Vec::new();
    let mut unavailable = BTreeMap::new();
    for tier in ["container", crate::protocol::Isolation::Gvisor.as_str(), "process", "wasm", "vm"] {
        if let Some(reason) = &available[tier] {
            unavailable.insert(tier, reason.clone());
            reasons.push(format!("{tier}: {reason}"));
            continue;
        }
        if tier == crate::protocol::Isolation::Gvisor.as_str() {
            // The same engine and host policy as the container tier, with runsc for each start.
            match container(config, &config.docker.socket) {
                Ok(docker) => backends.push(Arc::new(docker.gvisor()) as Arc<dyn ContainerRuntime>),
                Err(error) => {
                    unavailable.insert(tier, error.to_string());
                    reasons.push(format!("{tier}: {error}"));
                }
            }
            continue;
        }
        let mut single = config.clone();
        single.runtime = match tier {
            "container" => RuntimeKind::Container,
            "process" => RuntimeKind::Process,
            "vm" => RuntimeKind::Vm,
            _ => RuntimeKind::Wasm,
        };
        // Boxing closes only the construction recursion; workload dispatch remains the router.
        match Box::pin(create(&single)).await {
            Ok(runtime) => backends.push(runtime),
            Err(error) => {
                unavailable.insert(tier, error.to_string());
                reasons.push(format!("{tier}: {error}"));
            }
        }
    }
    for reason in &reasons {
        tracing::info!(reason, "isolation tier off");
    }
    if backends.is_empty() {
        anyhow::bail!("No isolation tier is available: {}", reasons.join("; "));
    }
    Ok(Construction { runtime: Arc::new(super::router::Router::new(backends)?), unavailable })
}

fn container(config: &Config, socket: &std::path::Path) -> anyhow::Result<DockerRuntime> {
    let mut docker = DockerRuntime::new(socket)?.with_oom_journal(&config.state_dir).with_native_tools(
        config.docker.native_tools_seccomp.as_deref(),
        config.docker.native_fuse,
        config.docker.native_tools_apparmor.as_deref(),
    )?;
    docker = docker.with_internal_ingress_agent(&config.workloads.agent_binary);
    docker.ownership =
        super::docker::ownership::Policy { user: config.workloads.user, owner: config.workloads.data_owner };
    Ok(docker)
}

pub(crate) fn process_available(separate: Option<&Capability>) -> Capability {
    if !pryx_agent::process_capabilities::probe().filesystem {
        return Capability::new(
            State::Off,
            pryx_agent::process_capabilities::FILESYSTEM_UNAVAILABLE,
            "Use a host whose policy permits Landlock ABI 3 filesystem confinement",
        );
    }
    if rustix::process::geteuid().is_root() {
        let measured = separate.cloned().unwrap_or_else(|| crate::capability::child("probe-separate-users"));
        if measured.state != State::On {
            return measured;
        }
    }
    Capability::on("Landlock ABI 3 filesystem confinement is enforceable")
}

pub(crate) fn wasm_available(config: &crate::config::WasmConfig) -> Capability {
    if rustix::process::geteuid().is_root() {
        let users = crate::capability::child("probe-separate-users");
        if users.state != State::On {
            return users;
        }
    }
    let library = crate::capability::c_library().to_ascii_lowercase();
    if std::env::consts::ARCH != "x86_64"
        || !(library.contains("glibc") || library.contains("gnu libc") || library.contains("gnu c library"))
    {
        return Capability::new(
            State::Off,
            "The current agentOS distribution needs an x86_64 GNU host",
            "Use process on a Landlock host or install a compatible supported distribution",
        );
    }
    crate::capability::probe_phase("wasm-distribution");
    let distribution = crate::runtime::wasm::check_distribution(config);
    crate::capability::probe_phase("wasm-distribution-complete");
    match distribution {
        Ok(()) => Capability::on("The pinned agentOS binary and packages are verified"),
        Err(error) => {
            Capability::new(State::Off, error.to_string(), "Install the pinned agentOS distribution for this host")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        manager::{Manager, Records},
        metrics::Metrics,
        runtime::RuntimeError,
        store::Store,
    };
    use pryx_agent::process_environment::{UNSUPPORTED, VERSION};
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    #[tokio::test]
    async fn configured_process_availability_refuses_legacy_agent() {
        if rustix::process::geteuid().is_root() {
            return;
        }
        let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let binary = directory.path().join("agent");
        fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agent-metadata"), &binary).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let metadata_path = binary.with_extension("json");
        let legacy = r#"{"browserPolicy":true}"#;
        fs::write(&metadata_path, legacy).unwrap();
        assert_eq!(crate::runtime::protected_agent(&binary, true).as_ref(), Some(&binary));
        let metadata = crate::runtime::agent_metadata::query(&binary).await.unwrap();
        assert!(metadata.process_environment_version < VERSION);
        assert_eq!(crate::runtime::agent_metadata::require_configured_process_agent(&binary).await, Err(UNSUPPORTED));
        let filesystem = pryx_agent::process_capabilities::probe().filesystem;

        let mut config: Config = toml::from_str(
            r#"
runtime = "process"
deployment_id = "legacy-agent-test"
[api]
listen = "127.0.0.1:0"
"#,
        )
        .unwrap();
        config.state_dir = directory.path().join("state");
        config.workloads.agent_binary = binary.clone();

        // A Landlock host can reach the supervisor-specific refusal.
        if filesystem {
            let tiers = availability(&config).await;
            assert_eq!(
                tiers["process"].as_deref(),
                Some(UNSUPPORTED),
                "configured process availability must report the launch refusal for older agents"
            );
        }

        let runtime = Arc::new(
            super::super::process::ProcessRuntime::new(&config.state_dir, config.process.clone())
                .unwrap()
                .with_configured_agent(binary),
        );
        if filesystem {
            assert!(matches!(runtime.info().await, Err(RuntimeError::Unavailable(reason)) if reason == UNSUPPORTED));
        }
        let store = Store::open(&config.state_dir).unwrap();
        let manager = Manager::new(Arc::new(config), runtime, store, Arc::new(Metrics::new()), Arc::new(|_, _| true));
        let reconcile = manager.reconcile(Records::FromDisk).await;
        assert_eq!(reconcile.error, None);
        assert!(manager.reconciled());
        assert!(!manager.docker_up(), "complete inventory must not admit placement on an unready process runtime");
        let current = serde_json::json!({"browserPolicy":true,"processEnvironmentVersion":VERSION}).to_string();
        for (advertised, metadata) in [(false, legacy), (true, current.as_str()), (false, legacy)] {
            if advertised && !filesystem {
                break;
            }
            fs::write(&metadata_path, metadata).unwrap();
            let health = manager.health().await;
            assert_eq!(health.docker, advertised);
            assert!(health.features.iter().any(|feature| feature == "process-tier"));
            for feature in ["tier-process", "agent"] {
                assert_eq!(health.features.iter().any(|entry| entry == feature), advertised, "{feature}");
            }
        }
    }
}
