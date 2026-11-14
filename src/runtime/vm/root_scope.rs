// SPDX-License-Identifier: FSL-1.1-ALv2
//! Own a separate root TAP helper and adopt its private gateway sockets in the original
//! namespace. Parent EOF requests retirement; the helper independently waits for the actual
//! monitor lease. This does not enable root placement or provide unrestricted IP forwarding.
//!
//! Parts (`root_scope/`):
//! - `handoff.rs`: bounded close-on-exec namespace/listener descriptor receiving.
//! - `tests.rs`: real descriptor rejection and bounded-root gateway/lease acceptance.
mod handoff;
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

struct Resources {
    namespace: Option<OwnedFd>,
    gateways: Option<DropGuard>,
    directory: tempfile::TempDir,
}

pub struct Scope {
    child: Option<Child>,
    resources: Option<Resources>,
    profile: Value,
    tap: String,
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

async fn receive(socket: UnixDatagram, metadata: &Advertisement) -> io::Result<Grant> {
    socket.set_nonblocking(true)?;
    let socket = tokio::net::UnixDatagram::from_std(socket)?;
    socket.async_io(Interest::READABLE, || handoff::receive(&socket, metadata)).await
}

fn tcp(file: OwnedFd, host: std::net::Ipv4Addr) -> io::Result<tokio::net::TcpListener> {
    let listener = std::net::TcpListener::from(file);
    if listener.local_addr()?.ip() != host || listener.local_addr()?.port() == 0 {
        return Err(handoff::invalid());
    }
    listener.set_nonblocking(true)?;
    tokio::net::TcpListener::from_std(listener)
}

impl Scope {
    pub async fn start(
        paths: Paths<'_>,
        policy: EgressPolicy,
        capacity: u16,
        deadline: Instant,
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
        let child = command.spawn().map_err(error)?;
        // Own asynchronous EOF/lease retirement before the first cancellable readiness wait.
        let mut scope = Self {
            child: Some(child),
            resources: Some(Resources { namespace: None, gateways: None, directory }),
            profile: Value::Null,
            tap: String::new(),
            ports: crate::runtime::egress::listeners::Ports { http: 0, tls: 0, dns: 0 },
            retirement: None,
            runtime: tokio::runtime::Handle::current(),
        };
        let grant = tokio::time::timeout_at(deadline, receive(socket, &metadata))
            .await
            .map_err(|_| RuntimeError::Timeout("Root scope handoff exceeded its deadline".into()))?
            .map_err(error)?;
        scope.adopt(grant, policy, capacity).map_err(error)?;
        if scope.exited().map_err(error)?.is_some() {
            return Err(error("Root scope exited during handoff"));
        }
        Ok(scope)
    }

    fn adopt(&mut self, grant: Grant, policy: EgressPolicy, capacity: u16) -> io::Result<()> {
        let Grant { namespace, http, tls, dns_tcp, dns_udp, profile, tap } = grant;
        let current = fs::metadata("/proc/self/ns/net")?;
        let granted = rustix::fs::fstat(&namespace)?;
        if rustix::fs::fstatfs(&namespace)?.f_type != libc::NSFS_MAGIC
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
            tokio::net::UdpSocket::from_std(dns_udp)?,
            dns_tcp,
            subnet.into(),
            crate::runtime::egress::Policy::Names(policy),
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
