// SPDX-License-Identifier: FSL-1.1-ALv2
//! A browser session owns its scoped Unix connection and bounded authenticated screenshots.
//! Acquisition and namespace launch remain in the closed worker, never in this service client.
use super::browser::Launch;
use crate::context::Failure;
use axum::http::{StatusCode, header::CONTENT_TYPE};
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::{Mutex, OnceLock};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Protocol {
    screenshot_path: String,
    private_header: String,
}
fn protocol() -> &'static Protocol {
    static PROTOCOL: OnceLock<Protocol> = OnceLock::new();
    PROTOCOL.get_or_init(|| {
        serde_json::from_str(include_str!("../../../templates/process-tools/browser/protocol.json"))
            .expect("committed private browser protocol")
    })
}
pub struct Browser {
    address: String,
    private_token: String,
    child: Mutex<std::process::Child>,
}
pub(super) fn failed(message: impl Into<String>) -> Failure {
    Failure(StatusCode::CONFLICT, "tool_failed", message.into())
}
impl Browser {
    pub(super) fn new(launch: Launch, child: std::process::Child) -> Self {
        Self { address: launch.address, private_token: launch.private_token, child: Mutex::new(child) }
    }
    pub(super) fn exited(&self) -> std::io::Result<bool> {
        Ok(self.child.lock().unwrap().try_wait()?.is_some())
    }
    pub async fn connect(&self) -> std::io::Result<tokio::net::UnixStream> {
        tokio::net::UnixStream::connect(format!("\0{}", self.address)).await
    }
    pub async fn screenshot(&self, url: &str) -> Result<Vec<u8>, Failure> {
        let client = reqwest::Client::builder()
            .unix_socket(std::path::PathBuf::from(format!("\0{}", self.address)))
            .timeout(crate::contract::TOOL_STARTUP)
            .build()
            .map_err(|_| failed("Browser client unavailable"))?;
        let response = client
            .post(format!("http://localhost{}", protocol().screenshot_path))
            .header(&protocol().private_header, &self.private_token)
            .header(CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(&serde_json::json!({"url":url})).expect("fixed screenshot request serializes"))
            .send()
            .await
            .map_err(|_| failed("Browser screenshot failed"))?
            .error_for_status()
            .map_err(|_| failed("Browser screenshot failed"))?;
        let mut bytes = Vec::new();
        let mut chunks = response.bytes_stream();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|_| failed("Browser image was incomplete"))?;
            if bytes.len().saturating_add(chunk.len()) > crate::contract::MAX_IMAGE_BYTES {
                return Err(failed("Browser image exceeded its budget"));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !bytes.starts_with(crate::contract::PNG) {
            return Err(failed("Browser did not return a PNG"));
        }
        Ok(bytes)
    }
}
impl Drop for Browser {
    fn drop(&mut self) {
        let child = self.child.get_mut().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}
