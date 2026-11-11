// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-bound native artifact/cache inputs. These types depend on no supervisor, guest service
//! or acquisition implementation, so policy construction and enforcement have one shape.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const VERSION: u16 = 1;

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
}
