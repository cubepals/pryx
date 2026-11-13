// SPDX-License-Identifier: FSL-1.1-ALv2
//! An owned SDK forwarding process binds private host addresses to fixed guest service
//! ports. Readiness is bounded and checked against the requested bindings; helper death
//! never counts as working ingress. The helper owns HTTP/2 framing and opaque stream flow.
use super::error;
use crate::runtime::RuntimeError;
use serde::Serialize;
use std::{
    net::{IpAddr, SocketAddr},
    path::Path,
    process::{ExitStatus, Stdio},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::Instant,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    host: IpAddr,
    port: u16,
    guest_port: u32,
}
impl Binding {
    pub fn service(host: IpAddr, host_port: u16, guest_port: u16) -> Result<Self, RuntimeError> {
        let private = match host {
            IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
            IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
        };
        if !private || guest_port == 0 {
            return Err(error("VM ingress requires explicit private host addresses and nonzero service ports"));
        }
        Ok(Self { host, port: host_port, guest_port: pryx_agent::vm_boot::stream_port(guest_port) })
    }
}

pub struct Forwarder {
    child: tokio::process::Child,
    addresses: Vec<SocketAddr>,
}
impl Forwarder {
    pub async fn start(
        binary: &Path,
        socket: &Path,
        bindings: &[Binding],
        concurrency: u16,
        deadline: Instant,
    ) -> Result<Self, RuntimeError> {
        if bindings.is_empty() || concurrency == 0 {
            return Err(error("VM forwarding requires admitted bindings and positive concurrency"));
        }
        let message = serde_json::to_vec(&serde_json::json!({"socket": socket,
            "forward": {"bindings": bindings, "concurrency": concurrency, "parent": std::process::id()}}))
        .map_err(error)?;
        if message.len() > pryx_agent::contract::MAX_REQUEST_BYTES {
            return Err(error("VM forwarding configuration exceeds its bound"));
        }
        let mut child = tokio::process::Command::new(binary)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(error)?;
        let mut input = child.stdin.take().ok_or_else(|| error("Missing VM forwarding input pipe"))?;
        let output = child.stdout.take().ok_or_else(|| error("Missing VM forwarding readiness pipe"))?;
        let addresses = tokio::time::timeout_at(deadline, async {
            input.write_all(&message).await.map_err(error)?;
            input.shutdown().await.map_err(error)?;
            drop(input);
            let mut bytes = Vec::new();
            output
                .take(pryx_agent::contract::MAX_REQUEST_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(error)?;
            if bytes.len() > pryx_agent::contract::MAX_REQUEST_BYTES {
                return Err(error("VM forwarding readiness exceeds its bound"));
            }
            let addresses: Vec<SocketAddr> = serde_json::from_slice(&bytes).map_err(error)?;
            if addresses.len() != bindings.len()
                || addresses.iter().zip(bindings).any(|(address, binding)| {
                    address.ip() != binding.host
                        || address.port() == 0
                        || (binding.port != 0 && binding.port != address.port())
                })
            {
                return Err(error("VM forwarding readiness differs from admitted host bindings"));
            }
            Ok(addresses)
        })
        .await
        .map_err(|_| RuntimeError::Timeout("VM forwarding readiness exceeded its deadline".into()))??;
        if child.try_wait().map_err(error)?.is_some() {
            return Err(error("VM forwarder exited during readiness"));
        }
        Ok(Self { child, addresses })
    }
    pub fn addresses(&self) -> &[SocketAddr] {
        &self.addresses
    }
    pub fn exited(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
    pub async fn stop(&mut self) -> std::io::Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill().await?;
        }
        Ok(())
    }
}
