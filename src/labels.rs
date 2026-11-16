// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The labels pryxd puts on every container it makes, under `protocol::RESERVED_LABEL_PREFIX`:
//! whose the container is, and the record it carries.

pub(crate) const LABEL_MANAGED: &str = "blocklyd.managed";
pub const LABEL_DEPLOYMENT: &str = "blocklyd.deployment";
pub(crate) const LABEL_NODE: &str = "blocklyd.node";
pub(crate) const LABEL_WORKLOAD: &str = "blocklyd.workload";
pub(crate) const LABEL_GENERATION: &str = "blocklyd.generation";
pub(crate) const LABEL_DIGEST: &str = "blocklyd.spec-digest";
pub const LABEL_EPOCH: &str = "blocklyd.epoch";
/// The whole record (minus secrets) as JSON: enough to rebuild pryxd's state from the
/// runtime alone if the state directory is lost.
pub(crate) const LABEL_RECORD: &str = "blocklyd.record";

/// Private ingress bindings for gateway-free internal networks; never user-provided.
pub(crate) const LABEL_INTERNAL_BINDINGS: &str = "blocklyd.internal-bindings";
/// Inner browser sandbox policy supplied through a supported agent's trusted bootstrap.
pub(crate) const LABEL_BROWSER_SANDBOX: &str = "blocklyd.browser-sandbox";
/// Observed, never sent: whether a process sandbox's kernel network policy is attached and verified.
pub(crate) const LABEL_NETWORK_POLICY: &str = "pryx.network-policy";
/// A process sandbox started in PID, mount and IPC namespaces of its own (`true` or `false`).
pub(crate) const LABEL_PRIVATE_PIDS: &str = "blocklyd.private-pids";
/// A process sandbox started in an empty network namespace of its own (`true` or `false`).
pub(crate) const LABEL_PRIVATE_NETWORK: &str = "blocklyd.private-network";
/// The engine runtime a container was made with, on tiers that name one (`runsc` for gvisor).
pub(crate) const LABEL_OCI_RUNTIME: &str = "blocklyd.oci-runtime";

/// Private exec cleanup attestation negotiated from the actual injected agent at creation.
pub(crate) const LABEL_EXEC_COMPLETION: &str = "blocklyd.exec-completion";

/// Protected fixed-target private ingress negotiated from the injected agent at creation.
pub(crate) const LABEL_INTERNAL_INGRESS: &str = "blocklyd.internal-ingress";

use crate::protocol::SpecRecord;
use crate::store::{AllocatedPort, RECORD_VERSION, WorkloadRecord};

/// The record as a label carries it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LabelRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<crate::protocol::Isolation>,
    pub version: u32,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    pub spec_digest: String,
    pub ports: Vec<AllocatedPort>,
    pub spec: SpecRecord,
    pub created_at: String,
}

impl LabelRecord {
    pub(super) fn of(record: &WorkloadRecord) -> Self {
        Self {
            isolation: record.isolation,
            version: RECORD_VERSION,
            generation: record.generation,
            epoch: record.epoch,
            spec_digest: record.spec_digest.clone(),
            ports: record.ports.clone(),
            spec: record.spec.clone(),
            created_at: record.created_at.clone(),
        }
    }
}

#[cfg(test)]
pub(crate) use record_fixture::private_record;

#[cfg(test)]
mod record_fixture {
    use super::*;

    pub(crate) fn private_record(image: &str, bindings: &[crate::runtime::PortBinding]) -> String {
        let ports: Vec<_> = bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| {
                serde_json::json!({
                    "name": format!("p{index}"), "containerPort": binding.container_port,
                    "protocol": binding.protocol, "audience": [crate::protocol::Audience::Control]
                })
            })
            .collect();
        let spec: crate::protocol::WorkloadSpec = serde_json::from_value(serde_json::json!({
            "image": image, "network": crate::protocol::NetworkMode::None,
            "resources": {"memoryMb": 256}, "storage": {"mountPath": "/data", "sizeGb": 1}, "ports": ports
        }))
        .unwrap();
        let spec = spec.record();
        let record = LabelRecord {
            isolation: Some(crate::protocol::Isolation::Container),
            version: RECORD_VERSION,
            generation: 1,
            epoch: None,
            spec_digest: spec.digest(),
            spec,
            ports: bindings
                .iter()
                .enumerate()
                .map(|(index, binding)| AllocatedPort {
                    name: format!("p{index}"),
                    protocol: binding.protocol,
                    container_port: binding.container_port,
                    host_port: binding.host_port,
                })
                .collect(),
            created_at: time::OffsetDateTime::UNIX_EPOCH
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap(),
        };
        serde_json::to_string(&record).unwrap()
    }
}
