// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private, in-memory bootstrap from the host to its pinned guest init. It contains secrets
//! and must never enter a journal, label, command line, log or data archive. Fleet protocol
//! shapes and public VM capability announcements remain outside this contract.
use serde::{Deserialize, Serialize};

pub const VERSION: u16 = 1;
pub const PORT: u32 = 11827;
pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_LAYERS: usize = 128;
pub const INIT_MODE: &str = "vm-init";
pub const NETWORK_PROGRAM: &str = "guest-network";
pub const AGENT_MODE: &str = "vm-agent";
pub const BOOTSTRAP_ROOT: &str = "/.pryx-bootstrap";
pub const MARKER: &str = "/pryx-vm-bootstrap";

pub fn marker() -> Vec<u8> {
    format!("pryx-vm-bootstrap-{VERSION}\n").into_bytes()
}

/// Full backing-file identities match the monitor's kernel-visible disk serial prefixes,
/// never host-selected guest paths. A truncated or ambiguous match is refused.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Boot {
    pub version: u16,
    pub nonce: String,
    pub cid: u32,
    pub layers: Vec<String>,
    pub writes: String,
    pub hostname: String,
    pub uid: u32,
    pub gid: u32,
    pub data: String,
    pub read_only_root: bool,
    pub temporary_bytes: u64,
    pub pids_limit: u64,
    /// Only the host constructs these arguments for the fixed authenticated agent.
    pub arguments: Vec<String>,
    pub environment: Vec<(String, String)>,
    #[serde(default)]
    pub network_arguments: Option<Vec<String>>,
}

pub fn guest_cid(cid: u32) -> bool {
    !matches!(
        cid,
        tokio_vsock::VMADDR_CID_HOST
            | tokio_vsock::VMADDR_CID_ANY
            | tokio_vsock::VMADDR_CID_HYPERVISOR
            | tokio_vsock::VMADDR_CID_LOCAL
    )
}
