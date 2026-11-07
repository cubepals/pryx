// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-owned paths for the pinned agentOS distribution. Workloads can select registry package
//! names, never a sidecar executable, a host mount, a registry URL or an execution backend.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WasmConfig {
    pub distribution: PathBuf,
    pub launcher_executable: Option<PathBuf>,
    pub max_ingress_requests: usize,
    pub worker_uid: u32,
    pub worker_gid: u32,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            distribution: "/usr/local/lib/pryx/agentos".into(),
            launcher_executable: None,
            max_ingress_requests: 8,
            worker_uid: 60000,
            worker_gid: 60000,
        }
    }
}
