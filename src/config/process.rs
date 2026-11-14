// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-owned process tier settings. A workload cannot change the UID pool, syscall policy,
//! virtual-memory ceiling or the selected runtime.
use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeKind {
    #[default]
    Container,
    Auto,
    Process,
    Wasm,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProcessConfig {
    pub uid_first: u32,
    pub uid_count: u32,
    pub address_space_mb: u64,
    pub cpu_seconds: u64,
    pub open_files: u64,
}
impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            uid_first: 20000,
            uid_count: 16384,
            address_space_mb: pryx_agent::process_policy::DEFAULT_ADDRESS_SPACE_MB,
            cpu_seconds: 600,
            open_files: 1024,
        }
    }
}
