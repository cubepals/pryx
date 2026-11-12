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
    #[serde(default)]
    pub process_tools_version: u16,
    #[serde(default)]
    pub process_browser_version: u16,
    #[serde(default)]
    pub process_desktop_version: u16,
}

pub fn probe() -> Capabilities {
    Capabilities {
        browser_policy: true,
        process_tools_version: crate::process_tool_policy::VERSION,
        process_browser_version: crate::process_tool_policy::BROWSER_VERSION,
        process_desktop_version: crate::process_tool_policy::DESKTOP_VERSION,
    }
}
