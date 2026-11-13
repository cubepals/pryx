// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private bounded pipe contract for the separately built OCI helper. The registry's image
//! configuration remains the OCI owner's type; neither credentials nor image paths are logged.
use super::error;
use crate::runtime::RuntimeError;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const MAX_REQUEST_BYTES: u64 = 16 * 1024;
pub const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub component: PathBuf,
    pub manifest_sha256: String,
    pub cache: PathBuf,
    pub image: String,
    pub timeout_seconds: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Prepared {
    pub digest: String,
    pub layers: Vec<PathBuf>,
    pub configuration: oci_spec::image::ImageConfiguration,
}

#[derive(Deserialize, Serialize)]
pub enum Failure {
    CacheInvalid,
    Timeout,
    Refused(String),
}

pub type Response = Result<Prepared, Failure>;

/// Executable and request paths are supplied only by verified host configuration, after
/// allowed-image admission. Killing a timed-out helper never retries a mutation or repairs cache.
pub async fn prepare(binary: &std::path::Path, request: &Request) -> Result<Prepared, RuntimeError> {
    let timeout = Duration::from_secs(request.timeout_seconds);
    let bytes = serde_json::to_vec(request).map_err(error)?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES || timeout.is_zero() {
        return Err(error("VM image request exceeds its admission bound"));
    }
    tokio::time::timeout(timeout, async {
        let mut child = tokio::process::Command::new(binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(error)?;
        let mut input = child.stdin.take().ok_or_else(|| error("Missing VM image input pipe"))?;
        input.write_all(&bytes).await.map_err(error)?;
        input.shutdown().await.map_err(error)?;
        drop(input);
        let mut response = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| error("Missing VM image output pipe"))?
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut response)
            .await
            .map_err(error)?;
        if response.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(error("VM image response exceeds its bound"));
        }
        if !child.wait().await.map_err(error)?.success() {
            return Err(error("VM image helper exited without a complete response"));
        }
        match serde_json::from_slice::<Response>(&response).map_err(error)? {
            Ok(prepared) => Ok(prepared),
            Err(Failure::CacheInvalid) => Err(RuntimeError::ImageCacheInvalid),
            Err(Failure::Timeout) => Err(RuntimeError::Timeout("VM image helper exceeded its deadline".into())),
            Err(Failure::Refused(problem)) => Err(error(problem)),
        }
    })
    .await
    .map_err(|_| RuntimeError::Timeout("VM image helper exceeded its deadline".into()))?
}
