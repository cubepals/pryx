// SPDX-License-Identifier: FSL-1.1-ALv2
//! Construct the host-configured backend. Fleet enrollment and server setup stay in main.
use super::{ContainerRuntime, docker::DockerRuntime};
use crate::capability::{Capability, State};
use crate::config::{Config, RuntimeKind};
use std::sync::Arc;

pub async fn create(config: &Config) -> anyhow::Result<Arc<dyn ContainerRuntime>> {
    if config.runtime == RuntimeKind::Auto {
        return auto(config).await;
    }
    let runtime: Arc<dyn ContainerRuntime> = if config.runtime == RuntimeKind::Wasm {
        Arc::new(crate::runtime::wasm::WasmRuntime::new(&config.state_dir, &config.wasm).await?)
    } else if config.runtime == RuntimeKind::Process {
        Arc::new(crate::runtime::process::ProcessRuntime::new(&config.state_dir, config.process.clone())?)
    } else {
        anyhow::ensure!(config.docker.enabled, "the container tier is disabled by this host preset");
        let socket = super::engine::socket(&config.docker).await?;
        let docker = DockerRuntime::new(&socket)?.with_native_tools(
            config.docker.native_tools_seccomp.as_deref(),
            config.docker.native_fuse,
            config.docker.native_tools_apparmor.as_deref(),
        )?;
        if config.store_lease.enabled {
            docker.check_store_lease().await?;
            crate::store_lease::check_policy(&config.state_dir, true)?;
        }
        Arc::new(docker)
    };
    Ok(runtime)
}

/// Probe prerequisites without starting compute or mutating a host.
pub async fn availability(config: &Config) -> std::collections::BTreeMap<&'static str, Option<String>> {
    let process = if matches!(config.runtime, RuntimeKind::Auto | RuntimeKind::Process) {
        process_available(None)
    } else {
        Capability::new(State::Off, "configuration selects another tier", "Set runtime = auto")
    };
    let docker = async {
        let socket = super::engine::socket(&config.docker).await?;
        super::engine::inspect(&socket).await
    };
    let container = if !matches!(config.runtime, RuntimeKind::Auto | RuntimeKind::Container) {
        Some("configuration selects another tier".into())
    } else if !config.docker.enabled {
        Some("disabled by this host preset; use the server preset for host engine access".into())
    } else {
        match docker.await {
            Ok(_) => None,
            Err(error) => Some(error.to_string()),
        }
    };
    std::collections::BTreeMap::from([
        ("container", container),
        ("process", (process.state != State::On).then_some(process.evidence)),
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

async fn auto(config: &Config) -> anyhow::Result<Arc<dyn ContainerRuntime>> {
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
    let mut available = availability(config).await;
    if let Some(reason) = engine_error {
        available.insert("container", Some(reason));
    }
    let mut backends = Vec::new();
    let mut reasons = Vec::new();
    for tier in ["container", "process", "wasm"] {
        if let Some(reason) = &available[tier] {
            reasons.push(format!("{tier}: {reason}"));
            continue;
        }
        let mut single = config.clone();
        single.runtime = match tier {
            "container" => RuntimeKind::Container,
            "process" => RuntimeKind::Process,
            _ => RuntimeKind::Wasm,
        };
        // Boxing closes only the construction recursion; workload dispatch remains the router.
        match Box::pin(create(&single)).await {
            Ok(runtime) => backends.push(runtime),
            Err(error) => reasons.push(format!("{tier}: {error}")),
        }
    }
    for reason in &reasons {
        tracing::info!(reason, "isolation tier off");
    }
    if backends.is_empty() {
        anyhow::bail!("No isolation tier is available: {}", reasons.join("; "));
    }
    Ok(Arc::new(super::router::Router::new(backends)?))
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
    match crate::runtime::wasm::check_distribution(config) {
        Ok(()) => Capability::on("The pinned agentOS binary and packages are verified"),
        Err(error) => {
            Capability::new(State::Off, error.to_string(), "Install the pinned agentOS distribution for this host")
        }
    }
}
