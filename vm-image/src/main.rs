// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! One private-stdin OCI preparation per invocation. No daemon is required; the parent applies
//! allowed-image admission and verifies this helper before execution. Output is bounded JSON.
use pryx_vm_image::{filesystems::FilesystemTools, image::Images};
use pryxd::runtime::{
    RuntimeError,
    vm::image::{Failure, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, Prepared, Request, Response},
};
use std::{
    io::{Read, Write},
    time::Duration,
};

async fn prepare() -> Result<Prepared, RuntimeError> {
    let mut bytes = Vec::new();
    std::io::stdin().take(MAX_REQUEST_BYTES + 1).read_to_end(&mut bytes).map_err(pryx_vm_image::error)?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err(pryx_vm_image::error("VM image request exceeds its admission bound"));
    }
    let request: Request = serde_json::from_slice(&bytes).map_err(pryx_vm_image::error)?;
    if request.timeout_seconds == 0 || !request.cache.is_absolute() {
        return Err(pryx_vm_image::error("VM image preparation needs an absolute private cache and a deadline"));
    }
    let tools = FilesystemTools::verified(&request.component, &request.manifest_sha256)?;
    let images = Images::new(request.cache, tools)?;
    let prepared = images.prepare(&request.image, Duration::from_secs(request.timeout_seconds)).await?;
    let configuration =
        serde_json::from_value(serde_json::to_value(prepared.configuration).map_err(pryx_vm_image::error)?)
            .map_err(pryx_vm_image::error)?;
    Ok(Prepared { digest: prepared.digest, layers: prepared.layers, configuration })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rustls::crypto::ring::default_provider().install_default().map_err(|_| "TLS provider was already installed")?;
    let response: Response = prepare().await.map_err(|problem| match problem {
        RuntimeError::ImageCacheInvalid => Failure::CacheInvalid,
        RuntimeError::Timeout(_) => Failure::Timeout,
        other => Failure::Refused(other.to_string()),
    });
    let bytes = serde_json::to_vec(&response)?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err("VM image response exceeds its bound".into());
    }
    std::io::stdout().lock().write_all(&bytes)?;
    Ok(())
}
