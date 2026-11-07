// SPDX-License-Identifier: FSL-1.1-ALv2
//! Kernel support probing creates an unused ruleset and never restricts the caller. Actual
//! enforcement is checked again when the supervisor installs its policy and fails closed.
use landlock::{ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, Ruleset, RulesetAttr};
use serde::Serialize;
pub const FILESYSTEM_UNAVAILABLE: &str = "Process tier requires fully enforceable Landlock ABI 3";
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub filesystem: bool,
    pub network: bool,
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
    }
}
