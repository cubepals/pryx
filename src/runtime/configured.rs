// SPDX-License-Identifier: FSL-1.1-ALv2
//! Construct the host-configured backend. Fleet enrollment and server setup stay in main.
use super::{ContainerRuntime, docker::DockerRuntime};
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
        let docker = DockerRuntime::new(&config.docker.socket)?.with_native_tools(
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
    let process = pryx_agent::process_capabilities::probe();
    let docker = async {
        let runtime = DockerRuntime::new(&config.docker.socket)?;
        runtime.info().await
    };
    let container = match tokio::time::timeout(std::time::Duration::from_secs(2), docker).await {
        Ok(Ok(_)) => None,
        Ok(Err(error)) => Some(error.to_string()),
        Err(_) => Some("engine info exceeded two seconds".into()),
    };
    std::collections::BTreeMap::from([
        ("container", container),
        ("process", (!process.filesystem).then(|| pryx_agent::process_capabilities::FILESYSTEM_UNAVAILABLE.into())),
        ("wasm", super::wasm::check_distribution(&config.wasm).err().map(|error| error.to_string())),
    ])
}

async fn auto(config: &Config) -> anyhow::Result<Arc<dyn ContainerRuntime>> {
    let available = availability(config).await;
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
