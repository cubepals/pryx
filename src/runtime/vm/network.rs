// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Own the maintained packet helper and hostname gateways through actual helper retirement.
//! Only trusted host code supplies these paths and policy. This module neither installs guest
//! rules nor registers a VM tier; readiness alone does not prove packet flow or guest isolation.
use super::error;
use crate::{protocol::EgressPolicy, runtime::RuntimeError};
use pryx_agent::vm_monitor::{Plan, Role};
use serde_json::Value;
use std::{
    fs, io,
    net::Ipv4Addr,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
    time::Instant,
};
use tokio_util::sync::DropGuard;

/// These endpoints are meaningful only with the matching trusted guest redirect policy.
pub struct Network {
    child: Option<Child>,
    resources: Option<Resources>,
    profile: Value,
    ports: crate::runtime::egress::listeners::Ports,
    socket: PathBuf,
    retirement: Option<tokio::sync::watch::Receiver<Option<Result<(), String>>>>,
    runtime: tokio::runtime::Handle,
    unrestricted: bool,
}
struct Resources {
    _gateways: DropGuard,
    _directory: tempfile::TempDir,
}

async fn read(mut output: impl tokio::io::AsyncRead + Unpin, limit: usize) -> Result<Value, RuntimeError> {
    let mut bytes = Vec::new();
    (&mut output).take(limit as u64 + 1).read_to_end(&mut bytes).await.map_err(error)?;
    if bytes.len() > limit {
        return Err(error("VM packet helper output exceeds its bound"));
    }
    serde_json::from_slice(&bytes).map_err(error)
}

fn argument(profile: &Value, field: &str) -> Result<String, RuntimeError> {
    profile[field]
        .as_str()
        .filter(|argument| {
            argument.strip_prefix("--").is_some_and(|name| {
                !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-')
            })
        })
        .map(str::to_owned)
        .ok_or_else(|| error("VM packet helper omitted its private argument name"))
}

pub struct Paths<'a> {
    pub launcher: &'a Path,
    pub program: &'a Path,
    pub boots: &'a Path,
    pub lease: &'a Path,
    pub socket: Option<&'a Path>,
}

impl Network {
    pub async fn start(
        paths: Paths<'_>,
        policy: Option<EgressPolicy>,
        capacity: u16,
        deadline: Instant,
    ) -> Result<Self, RuntimeError> {
        let Paths { launcher, program, boots, lease, socket } = paths;
        let unrestricted = policy.is_none();
        if socket.is_some_and(|socket| !socket.starts_with(boots) || socket.parent().is_none()) {
            return Err(error("VM packet socket requires a private boot directory"));
        }
        if rustix::process::geteuid().is_root() || capacity == 0 {
            return Err(error("VM user-mode networking requires an ordinary user and positive capacity"));
        }
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
        let profile = tokio::time::timeout_at(deadline, async {
            let output = metadata.stdout.take().ok_or_else(|| error("Missing packet helper metadata pipe"))?;
            let metadata_value = read(output, pryx_agent::contract::MAX_REQUEST_BYTES).await?;
            if !metadata.wait().await.map_err(error)?.success() || !metadata_value["production"].is_object() {
                return Err(error("VM packet helper did not export its production profile"));
            }
            Ok(metadata_value["production"].clone())
        })
        .await
        .map_err(|_| RuntimeError::Timeout("VM packet helper metadata exceeded its deadline".into()))??;
        let configuration_argument = argument(&profile, "configurationArgument")?;
        let descriptor_argument = argument(&profile, "descriptorArgument")?;
        if configuration_argument == descriptor_argument {
            return Err(error("VM packet helper private arguments collide"));
        }
        let limit = profile["maxConfigurationBytes"]
            .as_u64()
            .ok_or_else(|| error("VM packet helper omitted its configuration bound"))?;
        let configuration = serde_json::to_vec(&serde_json::json!({
            "parent":std::process::id(), "host":Ipv4Addr::LOCALHOST, "concurrency":capacity
        }))
        .map_err(error)?;
        if configuration.len() as u64 > limit {
            return Err(error("VM packet helper configuration exceeds its bound"));
        }
        let directory = tempfile::Builder::new()
            .prefix("network-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(boots)
            .map_err(error)?;
        let config = directory.path().join("network.json");
        crate::durable::write_atomic(&config, &configuration, 0o600).map_err(error)?;
        let (ports, gateways) = crate::runtime::egress::listeners::listen(
            Ipv4Addr::LOCALHOST.into(),
            crate::runtime::egress::listeners::Ports { http: 0, tls: 0, dns: 0 },
            ipnet::IpNet::new(Ipv4Addr::LOCALHOST.into(), u32::BITS as u8).map_err(error)?,
            policy.map_or(crate::runtime::egress::Policy::Public, crate::runtime::egress::Policy::Names),
            usize::from(capacity),
        )
        .await
        .map_err(error)?;
        let socket = socket.map_or_else(|| directory.path().join("packet"), Path::to_owned);
        let read = vec![config.clone()];
        let plan = Plan {
            program: program.into(),
            arguments: vec![configuration_argument, config.display().to_string()],
            directory: socket.parent().ok_or_else(|| error("Missing packet socket directory"))?.into(),
            lock: lease.into(),
            parent: std::process::id(),
            read,
            write: Vec::new(),
            role: Role::PacketProxy { socket: socket.clone(), descriptor_argument },
        };
        let mut command = Command::new(launcher);
        command
            .arg(pryx_agent::vm_monitor::MODE)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(test)]
        command.stderr(Stdio::inherit());
        let child = command.spawn().map_err(error)?;
        // From this point a cancelled readiness wait owns asynchronous retirement too.
        let mut network = Self {
            child: Some(child),
            resources: Some(Resources { _gateways: gateways, _directory: directory }),
            profile,
            ports,
            socket,
            retirement: None,
            runtime: tokio::runtime::Handle::current(),
            unrestricted,
        };
        tokio::time::timeout_at(deadline, network.ready(&plan))
            .await
            .map_err(|_| RuntimeError::Timeout("VM packet helper readiness exceeded its deadline".into()))??;
        Ok(network)
    }

    async fn ready(&mut self, plan: &Plan) -> Result<(), RuntimeError> {
        let child = self.child.as_mut().ok_or_else(|| error("VM packet helper has retired"))?;
        let mut input = child.stdin.take().ok_or_else(|| error("Missing packet helper launch pipe"))?;
        input.write_all(&serde_json::to_vec(plan).map_err(error)?).await.map_err(error)?;
        input.shutdown().await.map_err(error)?;
        drop(input);
        let output = child.stdout.take().ok_or_else(|| error("Missing packet helper readiness pipe"))?;
        if read(output, pryx_agent::contract::MAX_REQUEST_BYTES).await? != self.profile
            || child.try_wait().map_err(error)?.is_some()
        {
            return Err(error("VM packet helper readiness differs from its executable metadata"));
        }
        Ok(())
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }
    pub fn profile(&self) -> &Value {
        &self.profile
    }
    /// HTTP, TLS and paired UDP/TCP DNS ports, in that order.
    pub fn ports(&self) -> (u16, u16, u16) {
        (self.ports.http, self.ports.tls, self.ports.dns)
    }
    pub fn guest(&self) -> Result<pryx_agent::vm_boot::Network, RuntimeError> {
        use pryx_agent::vm_boot::{self, NetworkPolicy, Redirect};
        let text = |name: &str| {
            self.profile[name].as_str().ok_or_else(|| error("VM packet helper omitted its network profile"))
        };
        let port = |name: &str| {
            self.profile[name]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .filter(|port| *port != 0)
                .ok_or_else(|| error("VM packet helper omitted its service port"))
        };
        Ok(vm_boot::Network {
            interface: text("interface")?.into(),
            mac: text("guestMac")?.into(),
            mtu: self.profile["mtu"]
                .as_u64()
                .and_then(|mtu| u32::try_from(mtu).ok())
                .ok_or_else(|| error("VM packet helper omitted its MTU"))?,
            vsock_port: Some(vm_boot::PACKET_PORT),
            policy: Some(NetworkPolicy {
                unrestricted: self.unrestricted,
                host: text("host")?.parse().map_err(error)?,
                http: Redirect { source: port("http")?, target: self.ports.http },
                tls: Redirect { source: port("tls")?, target: self.ports.tls },
                dns: Redirect { source: port("dns")?, target: self.ports.dns },
                dhcp_client: port("dhcpClient")?,
                dhcp_server: port("dhcpServer")?,
            }),
        })
    }
    pub fn exited(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.as_mut().ok_or_else(|| io::Error::other("VM packet helper has retired"))?.try_wait()
    }

    /// The cleanup task owns both resources even when its caller cancels this wait.
    pub async fn stop(&mut self) -> io::Result<()> {
        self.retire();
        let retirement =
            self.retirement.as_mut().ok_or_else(|| io::Error::other("Missing packet helper retirement"))?;
        retirement
            .wait_for(Option::is_some)
            .await
            .map_err(io::Error::other)?
            .as_ref()
            .expect("retirement observation is present")
            .clone()
            .map_err(io::Error::other)
    }
    fn retire(&mut self) {
        let Some(mut child) = self.child.take() else { return };
        let resources = self.resources.take();
        let (finished, observation) = tokio::sync::watch::channel(None);
        self.retirement = Some(observation);
        self.runtime.spawn(async move {
            let result = child.kill().await;
            if result.is_err() {
                // Never release gateways or private files while exit is uncertain.
                let _ = child.wait().await;
            }
            drop(resources);
            finished.send_replace(Some(result.map_err(|problem| problem.to_string())));
        });
    }
}
impl Drop for Network {
    fn drop(&mut self) {
        self.retire();
    }
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;
