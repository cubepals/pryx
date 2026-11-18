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
pub const LABEL_INTERNAL_BINDINGS: &str = "blocklyd.internal-bindings";
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
