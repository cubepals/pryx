// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-installed VM descriptor location. Workloads cannot provide an archive, executable,
//! download URL or monitor identity; an absent installation leaves this tier unavailable.
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VmConfig {
    pub bundle: Option<PathBuf>,
}
