// SPDX-License-Identifier: FSL-1.1-ALv2
//! Live engine metadata and ownership observation. Container operations and policy application
//! stay in their own Docker backend modules; no engine capability is inferred from a version.
use super::*;

impl DockerRuntime {
    pub(super) async fn metadata(&self) -> Result<RuntimeInfo, RuntimeError> {
        // The maintained client negotiates the compatible API; its clones share that version.
        // Independent metadata reads run together within the existing discovery deadline.
        let client = self.docker()?;
        crate::capability::probe_phase("engine-negotiate");
        crate::capability::probe_phase("engine-version");
        let cached = self.metadata_version.lock().unwrap().clone();
        let version = if let Some(version) = &cached {
            version.clone()
        } else {
            tokio::try_join!(
                quick("negotiate version", client.clone().negotiate_version()),
                quick("version", client.version()),
            )?
            .1
        };
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
        let info = self.checked_metadata(cached.is_some(), &version.version, info, native.is_some())?;
        // Only stable version negotiation is reused. Controllers, ownership and policy stay live.
        if podman && native.is_some() && version.version.is_some() && version.version == info.server_version {
            *self.metadata_version.lock().unwrap() = Some(version.clone());
        }
        let runtimes: Vec<String> = info.runtimes.clone().unwrap_or_default().into_keys().collect();
        let mut result = RuntimeInfo {
            own_kernel: false,
            runtimes,
            podman,
            podman_version,
            container_policy_problem: (podman && native.is_none()).then(|| PODMAN_POLICY_PROBLEM.into()),
            rootless: info
                .security_options
                .as_ref()
                .is_some_and(|options| options.iter().any(|option| option == ROOTLESS_SECURITY_OPTION)),
            local_process_ids: None,
            unprivileged_ports: None,
            private_ingress: None,
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
            default_runtime: info.default_runtime,
            memory_sleep: false,
        };
        podman::enforcement(&mut result, native.as_ref());
        if self.oci_runtime.is_some() {
            gvisor::enforcement(&mut result);
        }
        let result = self.observed_policies(result).await;
        *self.info.lock().unwrap() = Some(result.clone());
        Ok(result)
    }

    async fn observed_policies(&self, mut result: RuntimeInfo) -> RuntimeInfo {
        if result.rootless {
            let visible = termination::peer(&self.socket).await.is_ok();
            result.local_process_ids = Some(visible);
            // ip(7): ports below ip_unprivileged_port_start require CAP_NET_BIND_SERVICE.
            // The host-side rootless port helper needs all nonzero ports available.
            result.unprivileged_ports = visible
                .then(|| {
                    std::fs::read_to_string("/proc/sys/net/ipv4/ip_unprivileged_port_start")
                        .ok()
                        .and_then(|value| value.trim().parse::<u16>().ok())
                        .map(|first| first <= 1)
                })
                .flatten();
        }
        crate::capability::probe_phase("engine-info-complete");
        if result.container_policy_problem.is_none() {
            result.container_policy_problem = ownership::problem(self, &result).await;
        }
        if result.rootless && !result.podman && result.container_policy_problem.is_none() {
            result.private_ingress = Some(self.ingress.available().await);
        }
        if result.container_policy_problem.is_none() && self.oci_runtime.is_some() {
            result.container_policy_problem = gvisor::problem(&result);
        }
        result
    }
}
