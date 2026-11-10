// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Protocol v1: what the control plane sends and what pryxd answers.
//!
//! The vocabulary is workloads, not containers. A workload is a spec (image, environment,
//! resources, storage, ports, stop) pryxd keeps running or stopped on this host. Nothing
//! here names Docker, so pryxd could move to containerd or microVMs under the same
//! protocol. It mirrors the control plane's `RuntimeSpec` (apps/control/src/app/ports/runtime.ts) closely
//! on purpose: an `infra/fleet` adapter would translate field for field.
//!
//! Requests are strict (`deny_unknown_fields`): a node daemon that silently ignored a field a newer
//! control plane relies on — a limit, say — would be worse than one that refuses. Responses only
//! ever grow; clients must ignore fields they don't know. New request fields are announced in
//! `NodeHealth::features` before anyone sends them.
//!
//! Parts (`protocol/`):
//! - `spec.rs`: what a workload is, as the control plane sends it and as pryxd keeps it.
//! - `workload.rs`: what pryxd reports about one workload: its view, stats and log lines.
//! - `lifecycle.rs`: the verbs that create, start, suspend, stop, fence and delete a workload.
//! - `transfer.rs`: moving a workload's data: export, restore, snapshots and their upload.
//! - `store_lease.rs`: optional ephemeral store authority and its versioned object.
//! - `exec.rs`: running one command inside a workload.
//! - `agent.rs`: optional agent bootstrap, tokens and file metadata.
//! - `guarantees.rs`: additive measured enforcement names shared with the public contract.
//! - `node.rs`: what the node reports about itself.
//! - `wire.rs`: what a node and its control plane send each other: enrollment, heartbeats, renewal.
//! - `schema.rs`: tests only; these types as the OpenAPI documents the control plane's TypeScript
//!   types are generated from.

use serde::{Deserialize, Serialize};

mod agent;
mod exec;
mod guarantees;
mod lifecycle;
mod node;
#[cfg(test)]
mod schema;
mod spec;
mod store_lease;
mod transfer;
pub mod wire;
mod workload;

pub use agent::*;
pub use exec::*;
pub use guarantees::*;
pub use lifecycle::*;
pub use node::*;
pub use spec::*;
pub(crate) use store_lease::*;
pub use transfer::*;
pub use workload::*;

pub(crate) const PROTOCOL_VERSION: u32 = 1;
/// The existing minimum requested CPU rate, shared by all runtime policies.
pub const MIN_CPU_MILLIS: u32 = 100;
/// Default host memory floor; the operator may configure a different policy.
pub const DEFAULT_MIN_MEMORY_MB: u32 = 256;
/// Default inclusive host listener pool, also used for legacy image configurations.
pub const DEFAULT_PORT_RANGE: [u16; 2] = [42000, 42999];
/// Every protocol version this pryxd speaks, PROTOCOL_VERSION among them.
pub(crate) const SUPPORTED_VERSIONS: &[u32] = &[1];
/// Sent on every response so a client can tell which protocol answered.
pub(crate) const PROTOCOL_HEADER: &str = "blocklyd-protocol";
/// Makes an exec run once: a repeat with the same key gets the first answer.
pub(crate) const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
/// Names a request in the node's logs and in its answer; one is made when a client sends none.
pub(crate) const REQUEST_ID_HEADER: &str = "x-request-id";
/// Must echo the workload id for `DELETE …?data=delete`: data never goes on a single typo.
pub(crate) const CONFIRM_DELETE_HEADER: &str = "x-blockly-confirm-delete-data";
/// Features a client may rely on, beyond protocol v1's base. Only ever added to (`features_only_grow`).
pub(crate) const FEATURES: &[&str] = &[
    "exec-idempotency-key",
    "logs-follow-across-restarts",
    "conditional-put",
    "placement-epochs",
    "data-transfer",
    "local-snapshots",
    "export-exclude",
    "execution-lease",
    "certificate-renewal",
    "resume-after-reboot",
    "multipart-upload",
    "self-upgrade",
    "suspend",
    "signed-upgrade",
    "exec-stream",
    "files",
    "cpu-reservation",
    "workload-options",
    "workload-metrics",
    "multi-tier",
];

impl ProtocolVersions {
    /// This build's, as enrollment, heartbeats and `/v1/health` say them.
    pub fn ours() -> Self {
        Self { current: PROTOCOL_VERSION, supported: SUPPORTED_VERSIONS.to_vec() }
    }
}

/// FEATURES, in order, as enrollment, heartbeats and `/v1/health` say them.
pub fn features() -> Vec<String> {
    FEATURES.iter().map(|f| (*f).to_owned()).collect()
}

/// Process hosts share pryx's bundled userland rather than pulling workload images.
pub const WASM_PORT_PROBLEM: &str = "this host runs the wasm tier; application ports require HTTP over TCP";
pub const WASM_IMAGE_PROBLEM: &str = "this host runs the wasm tier; images are not available";
pub const MAX_WASM_PACKAGES: usize = 32;
pub const PACKAGE_NAME_PATTERN: &str = "^[a-z][a-z0-9-]{0,63}$";
pub const PROCESS_IMAGE_PROBLEM: &str = "this host runs the process tier; images are not available";
pub const CPU_LIMIT_PROBLEM: &str = "CPU ceilings and weights require container isolation";

/// The placement epoch a mutating request acts for: a fencing token the control plane issues,
/// bumped each time a workload is placed anew (moved, recovered, or brought back). A node refuses
/// a request older than the copy it holds, so a delayed or stale caller can't act on it.
pub(crate) const EPOCH_HEADER: &str = "blocklyd-epoch";

/// Label keys pryxd writes itself; a spec may not set anything under this prefix, so a
/// request can never forge ownership of a container.
pub(crate) const RESERVED_LABEL_PREFIX: &str = "blocklyd.";

/// One thing wrong with a request, by the JSON path of the field.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FieldError {
    pub(crate) field: String,
    pub(crate) problem: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub(crate) struct ErrorBody {
    pub(crate) error: ErrorDetail,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub(crate) struct ErrorDetail {
    pub(crate) code: String,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) details: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_build_speaks_its_own_protocol_version() {
        let ours = ProtocolVersions::ours();
        assert_eq!(ours.current, PROTOCOL_VERSION);
        assert!(ours.supported.contains(&PROTOCOL_VERSION));
        // What enrollment, heartbeats and /v1/health have always sent.
        assert_eq!(serde_json::to_value(&ours).unwrap(), serde_json::json!({ "current": 1, "supported": [1] }));
        assert_eq!(features(), FEATURES);
    }

    /// The control plane stops placing on, or upgrading, a node without a feature it needs, so a
    /// feature once shipped stays: these are all the ones shipped so far.
    #[test]
    fn features_only_grow() {
        let shipped = [
            "exec-idempotency-key",
            "logs-follow-across-restarts",
            "conditional-put",
            "placement-epochs",
            "data-transfer",
            "local-snapshots",
            "export-exclude",
            "execution-lease",
            "certificate-renewal",
            "resume-after-reboot",
            "multipart-upload",
            "self-upgrade",
        ];
        for feature in shipped {
            assert!(FEATURES.contains(&feature), "{feature} shipped: control planes rely on it, so it stays");
        }
    }
}
