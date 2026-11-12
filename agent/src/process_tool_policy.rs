// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-bound native artifact/cache inputs. These types depend on no supervisor, guest service
//! or acquisition implementation, so policy construction and enforcement have one shape.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const VERSION: u16 = 1;
pub const BROWSER_VERSION: u16 = 1;
pub const BROWSER_CAPABILITY: &str = "process-browser-sandbox";
pub const DESKTOP_VERSION: u16 = 1;
pub const DESKTOP_CAPABILITY: &str = "process-desktop";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub url: String,
    pub sha256: String,
    pub manifest_sha256: String,
    pub bytes: u64,
    pub unpacked_bytes: u64,
    pub entries: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub cache: PathBuf,
    pub downloads: PathBuf,
    pub scratch: PathBuf,
    pub artifact: Option<Artifact>,
    pub unavailable: Option<String>,
    pub browser_on: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_on: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_unavailable: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_policy_round_trip_omits_unannounced_desktop_fields() {
        let legacy = serde_json::json!({"cache":"/cache","downloads":"/downloads","scratch":"/scratch","artifact":null,"unavailable":null,"browser_on":false});
        let decoded: Policy = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(decoded.desktop_on, None);
        assert_eq!(decoded.desktop_unavailable, None);
        assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    }
}
