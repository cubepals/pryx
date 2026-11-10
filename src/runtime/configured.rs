// SPDX-License-Identifier: FSL-1.1-ALv2
//! Construct the host-configured backend. Fleet enrollment and server setup stay in main.
use super::{ContainerRuntime, docker::DockerRuntime};
use crate::config::{Config, RuntimeKind};
use std::sync::Arc;

pub async fn create(config: &Config) -> anyhow::Result<Arc<dyn ContainerRuntime>> {
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
