// SPDX-License-Identifier: FSL-1.1-ALv2
//! A process sandbox's network policy in the kernel (`crate::bpf`): a cgroup of its own, rules
//! from the same specification a container's network takes (internet or none, hostname egress,
//! address rules), hostname gateways pryxd serves it on loopback, and finding all of it again
//! after a daemon restart. Where the host can't attach programs this is absent: a sandbox runs
//! as before and a request for policy is refused. Not the launch (`power.rs`) nor the gateways'
//! own protocol (`runtime/egress/`).
use super::*;
use crate::bpf::{
    attach::{Attachment, Counts, Programs},
    host::{self, Host},
    object, rules,
};
use crate::runtime::egress::{Policy, listeners};
use sha2::{Digest, Sha256};
use std::{net::Ipv4Addr, ops::RangeInclusive, os::fd::AsRawFd, path::Path};
use tokio_util::sync::DropGuard;

/// The ports pryxd hands out and its API's: no sandbox reaches them on a private address.
#[derive(Clone, Debug)]
pub(crate) struct Served {
    pub(crate) ports: RangeInclusive<u16>,
    pub(crate) api_port: u16,
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

fn error(problem: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Other(format!("Kernel network policy: {problem}"))
}

/// Where a sandbox's gateways listen; only its own programs let it reach them.
const GATEWAY: Ipv4Addr = Ipv4Addr::LOCALHOST;

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
    fn config(&self) -> Result<[u8; object::CONFIG_BYTES], RuntimeError> {
        let port = |address: std::io::Result<std::net::SocketAddr>| address.map(|a| a.port()).map_err(error);
        Ok(object::config_bytes(
            GATEWAY,
            port(self.http.local_addr())?,
            port(self.tls.local_addr())?,
            port(self.dns.1.local_addr())?,
        ))
    }
    fn admit(&self, programs: &mut Programs) -> Result<(), RuntimeError> {
        let sockets: [&dyn AsRawFd; 4] = [&self.http, &self.tls, &self.dns.0, &self.dns.1];
        for socket in sockets {
            programs.admit_gateway(&socket.as_raw_fd()).map_err(error)?;
        }
        Ok(())
    }
    fn serve(self, policy: crate::protocol::EgressPolicy) -> Result<DropGuard, RuntimeError> {
        let subnet = ipnet::IpNet::from(std::net::IpAddr::from(GATEWAY));
        let (udp, tcp) = self.dns;
        let (_, guard) = listeners::serve(
            self.http,
            self.tls,
            udp,
            tcp,
            subnet,
            Policy::Names(policy),
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

    /// Whether a spec asked for what only the programs enforce on this tier.
    fn needs_programs(spec: &ContainerSpec) -> bool {
        spec.egress.is_some() || spec.egress_addresses.is_some()
    }

    fn compile(&self, spec: &ContainerSpec) -> Result<BTreeMap<ipnet::Ipv6Net, object::Rule>, RuntimeError> {
        let destinations = |list: &[crate::protocol::AddressRule]| {
            list.iter()
                .map(|rule| rule.destination().ok_or_else(|| error(format!("invalid address rule {}", rule.prefix))))
                .collect::<Result<Vec<_>, _>>()
        };
        let (allow, deny) = match &spec.egress_addresses {
            Some(addresses) => (destinations(&addresses.allow)?, destinations(&addresses.deny)?),
            None => (Vec::new(), Vec::new()),
        };
        rules::compile(&rules::Inputs {
            reach: Self::reach(spec),
            allow: &allow,
            deny: &deny,
            host: &host::addresses(),
            nameservers: &crate::runtime::egress::dns::nameservers(),
            served: self.served.ports.clone(),
            api_port: self.served.api_port,
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
        let gateways = match spec.egress.is_some() {
            true => Some(Gateways::bind().await?),
            false => None,
        };
        let config = match &gateways {
            Some(gateways) => gateways.config()?,
            None => object::config_bytes(GATEWAY, 0, 0, 0),
        };
        let mut programs = Programs::attach(&self.host.pin_directory(&key), cgroup, &rules, config).map_err(error)?;
        let guard = match (gateways, &spec.egress) {
            (Some(gateways), Some(policy)) => {
                match gateways.admit(&mut programs).and_then(|()| gateways.serve(policy.clone())) {
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
        let config = match &gateways {
            Some(gateways) => gateways.config()?,
            None => object::config_bytes(GATEWAY, 0, 0, 0),
        };
        if let Some(gateways) = &gateways {
            gateways.admit(&mut programs)?;
        }
        programs.write(&rules, config).map_err(error)?;
        let guard = match (gateways, &spec.egress) {
            (Some(gateways), Some(policy)) => Some(gateways.serve(policy.clone())?),
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
            Some(held) => held.programs.detach(),
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

impl ProcessRuntime {
    /// Turns kernel network policy on where the host proves it can attach the programs, and
    /// finds those of supervisors that outlived the last daemon. A sandbox whose address or
    /// hostname policy can't be verified again is stopped rather than left running without it.
    pub(crate) async fn with_network(mut self, served: Served) -> Self {
        let found = crate::bpf::probe();
        self.network_capability = Some(crate::bpf::capability(&found));
        let Ok(host) = found else { return self };
        let network = Arc::new(Network::new(host, served, &self.state.root));
        for name in self.names() {
            let Ok(entry) = self.entry(&name) else { continue };
            if super::observe::matching_pid(entry.pid, entry.ticks).is_none() {
                continue;
            }
            let adopted = match &entry.network {
                Some(record) => network.adopt(&entry.spec, record).await,
                None => Err(error("the sandbox started without it")),
            };
            if let Err(problem) = adopted {
                tracing::warn!(sandbox = %entry.spec.name, %problem, "process sandbox has no verified kernel network policy");
                if entry.network.is_some() && Network::needs_programs(&entry.spec) {
                    let _ = self.terminate_tree(&name).await;
                }
            }
        }
        self.network = Some(network);
        self
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
