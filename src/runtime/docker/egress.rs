// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-only hostname egress uses a dedicated bridge per workload. The root-installed firewall
//! redirects HTTP/TLS to bounded gateways and drops every other new bridge connection. The
//! sandbox cannot opt out with proxy variables, DNS answers, raw IPs or another bridge's gateway.
//!
//! Parts (`egress/`):
//! - `gateway.rs`: HTTP/TLS forwarding and public DNS pinning.
mod gateway;
use super::*;
use crate::protocol::EgressPolicy;
use std::{
    io::Read,
    net::IpAddr,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    sync::Arc,
};
use tokio::{
    net::TcpListener,
    sync::{Mutex, Semaphore},
};
use tokio_util::sync::{CancellationToken, DropGuard};
const PREFIX: &str = "pye";
const TABLE: &str = "pryx_egress";
const READY: &str = "/run/pryx-egress-v1";
const VERSION: &[u8] = b"pryx-egress-v1\n";
const HTTP_PORT: u16 = 19080;
const TLS_PORT: u16 = 19443;
const MAX_CONNECTIONS: usize = 64;
const POLICY_LABEL: &str = "blocklyd.egress-policy";

pub(crate) fn ruleset() -> String {
    format!(
        "table inet {TABLE}\ndelete table inet {TABLE}\ntable inet {TABLE} {{\n chain redirect {{ type nat hook prerouting priority dstnat - 10; policy accept; iifname \"{PREFIX}*\" tcp dport 80 redirect to :{HTTP_PORT}; iifname \"{PREFIX}*\" tcp dport 443 redirect to :{TLS_PORT}; }}\n chain input {{ type filter hook input priority filter - 20; policy accept; iifname \"{PREFIX}*\" ct state established,related accept; iifname \"{PREFIX}*\" tcp dport {{ {HTTP_PORT}, {TLS_PORT} }} accept; iifname \"{PREFIX}*\" drop; }}\n chain forward {{ type filter hook forward priority filter - 20; policy accept; iifname \"{PREFIX}*\" ct state new drop; }}\n}}\n"
    )
}
pub(crate) fn mark_ready() -> std::io::Result<()> {
    let temporary = format!("{READY}.{}", std::process::id());
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true).mode(0o444).custom_flags(libc::O_NOFOLLOW);
    use std::io::Write;
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(VERSION)?;
        file.sync_all()?;
        std::fs::rename(&temporary, READY)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
pub(crate) fn ready() -> bool {
    let result = (|| -> std::io::Result<bool> {
        let mut file =
            std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(READY)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.uid() != 0 || metadata.permissions().mode() & 0o022 != 0 {
            return Ok(false);
        }
        let mut bytes = Vec::new();
        file.by_ref().take(VERSION.len() as u64 + 1).read_to_end(&mut bytes)?;
        Ok(bytes == VERSION)
    })();
    result.unwrap_or(false)
}
fn name(container: &str) -> String {
    let digest = hex::encode(Sha256::digest(container.as_bytes()));
    format!("{PREFIX}{}", &digest[..12])
}
fn error(message: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Other(format!("Hostname egress: {message}"))
}
#[derive(Default)]
pub(super) struct Egress {
    held: Mutex<HashMap<String, (EgressPolicy, DropGuard)>>,
}
impl Egress {
    pub(super) async fn prepare(&self, runtime: &DockerRuntime, spec: &mut ContainerSpec) -> Result<(), RuntimeError> {
        let Some(policy) = spec.egress.clone() else { return Ok(()) };
        if !ready() {
            return Err(error("host firewall has not announced egress-hostnames"));
        }
        let network = name(&spec.name);
        let docker = runtime.docker()?;
        match quick("inspect egress network", docker.inspect_network(&network, None::<InspectNetworkOptions>)).await {
            Ok(_) => {}
            Err(RuntimeError::NotFound(_)) => {
                let request = NetworkCreateRequest {
                    name: network.clone(),
                    driver: Some("bridge".into()),
                    enable_ipv6: Some(false),
                    options: Some(HashMap::from([
                        (ICC_OPTION.into(), "false".into()),
                        (BRIDGE_NAME_OPTION.into(), network.clone()),
                    ])),
                    labels: Some(spec.labels.clone().into_iter().collect()),
                    ..Default::default()
                };
                quick("create egress network", docker.create_network(request)).await?;
            }
            Err(e) => return Err(e),
        }
        self.listen(runtime, &network, policy.clone()).await?;
        spec.network = network;
        spec.labels.insert(POLICY_LABEL.into(), serde_json::to_string(&policy).map_err(error)?);
        Ok(())
    }
    async fn listen(&self, runtime: &DockerRuntime, network: &str, policy: EgressPolicy) -> Result<(), RuntimeError> {
        let mut held = self.held.lock().await;
        if let Some((existing, _)) = held.get(network) {
            return if existing == &policy { Ok(()) } else { Err(error("bridge has another policy")) };
        }
        if !ready() {
            return Err(error("host firewall is unavailable"));
        }
        let info =
            quick("inspect egress bridge", runtime.docker()?.inspect_network(network, None::<InspectNetworkOptions>))
                .await?;
        let options = info.options.unwrap_or_default();
        if options.get(ICC_OPTION).map(String::as_str) != Some("false")
            || options.get(BRIDGE_NAME_OPTION).map(String::as_str) != Some(network)
            || info.enable_ipv6 == Some(true)
        {
            return Err(error("bridge does not enforce host policy"));
        }
        let config = info
            .ipam
            .and_then(|ipam| ipam.config)
            .and_then(|configs| {
                configs
                    .into_iter()
                    .find(|c| c.gateway.as_ref().is_some_and(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok()))
            })
            .ok_or_else(|| error("missing gateway"))?;
        let gateway: IpAddr = config.gateway.ok_or_else(|| error("missing gateway"))?.parse().map_err(error)?;
        let subnet: ipnet::IpNet = config.subnet.ok_or_else(|| error("missing subnet"))?.parse().map_err(error)?;
        let http = TcpListener::bind((gateway, HTTP_PORT)).await.map_err(error)?;
        let tls = TcpListener::bind((gateway, TLS_PORT)).await.map_err(error)?;
        let cancel = CancellationToken::new();
        let guard = cancel.clone().drop_guard();
        let capacity = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        gateway::serve(http, false, subnet, policy.clone(), capacity.clone(), cancel.clone());
        gateway::serve(tls, true, subnet, policy.clone(), capacity, cancel);
        held.insert(network.into(), (policy, guard));
        Ok(())
    }
    pub(super) async fn restore(
        &self,
        runtime: &DockerRuntime,
        info: &bollard::models::ContainerInspectResponse,
    ) -> Result<(), RuntimeError> {
        let Some(encoded) =
            info.config.as_ref().and_then(|c| c.labels.as_ref()).and_then(|labels| labels.get(POLICY_LABEL))
        else {
            return Ok(());
        };
        let name = info.name.as_deref().ok_or_else(|| error("missing name"))?.trim_start_matches('/');
        self.listen(runtime, &self::name(name), serde_json::from_str(encoded).map_err(error)?).await
    }
    pub(super) async fn stop(&self, runtime: &DockerRuntime, container: &str) -> Result<(), RuntimeError> {
        let name = name(container);
        if self.held.lock().await.remove(&name).is_none() {
            return Ok(());
        }
        match quick("remove egress network", runtime.docker()?.remove_network(&name)).await {
            Ok(()) | Err(RuntimeError::NotFound(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }
}
