// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! One private-stdin OCI preparation, or eviction, per invocation. No daemon is required; the
//! parent applies allowed-image admission and verifies this helper before execution. Output is
//! bounded JSON.
use pryx_vm_image::{filesystems::FilesystemTools, image::Images};
use pryxd::runtime::{
    RuntimeError,
    vm::image::{Failure, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, Prepared, Request, Response},
};
use std::{
    io::{Read, Write},
    time::Duration,
};

/// A refusal as the pipe carries it.
fn failure(problem: RuntimeError) -> Failure {
    match problem {
        RuntimeError::ImageCacheInvalid => Failure::CacheInvalid,
        RuntimeError::Timeout(_) => Failure::Timeout,
        other => Failure::Refused(other.to_string()),
    }
}

fn request() -> Result<Request, RuntimeError> {
    let mut bytes = Vec::new();
    std::io::stdin().take(MAX_REQUEST_BYTES + 1).read_to_end(&mut bytes).map_err(pryx_vm_image::error)?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err(pryx_vm_image::error("VM image request exceeds its admission bound"));
    }
    let request: Request = serde_json::from_slice(&bytes).map_err(pryx_vm_image::error)?;
    if request.timeout_seconds == 0 || !request.cache.is_absolute() {
        return Err(pryx_vm_image::error("VM image preparation needs an absolute private cache and a deadline"));
    }
    Ok(request)
}

async fn prepare() -> Response {
    let request = request().map_err(failure)?;
    let tools =
        FilesystemTools::verified(&request.component, &request.manifest_sha256, &request.launcher).map_err(failure)?;
    let images = Images::new(request.cache, tools).map_err(failure)?.with_room(request.free_floor_bytes, request.keep);
    let timeout = Duration::from_secs(request.timeout_seconds);
    if request.evict {
        images.evict(timeout).await.map_err(failure)?;
        // An eviction prepares nothing, so it answers with nothing prepared.
        return Ok(Prepared { digest: String::new(), layers: Vec::new(), configuration: Default::default() });
    }
    let prepared =
        images.prepare_pinned(&request.image, request.pinned.as_deref(), timeout).await.map_err(|problem| {
            if images.short_of_room() { Failure::Room(problem.to_string()) } else { failure(problem) }
        })?;
    let configuration = serde_json::to_value(prepared.configuration)
        .and_then(serde_json::from_value)
        .map_err(|problem| failure(pryx_vm_image::error(problem)))?;
    Ok(Prepared { digest: prepared.digest, layers: prepared.layers, configuration })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rustls::crypto::ring::default_provider().install_default().map_err(|_| "TLS provider was already installed")?;
    let response: Response = prepare().await;
    let bytes = serde_json::to_vec(&response)?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err("VM image response exceeds its bound".into());
    }
    std::io::stdout().lock().write_all(&bytes)?;
    Ok(())
}
