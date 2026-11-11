// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! A workload as the protocol shows it: its state, ports, storage and issues, built from what the
//! core last recorded. It never asks the runtime; looking at a container is `observe`, in
//! `manager.rs`, and how a record and a container make a state is `derive_state`, in
//! `workload_state.rs`.

use super::*;

impl Manager {
    pub(super) fn state_of(&self, record: &WorkloadRecord, info: Option<&ContainerInfo>) -> WorkloadState {
        let state = self.state.lock().unwrap();
        self.state_in(&state, record, info)
    }

    /// The state as every view, count and heartbeat reports it.
    fn state_in(&self, state: &State, record: &WorkloadRecord, info: Option<&ContainerInfo>) -> WorkloadState {
        let id = &record.id;
        let sight = if self.runtime.multi_tier() && !self.runtime.tier_ready(self.record_tier(record)) {
            Sight::Stale
        } else {
            self.sight()
        };
        pending_restart(state, id, derive_state(record, info, state.stopping(id), state.suspend(id), sight))
    }

    // ─── views ─────────────────────────────────────────────────────────────────────────────

    pub fn view(&self, id: &WorkloadId) -> Result<WorkloadView, NodeError> {
        let state = self.state.lock().unwrap();
        let record = state.records.get(id).ok_or_else(|| NodeError::NotFound(id.clone()))?;
        let info = state.observed.get(id).and_then(|o| o.info.as_ref());
        Ok(self.build_view(&state, record, info))
    }

    fn workload_guarantees(
        &self,
        record: &WorkloadRecord,
        observed: Option<&ContainerInfo>,
    ) -> BTreeMap<crate::protocol::Guarantee, bool> {
        use crate::protocol::Guarantee as G;
        let tier = self.record_tier(record);
        let mut guarantees = self.tier_guarantees(tier);
        let rootless = if self.runtime.multi_tier() {
            self.runtime.tier_info(tier).is_some_and(|info| info.rootless)
        } else {
            self.runtime_info.lock().unwrap().as_ref().is_some_and(|info| info.rootless)
        };
        if tier == "container" && rootless {
            for (guarantee, configured) in [
                (G::MemoryLimit, observed.and_then(|info| info.memory_enforced)),
                (G::CpuLimit, observed.and_then(|info| info.cpu_enforced)),
            ] {
                guarantees.entry(guarantee).and_modify(|on| *on &= configured == Some(true));
            }
        }
        guarantees
    }

    fn build_view(&self, state: &State, record: &WorkloadRecord, info: Option<&ContainerInfo>) -> WorkloadView {
        let workload_state = self.state_in(state, record, info);
        let exit = info
            .filter(|i| matches!(i.status, ContainerStatus::Exited | ContainerStatus::Dead))
            .map(|i| ExitInfo { code: i.exit_code, oom_killed: i.oom_killed, at: i.finished_at.map(format_time) });
        let restart_count = record.restart_count;
        let last_failure_at = state
            .last_failure
            .get(&record.id)
            .copied()
            .or_else(|| info.filter(|i| i.restart_count > 0).and_then(|i| i.started_at))
            .map(format_time);
        let changed = info
            .and_then(|i| match (i.started_at, i.finished_at) {
                (Some(s), Some(f)) => Some(s.max(f)),
                (s, f) => s.or(f),
            })
            .map(format_time)
            .unwrap_or_else(|| record.updated_at.clone());
        let ports = record
            .ports
            .iter()
            .filter(|p| !p.name.starts_with(pryx_agent::contract::INTERNAL_PORT_PREFIX))
            .map(|p| {
                let audience =
                    record.spec.ports.iter().find(|s| s.name == p.name).map(|s| s.audience.clone()).unwrap_or_default();
                let mut endpoints = crate::protocol::Endpoints::default();
                if audience.contains(&Audience::Edge) {
                    endpoints.edge = self.config.network.edge_ips.iter().map(|ip| endpoint(ip, p.host_port)).collect();
                }
                if audience.contains(&Audience::Control) {
                    endpoints.control =
                        self.config.network.control_ips.iter().map(|ip| endpoint(ip, p.host_port)).collect();
                }
                PortView {
                    name: p.name.clone(),
                    protocol: p.protocol,
                    container_port: p.container_port,
                    host_port: p.host_port,
                    endpoints,
                }
            })
            .collect();
        let (used, measured) =
            state.disk.get(&record.id).map(|(b, at)| (Some(*b), Some(format_time(*at)))).unwrap_or_default();
        WorkloadView {
            guarantees: self.workload_guarantees(record, info),
            isolation: (self.record_tier(record) != "container").then(|| self.record_tier(record).into()),
            agent: self.agent_view(record, info),
            id: record.id.clone(),
            generation: record.generation,
            epoch: record.epoch,
            superseded_by: record.superseded_by,
            spec_digest: record.spec_digest.clone(),
            pending_spec_digest: state
                .suspend
                .get(&record.id)
                .and_then(|s| s.pending.as_ref())
                .map(|p| p.spec_digest.clone()),
            image: record.spec.image.clone(),
            state: workload_state,
            health: info.filter(|i| i.status == ContainerStatus::Running).and_then(|i| i.health),
            exit,
            restart_count,
            last_failure_at,
            started_at: info.and_then(|i| i.started_at).map(format_time),
            finished_at: info.and_then(|i| i.finished_at).map(format_time),
            changed_at: changed,
            ports,
            resources: record.spec.resources.clone(),
            storage: StorageView {
                mount_path: record.spec.storage.mount_path.clone(),
                size_gb: record.spec.storage.size_gb,
                used_bytes: used,
                measured_at: measured,
            },
            labels: record.spec.labels.clone(),
            secret_names: record.spec.secret_names.clone(),
            issues: state
                .issues
                .get(&record.id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .chain(self.store_lease_issue(record))
                .collect(),
            locate: Locate {
                container_name: record.container_name.clone(),
                container_id: info.map(|i| i.id.clone()).or_else(|| record.container_id.clone()),
                data_dir: self.store.data_dir(&record.id).to_string_lossy().into_owned(),
            },
            created_at: record.created_at.clone(),
            updated_at: record.updated_at.clone(),
        }
    }

    /// Every workload, or those whose state changed at or after `since` (the control plane's
    /// `observeChanged`: level-triggered, answered from one local listing).
    pub(crate) fn list(&self, since: Option<OffsetDateTime>) -> Vec<WorkloadView> {
        let state = self.state.lock().unwrap();
        state
            .records
            .values()
            .map(|r| {
                let info = state.observed.get(&r.id).and_then(|o| o.info.as_ref());
                self.build_view(&state, r, info)
            })
            .filter(|v| match since {
                None => true,
                Some(since) => {
                    parse_time(Some(&v.changed_at)).is_none_or(|t| t >= since)
                        || parse_time(Some(&v.updated_at)).is_none_or(|t| t >= since)
                }
            })
            .collect()
    }

    pub(crate) fn workloads_by_state(&self) -> BTreeMap<WorkloadState, u64> {
        let state = self.state.lock().unwrap();
        let mut counts: BTreeMap<WorkloadState, u64> = WorkloadState::ALL.iter().map(|s| (*s, 0)).collect();
        for r in state.records.values() {
            let info = state.observed.get(&r.id).and_then(|o| o.info.as_ref());
            *counts.entry(self.state_in(&state, r, info)).or_default() += 1;
        }
        counts
    }
}

/// A failure pryxd will restart reads as restarting, not crashed: the host is starting it again,
/// as Docker's own policy would say.
fn pending_restart(state: &State, id: &WorkloadId, derived: WorkloadState) -> WorkloadState {
    let waiting = match derived {
        WorkloadState::Crashed => state.restart_due.contains_key(id) || state.resuming.contains(id),
        WorkloadState::Stopped => state.resuming.contains(id),
        _ => false,
    };
    if waiting { WorkloadState::Restarting } else { derived }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{protocol::Guarantee, runtime::fake::FakeRuntime};

    #[tokio::test]
    async fn gaining_controllers_does_not_upgrade_an_existing_unlimited_copy() {
        let directory = tempfile::tempdir().unwrap();
        let (uid, gid) = (rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw());
        let config: Config = toml::from_str(&format!(
            "deployment_id = \"test\"\nnode_id = \"node\"\nstate_dir = \"{}\"\n[api]\nlisten = \"127.0.0.1:0\"\n\
             [workloads]\nallowed_images = [\"alpine:\"]\ndata_owner = \"{uid}:{gid}\"\n\
             [capacity]\nmin_free_disk_mb = 1\n",
            directory.path().display()
        ))
        .unwrap();
        let runtime = Arc::new(FakeRuntime::new());
        runtime.runtime_info.lock().unwrap().rootless = true;
        let store = Store::open(&config.state_dir).unwrap();
        let manager =
            Manager::new(Arc::new(config), runtime.clone(), store, Arc::new(Metrics::new()), Arc::new(|_, _| true));
        assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
        let id = WorkloadId::parse("unlimited").unwrap();
        let spec = serde_json::from_value(serde_json::json!({
            "image": "alpine:3.22", "resources": {"memoryMb": 256, "allowUnenforcedMemory": true},
            "storage": {"mountPath": "/data", "sizeGb": 1}
        }))
        .unwrap();
        manager.ensure(id.clone(), spec, Precondition::None, None).await.unwrap();
        {
            let mut state = manager.state.lock().unwrap();
            let copy = state.observed.get_mut(&id).unwrap().info.as_mut().unwrap();
            copy.memory_enforced = Some(false);
            copy.cpu_enforced = Some(false);
        }
        {
            let mut info = manager.runtime_info.lock().unwrap();
            let info = info.as_mut().unwrap();
            info.memory_limit = true;
            info.cpu_limit = true;
        }
        let view = manager.view(&id).unwrap();
        assert!(!view.guarantees[&Guarantee::MemoryLimit], "the existing copy still has no memory ceiling");
        assert!(!view.guarantees[&Guarantee::CpuLimit], "the existing copy still has no CPU enforcement");
        let report =
            || crate::fleet::heartbeat::heartbeat_report(&manager, &crate::ids::NodeId::from("node"), "s", None, 1);
        let rootless_report = report();
        assert!(rootless_report.features.iter().any(|f| f == crate::protocol::OBSERVED_RESOURCE_LIMITS_FEATURE));
        assert_eq!(rootless_report.workloads[0].guarantees.as_ref(), Some(&view.guarantees));
        {
            let mut state = manager.state.lock().unwrap();
            let copy = state.observed.get_mut(&id).unwrap().info.as_mut().unwrap();
            copy.memory_enforced = Some(true);
            copy.cpu_enforced = Some(true);
        }
        let configured = manager.view(&id).unwrap();
        assert!(configured.guarantees[&Guarantee::MemoryLimit] && configured.guarantees[&Guarantee::CpuLimit]);
        assert_eq!(report().workloads[0].guarantees.as_ref(), Some(&configured.guarantees));
        {
            let mut state = manager.state.lock().unwrap();
            let copy = state.observed.get_mut(&id).unwrap().info.as_mut().unwrap();
            copy.memory_enforced = Some(false);
            copy.cpu_enforced = Some(false);
        }
        manager.runtime_info.lock().unwrap().as_mut().unwrap().rootless = false;
        let rootful = manager.view(&id).unwrap();
        assert!(
            rootful.guarantees[&Guarantee::MemoryLimit] && rootful.guarantees[&Guarantee::CpuLimit],
            "retain legacy rootful reporting"
        );
        let rootful_report = report();
        assert!(!rootful_report.features.iter().any(|f| f == crate::protocol::OBSERVED_RESOURCE_LIMITS_FEATURE));
        assert!(rootful_report.workloads[0].guarantees.is_none());
        assert!(serde_json::to_value(&rootful_report.workloads[0]).unwrap().get("guarantees").is_none());
    }
}
