// SPDX-License-Identifier: FSL-1.1-ALv2
//! Bounded private-pipe requests through the separately pinned SDK transport. Credentials
//! never enter argv, environment, logs or journals. This adapter submits a mutation once;
//! callers may retry only safe reads. Large data and live streams use the forwarding process.
use super::error;
use crate::runtime::RuntimeError;
use base64::{Engine, engine::general_purpose::STANDARD};
use hyper::StatusCode;
use serde::Deserialize;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Deserialize)]
struct Response {
    status: u16,
    body: String,
}

pub async fn request(
    binary: &Path,
    message: &serde_json::Value,
    timeout: Duration,
    response_limit: usize,
) -> Result<(StatusCode, Vec<u8>), RuntimeError> {
    let input = serde_json::to_vec(message).map_err(error)?;
    if input.len() > pryx_agent::contract::MAX_REQUEST_BYTES || response_limit == 0 || timeout.is_zero() {
        return Err(error("VM control request exceeds its host admission bound"));
    }
    // Base64 plus its JSON envelope is bounded before deserialization or decoding.
    let limit = response_limit
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(pryx_agent::contract::MAX_REQUEST_BYTES))
        .ok_or_else(|| error("Invalid VM control response bound"))?;
    tokio::time::timeout(timeout, async {
        let mut child = tokio::process::Command::new(binary)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(error)?;
        let mut pipe = child.stdin.take().ok_or_else(|| error("Missing VM transport input pipe"))?;
        pipe.write_all(&input).await.map_err(error)?;
        pipe.shutdown().await.map_err(error)?;
        drop(pipe);
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| error("Missing VM transport output pipe"))?
            .take(limit as u64 + 1)
            .read_to_end(&mut output)
            .await
            .map_err(error)?;
        if output.len() > limit {
            return Err(error("VM control response exceeds its host admission bound"));
        }
        if !child.wait().await.map_err(error)?.success() {
            return Err(error("VM SDK transport exited without a complete response"));
        }
        let response: Response = serde_json::from_slice(&output).map_err(error)?;
        let body = STANDARD.decode(response.body).map_err(error)?;
        if body.len() > response_limit {
            return Err(error("Decoded VM control response exceeds its bound"));
        }
        Ok((StatusCode::from_u16(response.status).map_err(error)?, body))
    })
    .await
    .map_err(|_| RuntimeError::Timeout("VM SDK transport exceeded its deadline".into()))?
}
