// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Small non-secret checkpoints in persistent data. The node reads these with the same safe
//! descriptors as files; signing material, commands and environments never enter them.

use crate::paths::DataDir;
use serde::{Deserialize, Serialize};
use std::io::{self, Read};

pub const PORTS_PATH: &str = ".pryx-agent/ports.json";
pub const ACTIVITY_PATH: &str = ".pryx-agent/activity.json";
const MAX_CHECKPOINT_BYTES: u64 = 8192;
// A revision is carried through JSON clients whose numeric mantissa is IEEE 754 binary64.
pub const MAX_REVISION: u64 = (1_u64 << f64::MANTISSA_DIGITS) - 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PortState {
    pub revision: u64,
    pub targets: Vec<Option<u16>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Activity {
    pub at: i64,
    pub streams: u32,
}

/// Current non-secret routing and activity metadata, transported without mounting guest data.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub ports: PortState,
    pub activity: Activity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vm_resources: Option<VmResources>,
}

/// Guest kernel counters are observations, never evidence that host policy was enforced.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VmResources {
    pub cpu_total_ns: Option<u64>,
    pub memory_usage: Option<u64>,
    pub memory_inactive_file: Option<u64>,
    pub pids: Option<u64>,
    pub rx_bytes: Option<u64>,
    pub tx_bytes: Option<u64>,
}

pub fn read<T: serde::de::DeserializeOwned + Default>(directory: &DataDir, path: &str) -> io::Result<T> {
    let file = match directory.read(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(T::default()),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(MAX_CHECKPOINT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    serde_json::from_slice(&bytes).map_err(|_| io::ErrorKind::InvalidData.into())
}

pub fn write<T: Serialize>(directory: &DataDir, path: &str, value: &T, durable: bool) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    directory.write_atomic(path, &bytes, durable)
}
