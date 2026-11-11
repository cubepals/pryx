// SPDX-License-Identifier: FSL-1.1-ALv2
//! The node queries the actual agent executable before adding optional bootstrap arguments.
//! These describe parser support, never host isolation or native-tool readiness.
use serde::{Deserialize, Serialize};

pub const PROBE: &str = "bootstrap-probe";
pub const BROWSER_ARGUMENT: &str = "browser-sandbox";

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default)]
    pub browser_policy: bool,
}

pub fn probe() -> Capabilities {
    Capabilities { browser_policy: true }
}
