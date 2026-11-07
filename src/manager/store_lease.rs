// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The host's opt-in lease admission and ephemeral grants. Neither presigned URL reaches the workload record or view.
use super::*;
use crate::protocol::StoreLeaseIdentity;
use crate::protocol::{MIN_STORE_LEASE_SECONDS, STORE_LEASE_MARGIN_SECONDS, StoreLeaseGrant};
use crate::store_lease::Held;

impl Manager {
    pub fn features(&self) -> Vec<String> {
        let mut features = crate::protocol::features();
        if self.runtime.is_wasm() {
            features.extend(["wasm-tier".into(), "wasm-packages".into()]);
        }
        if self.runtime.is_process() {
            if pryx_agent::process_capabilities::probe().network {
                features.push("process-network-landlock".into());
            }
            features.extend(
                [
                    "process-tier",
                    "process-network-seccomp",
                    if rustix::process::geteuid().is_root() { "process-uid-pool" } else { "process-shared-uid" },
                ]
                .into_iter()
                .map(String::from),
            );
        }
        if self.agent_binary().is_some() {
            features.push("agent".into());
        }
        if self.runtime.supports_network_policy() {
            features.push("network-policy".into());
        }
        if self.config.store_lease.enabled {
            features.push("store-lease".into());
        }
        features
    }
    pub(super) fn require_store_lease(&self, record: &WorkloadRecord) -> Result<(), NodeError> {
        if !self.config.store_lease.enabled {
            return Ok(());
        }
        let parent = crate::store_lease::guard::docker_parent(record.id.as_str(), record.epoch.unwrap_or_default());
        let grouped = self
            .state
            .lock()
            .unwrap()
            .observed
            .get(&record.id)
            .and_then(|o| o.info.as_ref())
            .is_some_and(|i| i.cgroup_parent.as_deref() == Some(parent.as_str()));
        if !grouped {
            return Err(NodeError::Conflict {
                code: ConflictCode::StoreLeaseRequired,
                message: "PUT this spec again to apply this host's lease containment policy".into(),
            });
        }
        let held = self.store_leases.lock().unwrap();
        if held.get(&record.id).is_some_and(|h| Some(h.epoch) == record.epoch && h.live()) {
            return Ok(());
        }
        Err(NodeError::Conflict {
            code: ConflictCode::StoreLeaseRequired,
            message: "A live store lease is required before this host runs a sandbox".into(),
        })
    }
    pub(crate) fn store_lease_identity(
        &self,
        id: &WorkloadId,
        epoch: Option<u64>,
    ) -> Result<StoreLeaseIdentity, NodeError> {
        if !self.config.store_lease.enabled {
            return Err(NodeError::Conflict {
                code: ConflictCode::StoreLeaseRequired,
                message: "Store lease is disabled on this host".into(),
            });
        }
        check_epoch(&self.record(id)?, epoch, EpochRule::Exact)?;
        Ok(StoreLeaseIdentity { session: self.store_lease_session.clone() })
    }
    pub(crate) async fn grant_store_lease(
        self: &Arc<Self>,
        id: WorkloadId,
        epoch: Option<u64>,
        grant: StoreLeaseGrant,
    ) -> Result<(), NodeError> {
        if !self.config.store_lease.enabled {
            return Err(NodeError::Invalid(vec![FieldError {
                field: "storeLease".into(),
                problem: "store-lease is disabled on this host".into(),
            }]));
        }
        if grant.session != self.store_lease_session {
            return Err(NodeError::Conflict {
                code: ConflictCode::StoreLeaseRequired,
                message: "The grant belongs to a different daemon session".into(),
            });
        }
        if grant.seconds < MIN_STORE_LEASE_SECONDS
            || grant.seconds > self.config.store_lease.max_seconds
            || grant.renew_seconds == 0
            || grant.renew_seconds.saturating_mul(2) >= grant.seconds.saturating_sub(STORE_LEASE_MARGIN_SECONDS)
            || grant.generation.is_empty()
        {
            return Err(NodeError::Invalid(vec![FieldError {
                field: "storeLease".into(),
                problem: "term/renewal exceed this host's lease policy".into(),
            }]));
        }
        for url in [&grant.read_url, &grant.write_url] {
            if url.len() > 16 * 1024
                || url
                    .parse::<hyper::Uri>()
                    .ok()
                    .is_none_or(|u| !matches!(u.scheme_str(), Some("http" | "https")) || u.host().is_none())
            {
                return Err(NodeError::Invalid(vec![FieldError {
                    field: "storeLease.url".into(),
                    problem: "an http(s) object-store URL is required".into(),
                }]));
            }
        }
        let read_uri =
            grant.read_url.parse::<hyper::Uri>().map_err(|_| NodeError::Internal("Lease URL invalid".into()))?;
        let write_uri =
            grant.write_url.parse::<hyper::Uri>().map_err(|_| NodeError::Internal("Lease URL invalid".into()))?;
        if read_uri.scheme() != write_uri.scheme()
            || read_uri.authority() != write_uri.authority()
            || read_uri.path() != write_uri.path()
        {
            return Err(NodeError::Invalid(vec![FieldError {
                field: "storeLease.url".into(),
                problem: "read and write must address the same lease object".into(),
            }]));
        }
        let lock = self.lock_for(&id);
        let _guard = lock.lock().await;
        let record = self.record(&id)?;
        check_epoch(&record, epoch, EpochRule::Exact)?;
        let epoch = record.epoch.ok_or_else(|| NodeError::Internal("Store leases require placement epochs".into()))?;
        let parent = crate::store_lease::guard::docker_parent(id.as_str(), epoch);
        if self
            .inspect(&record.container_name)
            .await?
            .is_none_or(|i| i.cgroup_parent.as_deref() != Some(parent.as_str()))
        {
            return Err(NodeError::Conflict {
                code: ConflictCode::StoreLeaseRequired,
                message: "PUT the spec to create a lease-contained container first".into(),
            });
        }
        let previous = self.store_leases.lock().unwrap().get(&id).cloned();
        if let Some(previous) = previous.filter(|h| h.epoch == epoch) {
            if !previous.live() {
                return Err(NodeError::Conflict {
                    code: ConflictCode::StoreLeaseRequired,
                    message: "This lease expired; recovery needs a new epoch".into(),
                });
            }
            if previous.grant.read().unwrap().generation != grant.generation {
                return Err(NodeError::Conflict {
                    code: ConflictCode::StoreLeaseRequired,
                    message: "Lease generation changed".into(),
                });
            }
            *previous.grant.write().unwrap() = grant;
            return Ok(());
        }
        let held =
            Held::start(id.as_str(), epoch, self.config.node_id.as_str(), grant).await.map_err(NodeError::Transfer)?;
        self.store_leases.lock().unwrap().insert(id, held);
        Ok(())
    }
    /// Enabling on a host with existing containers is fail-closed. A fresh process owns no grant.
    pub async fn stop_unleased(self: &Arc<Self>) -> Result<(), NodeError> {
        if !self.config.store_lease.enabled {
            return Ok(());
        }
        for view in self.list(None) {
            if matches!(view.state, WorkloadState::Running | WorkloadState::Suspended | WorkloadState::Restarting) {
                self.kill(view.id, view.epoch).await?;
            }
        }
        Ok(())
    }
    pub(super) fn store_lease_issue(&self, record: &WorkloadRecord) -> Option<Issue> {
        if !self.config.store_lease.enabled {
            return None;
        }
        self.store_leases
            .lock()
            .unwrap()
            .get(&record.id)
            .filter(|h| h.epoch == record.epoch.unwrap_or_default() && !h.live())
            .map(|_| {
                Issue::new(
                    IssueCode::StoreLeaseExpired,
                    "Object-store lease could not renew; independent watchdog fencing requested",
                )
            })
    }
}
