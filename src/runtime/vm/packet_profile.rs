// SPDX-License-Identifier: FSL-1.1-ALv2
//! Read the verified packet executable's own metadata and assemble its trusted guest profile.
//! This neither starts a packet helper nor supplies network values owned by another program.
use super::error;
use crate::runtime::RuntimeError;
use serde_json::Value;
use std::{path::Path, process::Stdio};
use tokio::{io::AsyncReadExt, process::Command, time::Instant};

pub(super) async fn read(mut output: impl tokio::io::AsyncRead + Unpin, limit: usize) -> Result<Value, RuntimeError> {
    let mut bytes = Vec::new();
    (&mut output).take(limit as u64 + 1).read_to_end(&mut bytes).await.map_err(error)?;
    if bytes.len() > limit {
        return Err(error("VM packet helper output exceeds its bound"));
    }
    serde_json::from_slice(&bytes).map_err(error)
}

pub(super) async fn load(program: &Path, deadline: Instant) -> Result<Value, RuntimeError> {
    // Query the verified executable, never a workload-selected network description.
    let mut metadata = Command::new(program)
        .arg("--metadata")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(error)?;
    tokio::time::timeout_at(deadline, async {
        let output = metadata.stdout.take().ok_or_else(|| error("Missing packet helper metadata pipe"))?;
        let metadata_value = read(output, pryx_agent::contract::MAX_REQUEST_BYTES).await?;
        if !metadata.wait().await.map_err(error)?.success() || !metadata_value["production"].is_object() {
            return Err(error("VM packet helper did not export its production profile"));
        }
        Ok(metadata_value["production"].clone())
    })
    .await
    .map_err(|_| RuntimeError::Timeout("VM packet helper metadata exceeded its deadline".into()))?
}

pub(super) fn guest(
    profile: &Value,
    ports: &crate::runtime::egress::listeners::Ports,
    unrestricted: bool,
) -> Result<pryx_agent::vm_boot::Network, RuntimeError> {
    use pryx_agent::vm_boot::{self, NetworkPolicy, Redirect};
    let text = |name: &str| profile[name].as_str().ok_or_else(|| error("VM packet helper omitted its network profile"));
    let port = |name: &str| {
        profile[name]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port != 0)
            .ok_or_else(|| error("VM packet helper omitted its service port"))
    };
    Ok(vm_boot::Network {
        static_ipv4: None,
        interface: text("interface")?.into(),
        mac: text("guestMac")?.into(),
        mtu: profile["mtu"]
            .as_u64()
            .and_then(|mtu| u32::try_from(mtu).ok())
            .ok_or_else(|| error("VM packet helper omitted its MTU"))?,
        vsock_port: Some(vm_boot::PACKET_PORT),
        policy: Some(NetworkPolicy {
            unrestricted,
            host: text("host")?.parse().map_err(error)?,
            http: Redirect { source: port("http")?, target: ports.http },
            tls: Redirect { source: port("tls")?, target: ports.tls },
            dns: Redirect { source: port("dns")?, target: ports.dns },
            dhcp_client: port("dhcpClient")?,
            dhcp_server: port("dhcpServer")?,
        }),
    })
}
