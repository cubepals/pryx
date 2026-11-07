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
