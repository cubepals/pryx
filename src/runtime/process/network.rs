// SPDX-License-Identifier: FSL-1.1-ALv2
//! A process sandbox's network policy in the kernel (`crate::bpf`): a cgroup of its own, rules
//! from the same specification a container's network takes (internet or none, hostname egress,
//! address rules), hostname gateways pryxd serves it on loopback, and finding all of it again
//! after a daemon restart. Where the host can't attach programs this is absent: a sandbox runs
//! as before and a request for policy is refused. Not the launch (`power.rs`) nor the gateways'
//! own protocol (`runtime/egress/`), nor turning it on at start (`adoption.rs`).
use crate::bpf::{
    attach::{Attachment, Counts, Programs},
    host::{self, Host},
    object, rules,
};
use crate::runtime::egress::{Policy, Refused, listeners};
use crate::runtime::{ContainerSpec, RuntimeError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    net::Ipv4Addr,
    ops::RangeInclusive,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tokio_util::sync::DropGuard;

/// The ports pryxd hands out and its API's: no sandbox reaches them on a private address.
#[derive(Clone, Debug)]
pub struct Served {
    pub ports: RangeInclusive<u16>,
    pub api_port: u16,
}

/// What the journal keeps to find a sandbox's programs again.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Record {
    pub(super) key: String,
    pub(super) attachments: Vec<Attachment>,
}

struct Held {
    programs: Programs,
    _gateways: Option<DropGuard>,
}

pub(super) struct Network {
    host: Host,
    served: Served,
    root: PathBuf,
    held: Mutex<HashMap<String, Held>>,
}

pub(super) fn error(problem: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Other(format!("Kernel network policy: {problem}"))
}

/// Where a sandbox's gateways listen; only its own programs let it reach them.
const GATEWAY: Ipv4Addr = Ipv4Addr::LOCALHOST;
/// IPv6 link-local unicast (RFC 4291 §2.5.6), refused as the host firewall refuses IPv4's.
const LINK_LOCAL_V6: ipnet::IpNet =
    ipnet::IpNet::V6(ipnet::Ipv6Net::new_assert(std::net::Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0), 10));

/// The host firewall's ranges, as `ranges.rs` writes them for nftables.
fn ranges(list: &str) -> impl Iterator<Item = ipnet::IpNet> + '_ {
    list.split(", ").map(|net| net.parse().expect("the firewall's own ranges parse"))
}

/// A sandbox's hostname gateways, bound but not yet serving.
struct Gateways {
    http: tokio::net::TcpListener,
    tls: tokio::net::TcpListener,
    dns: (tokio::net::UdpSocket, tokio::net::TcpListener),
}

impl Gateways {
    async fn bind() -> Result<Self, RuntimeError> {
        let http = tokio::net::TcpListener::bind((GATEWAY, 0)).await.map_err(error)?;
        let tls = tokio::net::TcpListener::bind((GATEWAY, 0)).await.map_err(error)?;
        let dns = listeners::resolver(GATEWAY.into(), 0).await.map_err(error)?;
        Ok(Self { http, tls, dns })
    }
    /// The HTTP, TLS and DNS gateways' ports.
    fn ports(&self) -> Result<[u16; 3], RuntimeError> {
        let port = |address: std::io::Result<std::net::SocketAddr>| address.map(|a| a.port()).map_err(error);
        Ok([port(self.http.local_addr())?, port(self.tls.local_addr())?, port(self.dns.1.local_addr())?])
    }
    fn admit(&self, programs: &mut Programs) -> Result<(), RuntimeError> {
        let sockets: [&dyn AsRawFd; 4] = [&self.http, &self.tls, &self.dns.0, &self.dns.1];
        for socket in sockets {
            programs.admit_gateway(&socket.as_raw_fd()).map_err(error)?;
        }
        Ok(())
    }
    /// Serves the sandbox's hostname policy; no name reaches an upstream its own address rules
    /// refuse (`deny`), as its programs refuse the same address dialled directly.
    fn serve(
        self,
        policy: crate::protocol::EgressPolicy,
        deny: &[rules::Destination],
    ) -> Result<DropGuard, RuntimeError> {
        let subnet = ipnet::IpNet::from(std::net::IpAddr::from(GATEWAY));
        let refused = Refused::new(deny);
        let (_, guard) = listeners::serve(
            self.http,
            self.tls,
            self.dns,
            subnet,
            Policy::Names(policy),
            refused,
            crate::runtime::docker::egress::MAX_CONNECTIONS,
        )
        .map_err(error)?;
        Ok(guard)
    }
}

impl Network {
    pub(super) fn new(host: Host, served: Served, root: &Path) -> Self {
        Self { host, served, root: root.to_path_buf(), held: Mutex::new(HashMap::new()) }
    }

    /// A sandbox's name on the host's cgroup and BPF filesystem, distinct per state directory.
    fn key(&self, name: &str) -> String {
        let digest = Sha256::new().chain_update(self.root.as_os_str().as_encoded_bytes()).chain_update(b"\0");
        hex::encode(digest.chain_update(name.as_bytes()).finalize())[..16].to_owned()
    }

    fn reach(spec: &ContainerSpec) -> rules::Reach {
        if spec.internal_network {
            rules::Reach::None
        } else if spec.egress.is_some() {
            rules::Reach::Hostnames
        } else {
            rules::Reach::Public
        }
    }

    /// The gateways' ports, and the ports pryxd published for the sandbox (its agent's and its
    /// spares, which the edge dials): the only ones its programs let reach it from off this host.
    fn config(spec: &ContainerSpec, gateways: Option<&Gateways>) -> Result<[u8; object::CONFIG_BYTES], RuntimeError> {
        let [http, tls, dns] = match gateways {
            Some(gateways) => gateways.ports()?,
            None => [0; 3],
        };
        let published: Vec<u16> = spec
            .ports
            .iter()
            .filter(|binding| binding.protocol == crate::protocol::Proto::Tcp)
            .map(|binding| binding.host_port)
            .collect();
        object::config_bytes(GATEWAY, http, tls, dns, &published).ok_or_else(|| {
            error(format!("{} published ports; at most {} fit", published.len(), object::PUBLISHED_PORTS))
        })
    }

    /// Whether a spec asked for what only the programs enforce on this tier.
    pub(super) fn needs_programs(spec: &ContainerSpec) -> bool {
        spec.egress.is_some() || spec.egress_addresses.is_some()
    }

    /// A spec's own address rules, allow and deny.
    fn destinations(spec: &ContainerSpec) -> Result<(Vec<rules::Destination>, Vec<rules::Destination>), RuntimeError> {
        let destinations = |list: &[crate::protocol::AddressRule]| {
            list.iter()
                .map(|rule| {
                    let (prefix, ports) =
                        rule.parsed().ok_or_else(|| error(format!("invalid address rule {}", rule.prefix)))?;
                    Ok(rules::Destination { prefix, ports })
                })
                .collect::<Result<Vec<_>, RuntimeError>>()
        };
        match &spec.egress_addresses {
            Some(addresses) => Ok((destinations(&addresses.allow)?, destinations(&addresses.deny)?)),
            None => Ok((Vec::new(), Vec::new())),
        }
    }

    fn compile(&self, spec: &ContainerSpec) -> Result<BTreeMap<ipnet::Ipv6Net, object::Rule>, RuntimeError> {
        let private: Vec<ipnet::IpNet> =
            ranges(crate::ranges::PRIVATE_V4).chain(ranges(crate::ranges::PRIVATE_V6)).collect();
        let link_local: Vec<ipnet::IpNet> =
            ranges(crate::ranges::LINK_LOCAL_V4).chain(std::iter::once(LINK_LOCAL_V6)).collect();
        let (allow, deny) = Self::destinations(spec)?;
        rules::compile(&rules::Inputs {
            reach: Self::reach(spec),
            allow: &allow,
            deny: &deny,
            host: &host::addresses(),
            nameservers: &crate::runtime::egress::dns::nameservers(),
            served: self.served.ports.clone(),
            api_port: self.served.api_port,
            private: &private,
            link_local: &link_local,
            dns: crate::runtime::egress::dns::PORT,
        })
        .map_err(error)
    }

    /// Before a supervisor starts: its cgroup, its programs and rules, and its gateways. The
    /// supervisor must join the returned cgroup before it runs.
    pub(super) async fn prepare(&self, spec: &ContainerSpec) -> Result<(Record, PathBuf), RuntimeError> {
        self.release(&spec.name)?;
        let cgroup = self.host.cgroup(&self.key(&spec.name));
        host::create_cgroup(&cgroup).map_err(error)?;
        let prepared = self.attach(spec, &cgroup).await;
        if prepared.is_err() {
            let _ = host::remove_cgroup(&cgroup);
        }
        prepared.map(|record| (record, cgroup))
    }

    async fn attach(&self, spec: &ContainerSpec, cgroup: &Path) -> Result<Record, RuntimeError> {
        let key = self.key(&spec.name);
        let rules = self.compile(spec)?;
        let (_, deny) = Self::destinations(spec)?;
        let gateways = match spec.egress.is_some() {
            true => Some(Gateways::bind().await?),
            false => None,
        };
        let config = Self::config(spec, gateways.as_ref())?;
        let mut programs =
            Programs::attach(object::OBJECT, &self.host.pin_directory(&key), cgroup, &rules, config).map_err(error)?;
        let guard = match (gateways, &spec.egress) {
            (Some(gateways), Some(policy)) => {
                match gateways.admit(&mut programs).and_then(|()| gateways.serve(policy.clone(), &deny)) {
                    Ok(guard) => Some(guard),
                    Err(problem) => {
                        let _ = programs.detach();
                        return Err(problem);
                    }
                }
            }
            _ => None,
        };
        let record = Record { key, attachments: programs.attachments.clone() };
        self.held.lock().unwrap().insert(spec.name.clone(), Held { programs, _gateways: guard });
        Ok(record)
    }

    /// After a daemon restart, for a supervisor still running: its programs found by their pins
    /// and checked, its rules written again for today's host, and its gateways served again.
    pub(super) async fn adopt(&self, spec: &ContainerSpec, record: &Record) -> Result<(), RuntimeError> {
        if record.key != self.key(&spec.name) {
            return Err(error("the journal names another sandbox's programs"));
        }
        let mut programs =
            Programs::adopt(&self.host.pin_directory(&record.key), &record.attachments).map_err(error)?;
        let rules = self.compile(spec)?;
        let gateways = match spec.egress.is_some() {
            true => Some(Gateways::bind().await?),
            false => None,
        };
        let config = Self::config(spec, gateways.as_ref())?;
        if let Some(gateways) = &gateways {
            gateways.admit(&mut programs)?;
        }
        programs.write(&rules, config).map_err(error)?;
        let guard = match (gateways, &spec.egress) {
            (Some(gateways), Some(policy)) => Some(gateways.serve(policy.clone(), &Self::destinations(spec)?.1)?),
            _ => None,
        };
        self.held.lock().unwrap().insert(spec.name.clone(), Held { programs, _gateways: guard });
        Ok(())
    }

    /// Once a sandbox's supervisor is gone: whatever is left in its cgroup is killed, then its
    /// programs are detached and the cgroup removed. Idempotent.
    pub(super) fn release(&self, name: &str) -> Result<(), RuntimeError> {
        let key = self.key(name);
        let held = self.held.lock().unwrap().remove(name);
        let cgroup = self.host.cgroup(&key);
        if cgroup.exists() && !host::members(&cgroup).map_err(error)?.is_empty() {
            // cgroup.kill (Linux 5.14) kills every process the cgroup holds, escaped ones too.
            std::fs::write(cgroup.join("cgroup.kill"), b"1").map_err(error)?;
        }
        let detached = match held {
            Some(held) => {
                if let Ok(counts) = held.programs.counts() {
                    tracing::info!(
                        sandbox = name,
                        received = counts.ingress[1],
                        sent = counts.egress[1],
                        refused_destinations = counts.refused_addresses,
                        refused_sockets = counts.refused_sockets,
                        refused_ingress = counts.refused_ingress[0],
                        "kernel network policy detached"
                    );
                }
                held.programs.detach()
            }
            None => crate::bpf::attach::remove_pins(&self.host.pin_directory(&key)),
        };
        host::remove_cgroup(&cgroup).map_err(error)?;
        detached.map_err(error)
    }

    pub(super) fn verified(&self, name: &str) -> bool {
        self.held.lock().unwrap().contains_key(name)
    }

    pub(super) fn counts(&self, name: &str) -> Option<Counts> {
        self.held.lock().unwrap().get(name).and_then(|held| held.programs.counts().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sandbox_key_is_stable_per_state_directory_and_name() {
        let network = |root: &str| Network {
            host: Host { cgroups: "/sys/fs/cgroup/x".into(), pins: "/sys/fs/bpf/pryx".into(), delegated: false },
            served: Served { ports: 42000..=42999, api_port: 7443 },
            root: root.into(),
            held: Mutex::new(HashMap::new()),
        };
        let (a, b) = (network("/var/lib/pryxd/process-runtime"), network("/tmp/other/process-runtime"));
        assert_eq!(a.key("box"), a.key("box"));
        assert_ne!(a.key("box"), a.key("other"));
        assert_ne!(a.key("box"), b.key("box"));
        assert_eq!(a.key("box").len(), 16);
    }
}
