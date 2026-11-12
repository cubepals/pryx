// SPDX-License-Identifier: FSL-1.1-ALv2
//! A native desktop session owns only its closed worker and scoped Unix connection. Tool
//! commands keep their original API validation and output bounds; public access expires.
use super::{browser_session::failed, desktop};
use crate::context::Failure;
use futures_util::StreamExt;
use std::sync::Mutex;

pub struct Desktop {
    pub(super) launch: desktop::Launch,
    child: Mutex<std::process::Child>,
}
impl Desktop {
    pub(super) fn new(launch: desktop::Launch, child: std::process::Child) -> Self {
        Self { launch, child: Mutex::new(child) }
    }
    pub(super) fn exited(&self) -> std::io::Result<bool> {
        Ok(self.child.lock().unwrap().try_wait()?.is_some())
    }
    pub async fn connect(&self) -> std::io::Result<tokio::net::UnixStream> {
        tokio::net::UnixStream::connect(format!("\0{}", self.launch.address)).await
    }
    pub async fn command(
        &self,
        argv: Vec<String>,
        input: Option<&[u8]>,
        timeout: std::time::Duration,
    ) -> Result<Vec<u8>, Failure> {
        let (name, args) = argv.split_first().ok_or_else(|| failed("Missing desktop command"))?;
        let input = input.map(std::str::from_utf8).transpose().map_err(|_| failed("Desktop text must be UTF-8"))?;
        let protocol = desktop::protocol();
        let client = reqwest::Client::builder()
            .unix_socket(std::path::PathBuf::from(format!("\0{}", self.launch.address)))
            .timeout(timeout)
            .build()
            .map_err(|_| failed("Desktop client unavailable"))?;
        let response = client
            .post(format!("http://localhost{}", protocol.command_path))
            .header(&protocol.private_header, &self.launch.private_token)
            .body(
                serde_json::json!({"name":name,"args":args,"input":input,"timeoutMs":timeout.as_millis()}).to_string(),
            )
            .send()
            .await
            .map_err(|_| failed("Desktop command failed"))?
            .error_for_status()
            .map_err(|_| failed("Desktop command failed"))?;
        let mut bytes = Vec::new();
        let mut chunks = response.bytes_stream();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|_| failed("Desktop command output was incomplete"))?;
            if bytes.len().saturating_add(chunk.len()) > crate::contract::CAPTURE_BYTES {
                return Err(failed("Desktop command output exceeded its budget"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        let child = self.child.get_mut().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}
