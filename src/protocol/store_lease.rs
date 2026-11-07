// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Optional store-lease grant and object format. Credentials are in ephemeral presigned URLs, never records or labels.
use serde::{Deserialize, Serialize};

pub(crate) const MIN_STORE_LEASE_SECONDS: u64 = 10;
pub(crate) const STORE_LEASE_MARGIN_SECONDS: u64 = 2;

#[derive(Clone, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoreLeaseGrant {
    pub(crate) session: String,
    pub(crate) read_url: String,
    pub(crate) write_url: String,
    pub(crate) generation: String,
    pub(crate) seconds: u64,
    pub(crate) renew_seconds: u64,
}
impl std::fmt::Debug for StoreLeaseGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreLeaseGrant")
            .field("seconds", &self.seconds)
            .field("renew_seconds", &self.renew_seconds)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoreLeaseObject {
    pub(crate) session: String,
    pub(crate) version: u32,
    pub(crate) workload: String,
    pub(crate) node: String,
    pub(crate) epoch: u64,
    pub(crate) generation: String,
    pub(crate) expires_at_ms: u64,
}

/// A new random identity for this daemon process, never loaded from local disk or container metadata.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoreLeaseIdentity {
    pub(crate) session: String,
}
