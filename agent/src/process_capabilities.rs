// SPDX-License-Identifier: FSL-1.1-ALv2
//! Kernel support queries never restrict the caller. Disposable workers additionally prove
//! abstract Unix socket scoping; an available API alone cannot establish private tool IPC.
//!
//! Parts (`process_capabilities/`):
//! - `ipc.rs`: same-domain positive and independent-domain negative socket probes.
mod ipc;
pub use ipc::{IpcProbe, probe as private_ipc_probe};
use landlock::{ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, Ruleset, RulesetAttr};
use serde::Serialize;
pub const FILESYSTEM_UNAVAILABLE: &str = "Process tier requires fully enforceable Landlock ABI 3";
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub filesystem: bool,
    pub network: bool,
    #[serde(skip_serializing_if = "disabled")]
    pub abstract_unix_ipc: bool,
}
fn disabled(value: &bool) -> bool {
    !value
}
pub fn probe() -> Capabilities {
    Capabilities {
        filesystem: Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .handle_access(AccessFs::from_all(ABI::V3))
            .and_then(|rules| rules.create())
            .is_ok(),
        network: Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .handle_access(AccessNet::ConnectTcp)
            .and_then(|rules| rules.create())
            .is_ok(),
        abstract_unix_ipc: private_ipc_probe().supported(),
    }
}
