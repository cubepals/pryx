// SPDX-License-Identifier: FSL-1.1-ALv2
//! Own a separate root TAP helper and adopt its private gateway sockets in the original
//! namespace. Public bridge adoption retains its actual namespace and attached queue. Parent
//! EOF requests retirement behind the monitor lease; this does not enable root placement.
//!
//! Parts (`root_scope/`):
//! - `packet.rs`: descriptor-only configuration, fixed packet launch and kernel identity.
//! - `handoff.rs`: bounded close-on-exec namespace/listener descriptor receiving.
//! - `tests.rs`: real descriptor rejection and bounded-root gateway/lease acceptance.
mod handoff;
mod packet;
#[cfg(test)]
mod tests;
use super::error;
use crate::{protocol::EgressPolicy, runtime::RuntimeError};
use handoff::{Advertisement, Grant};
use serde_json::Value;
use std::{
    fs, io,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::{
            fs::{MetadataExt, PermissionsExt},
            net::UnixDatagram,
        },
    },
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
};
use tokio::{
    io::{AsyncReadExt, Interest},
    process::{Child, Command},
    time::Instant,
};
use tokio_util::sync::DropGuard;

pub struct Paths<'a> {
    pub program: &'a Path,
    pub plugins: &'a Path,
    pub boots: &'a Path,
    pub lease: &'a Path,
    pub owner: (u32, u32),
}

pub struct BridgePrograms<'a> {
    pub agent: &'a Path,
    pub packet: &'a Path,
}

struct Resources {
    namespace: Option<OwnedFd>,
    gateways: Option<DropGuard>,
    packet: Option<OwnedFd>,
    configuration: Option<OwnedFd>,
    identity: Option<OwnedFd>,
    channel: Option<tokio::net::UnixDatagram>,
    directory: tempfile::TempDir,
}

pub struct Scope {
    child: Option<Child>,
    resources: Option<Resources>,
    profile: Value,
    packet_paths: Option<handoff::PacketPaths>,
    packet_profile: Value,
    packet_metadata: Option<Advertisement>,
    tap: String,
    packet_tap: String,
    ports: crate::runtime::egress::listeners::Ports,
    retirement: Option<tokio::sync::watch::Receiver<Option<Result<(), String>>>>,
    runtime: tokio::runtime::Handle,
}

async fn advertisement(program: &Path) -> Result<Advertisement, RuntimeError> {
    let mut child = Command::new(program)
        .arg("--metadata")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(error)?;
    let output = child.stdout.take().ok_or_else(|| error("Missing root scope metadata pipe"))?;
    let mut bytes = Vec::new();
    output.take(pryx_agent::contract::MAX_REQUEST_BYTES as u64 + 1).read_to_end(&mut bytes).await.map_err(error)?;
    if bytes.len() > pryx_agent::contract::MAX_REQUEST_BYTES || !child.wait().await.map_err(error)?.success() {
        return Err(error("Root scope metadata exceeds its bound or its program refused"));
    }
    let metadata: Advertisement = serde_json::from_slice(&bytes).map_err(error)?;
    metadata.validate().map_err(error)?;
    Ok(metadata)
}

enum Received {
    Names(Grant),
    Bridge(handoff::BridgeGrant),
}

async fn receive(socket: &tokio::net::UnixDatagram, metadata: &Advertisement, bridge: bool) -> io::Result<Received> {
    socket
        .async_io(Interest::READABLE, || {
            if bridge {
                handoff::receive_bridge(socket, metadata).map(Received::Bridge)
            } else {
                handoff::receive(socket, metadata).map(Received::Names)
            }
        })
        .await
}

fn tcp(file: OwnedFd, host: std::net::Ipv4Addr) -> io::Result<tokio::net::TcpListener> {
    let listener = std::net::TcpListener::from(file);
    if listener.local_addr()?.ip() != host || listener.local_addr()?.port() == 0 {
        return Err(handoff::invalid());
    }
    listener.set_nonblocking(true)?;
    tokio::net::TcpListener::from_std(listener)
}

fn validate_profile(
    namespace: &OwnedFd,
    profile: &Value,
    tap: &str,
) -> io::Result<(std::net::Ipv4Addr, ipnet::Ipv4Net)> {
    let current = fs::metadata("/proc/self/ns/net")?;
    let granted = rustix::fs::fstat(namespace)?;
    if rustix::fs::fstatfs(namespace)?.f_type != libc::NSFS_MAGIC
        || (current.dev(), current.ino()) == (granted.st_dev, granted.st_ino)
    {
        return Err(handoff::invalid());
    }
    let host: std::net::Ipv4Addr =
        profile["host"].as_str().ok_or_else(handoff::invalid)?.parse().map_err(|_| handoff::invalid())?;
    let subnet: ipnet::Ipv4Net =
        profile["subnet"].as_str().ok_or_else(handoff::invalid)?.parse().map_err(|_| handoff::invalid())?;
    if !host.is_private() || !subnet.contains(&host) || tap.is_empty() || tap.contains('/') {
        return Err(handoff::invalid());
    }
    Ok((host, subnet))
}

impl Scope {
    pub async fn start(
        paths: Paths<'_>,
        policy: EgressPolicy,
        capacity: u16,
        deadline: Instant,
    ) -> Result<Self, RuntimeError> {
        Self::start_mode(paths, Some(policy), capacity, deadline, None, false).await
    }
    pub async fn start_bridge(
        paths: Paths<'_>,
        programs: BridgePrograms<'_>,
        deadline: Instant,
    ) -> Result<Self, RuntimeError> {
        Self::start_mode(paths, None, 1, deadline, Some(programs), false).await
    }
    pub async fn start_packet(
        paths: Paths<'_>,
        programs: BridgePrograms<'_>,
        deadline: Instant,
    ) -> Result<Self, RuntimeError> {
        Self::start_mode(paths, None, 1, deadline, Some(programs), true).await
    }
    async fn start_mode(
        paths: Paths<'_>,
        policy: Option<EgressPolicy>,
        capacity: u16,
        deadline: Instant,
        bridge: Option<BridgePrograms<'_>>,
        packet_runtime: bool,
    ) -> Result<Self, RuntimeError> {
        if !rustix::process::geteuid().is_root() || paths.owner.0 == 0 || paths.owner.1 == 0 || capacity == 0 {
            return Err(error("Root TAP scope requires fixed ordinary monitor IDs and positive capacity"));
        }
        super::bundle::ancestors(paths.program.parent().ok_or_else(|| error("Missing root scope directory"))?)
            .map_err(error)?;
        let executable = fs::symlink_metadata(paths.program).map_err(error)?;
        if !executable.is_file()
            || executable.uid() != rustix::process::Uid::ROOT.as_raw()
            || executable.nlink() != 1
            || executable.mode() & 0o022 != 0
            || executable.mode() & 0o111 == 0
        {
            return Err(error("Root scope requires one protected root-owned executable"));
        }
        let metadata = tokio::time::timeout_at(deadline, advertisement(paths.program))
            .await
            .map_err(|_| RuntimeError::Timeout("Root scope metadata exceeded its deadline".into()))??;
        let metadata = if bridge.is_some() { metadata.bridge().map_err(error)? } else { metadata };
        let metadata = if packet_runtime { metadata.packet().map_err(error)? } else { metadata };
        let directory = tempfile::Builder::new()
            .prefix("root-network-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(paths.boots)
            .map_err(error)?;
        let (socket, output) = UnixDatagram::pair().map_err(error)?;
        let mut command = Command::new(paths.program);
        command
            .args([
                "--plugins",
                &paths.plugins.display().to_string(),
                "--state",
                &directory.path().display().to_string(),
                "--lease",
                &paths.lease.display().to_string(),
                "--uid",
                &paths.owner.0.to_string(),
                "--gid",
                &paths.owner.1.to_string(),
            ])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::from(OwnedFd::from(output)))
            .stderr(Stdio::null());
        #[cfg(test)]
        command.stderr(Stdio::inherit());
        if let Some(programs) = &bridge {
            command.args([
                &metadata.bridge_agent_argument,
                &programs.agent.display().to_string(),
                &metadata.bridge_packet_argument,
                &programs.packet.display().to_string(),
            ]);
        }
        if packet_runtime {
            command.arg(&metadata.packet_runtime_argument);
        }
        let child = command.spawn().map_err(error)?;
        // Own asynchronous EOF/lease retirement before the first cancellable readiness wait.
        let mut scope = Self {
            child: Some(child),
            resources: Some(Resources {
                namespace: None,
                gateways: None,
                packet: None,
                configuration: None,
                identity: None,
                channel: None,
                directory,
            }),
            profile: Value::Null,
            packet_paths: None,
            packet_profile: Value::Null,
            packet_metadata: None,
            tap: String::new(),
            packet_tap: String::new(),
            ports: crate::runtime::egress::listeners::Ports { http: 0, tls: 0, dns: 0 },
            retirement: None,
            runtime: tokio::runtime::Handle::current(),
        };
        socket.set_nonblocking(true).map_err(error)?;
        let socket = tokio::net::UnixDatagram::from_std(socket).map_err(error)?;
        let grant = tokio::time::timeout_at(deadline, receive(&socket, &metadata, bridge.is_some()))
            .await
            .map_err(|_| RuntimeError::Timeout("Root scope handoff exceeded its deadline".into()))?
            .map_err(error)?;
        match grant {
            Received::Names(grant) => {
                scope.adopt(grant, policy.ok_or_else(|| error("Missing hostname policy"))?, capacity).map_err(error)?
            }
            Received::Bridge(grant) => scope.adopt_bridge(grant).map_err(error)?,
        }
        if packet_runtime {
            scope.resources.as_mut().ok_or_else(|| error("Missing root packet resources"))?.channel = Some(socket);
            scope.packet_metadata = Some(metadata);
        }
        if scope.exited().map_err(error)?.is_some() {
            return Err(error("Root scope exited during handoff"));
        }
        Ok(scope)
    }

    fn adopt(&mut self, grant: Grant, policy: EgressPolicy, capacity: u16) -> io::Result<()> {
        let Grant { namespace, http, tls, dns_tcp, dns_udp, profile, tap } = grant;
        let (host, subnet) = validate_profile(&namespace, &profile, &tap)?;
        let http = tcp(http, host)?;
        let tls = tcp(tls, host)?;
        let dns_tcp = tcp(dns_tcp, host)?;
        let dns_udp = std::net::UdpSocket::from(dns_udp);
        if dns_udp.local_addr()? != dns_tcp.local_addr()? {
            return Err(handoff::invalid());
        }
        dns_udp.set_nonblocking(true)?;
        let (ports, gateways) = crate::runtime::egress::listeners::serve(
            http,
            tls,
            (tokio::net::UdpSocket::from_std(dns_udp)?, dns_tcp),
            subnet.into(),
            crate::runtime::egress::Policy::Names(policy),
            crate::runtime::egress::Refused::default(),
            usize::from(capacity),
        )?;
        let resources = self.resources.as_mut().ok_or_else(handoff::invalid)?;
        resources.namespace = Some(namespace);
        resources.gateways = Some(gateways);
        self.profile = profile;
        self.tap = tap;
        self.ports = ports;
        Ok(())
    }

    fn adopt_bridge(&mut self, grant: handoff::BridgeGrant) -> io::Result<()> {
        let handoff::BridgeGrant { namespace, packet, profile, tap, packet_tap, configuration, paths, packet_profile } =
            grant;
        validate_profile(&namespace, &profile, &tap)?;
        let actual = rustix::fs::fstat(&packet)?;
        if actual.st_mode & libc::S_IFMT != libc::S_IFCHR
            || actual.st_rdev != fs::metadata(pryx_agent::vm_monitor::TUN_DEVICE)?.rdev()
        {
            return Err(handoff::invalid());
        }
        let resources = self.resources.as_mut().ok_or_else(handoff::invalid)?;
        resources.namespace = Some(namespace);
        resources.packet = Some(packet);
        resources.configuration = configuration;
        self.packet_paths = paths;
        self.packet_profile = packet_profile;
        self.profile = profile;
        self.tap = tap;
        self.packet_tap = packet_tap;
        Ok(())
    }
    pub fn packet_queue(&self) -> io::Result<OwnedFd> {
        let queue =
            self.resources.as_ref().and_then(|resources| resources.packet.as_ref()).ok_or_else(handoff::invalid)?;
        rustix::io::fcntl_dupfd_cloexec(queue, libc::STDERR_FILENO + 1).map_err(Into::into)
    }
    pub fn packet_tap(&self) -> &str {
        &self.packet_tap
    }

    pub fn namespace(&self) -> io::Result<PathBuf> {
        let namespace =
            self.resources.as_ref().and_then(|resources| resources.namespace.as_ref()).ok_or_else(handoff::invalid)?;
        Ok(PathBuf::from(format!("/proc/{}/fd/{}", std::process::id(), namespace.as_raw_fd())))
    }
    pub fn monitor_namespace(&self) -> io::Result<pryx_agent::vm_monitor::Namespace> {
        let namespace =
            self.resources.as_ref().and_then(|resources| resources.namespace.as_ref()).ok_or_else(handoff::invalid)?;
        let identity = rustix::fs::fstat(namespace)?;
        Ok(pryx_agent::vm_monitor::Namespace {
            descriptor: namespace.as_raw_fd(),
            device: identity.st_dev,
            inode: identity.st_ino,
        })
    }
    pub fn profile(&self) -> &Value {
        &self.profile
    }
    pub fn tap(&self) -> &str {
        &self.tap
    }
    pub fn ports(&self) -> (u16, u16, u16) {
        (self.ports.http, self.ports.tls, self.ports.dns)
    }
    pub fn exited(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.as_mut().ok_or_else(handoff::invalid)?.try_wait()
    }
    pub async fn stop(&mut self) -> io::Result<()> {
        self.retire();
        self.retirement
            .as_mut()
            .ok_or_else(handoff::invalid)?
            .wait_for(Option::is_some)
            .await
            .map_err(io::Error::other)?
            .as_ref()
            .expect("scope retirement was observed")
            .clone()
            .map_err(io::Error::other)
    }
    fn retire(&mut self) {
        let Some(mut child) = self.child.take() else { return };
        drop(child.stdin.take());
        let resources = self.resources.take();
        let (finished, observation) = tokio::sync::watch::channel(None);
        self.retirement = Some(observation);
        self.runtime.spawn(async move {
            // Never kill a scope to detach a live monitor. EOF asks the helper to await its lease.
            let result = match child.wait().await {
                Ok(status) => {
                    if status.success() {
                        drop(resources);
                        Ok(())
                    } else {
                        if let Some(resources) = resources {
                            let _ = resources.directory.keep();
                        }
                        Err(io::Error::other("Root scope cleanup failed"))
                    }
                }
                Err(problem) => {
                    // Unconfirmed exit cannot release live namespace or gateway authority.
                    std::mem::forget(resources);
                    Err(problem)
                }
            };
            finished.send_replace(Some(result.map_err(|problem| problem.to_string())));
        });
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        self.retire();
    }
}
