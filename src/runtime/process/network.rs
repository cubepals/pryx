// SPDX-License-Identifier: FSL-1.1-ALv2
//! A process sandbox's network policy in the kernel (`crate::bpf`): a cgroup of its own, rules
//! from the same specification a container's network takes (internet or none, hostname egress,
//! address rules), hostname gateways pryxd serves it on loopback, every live sandbox's rules kept
//! for this host's addresses as they change, and finding all of it again after a daemon restart. Where the host can't attach programs this is absent: a sandbox runs
//! as before and a request for policy is refused. Not the launch (`power.rs`) nor the gateways'
//! own protocol (`runtime/egress/`), nor turning it on at start (`adoption.rs`).
//!
//! Parts (`network/`):
//! - `guard_tests.rs`: pure stale-map and pinned-policy retirement regressions.
//! - `route_fixture.py`: owned and refused sockets inside the real kernel-policy test cgroup.
//! - `path_tests.rs`: where a policed sandbox's packets actually go: broadcasts, NAT64 twins,
//!   reuse-port groups, source routes and TIME-WAIT.
//! - `paths_fixture.py`: the sandbox's side of each of those, inside the real test cgroup.
use crate::bpf::{
    attach::{Attachment, Counts, Programs},
    host::{self, AddressChanges, Host},
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
    /// Legacy records have no object identity and cannot retain a newer policy guarantee.
    #[serde(default)]
    pub(super) object_sha256: String,
}

/// What a sandbox's rules are compiled from besides the host, kept so they can be compiled again
/// when this host's addresses change.
struct Asked {
    reach: rules::Reach,
    allow: Vec<rules::Destination>,
    deny: Vec<rules::Destination>,
}

struct Held {
    programs: Programs,
    asked: Asked,
    config: [u8; object::CONFIG_BYTES],
    /// This host's addresses as its rules name them.
    host: host::Addresses,
    /// Its policy could not be verified: the cgroup is fenced and may not run again until a
    /// fresh prepare removes it and attaches a new set of programs.
    stale: bool,
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
/// The private journal must bind the same compiled policy this daemon currently proves.
const OBJECT_MISMATCH: &str = "the journal names another program object; stop and start with current programs";
/// IPv6 link-local unicast (RFC 4291 §2.5.6), refused as the host firewall refuses IPv4's.
const LINK_LOCAL_V6: ipnet::IpNet =
    ipnet::IpNet::V6(ipnet::Ipv6Net::new_assert(std::net::Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0), 10));

/// AWS's IPv6 metadata address, as `ranges.rs` writes it for nftables.
fn metadata_v6() -> std::net::IpAddr {
    crate::ranges::METADATA_V6.parse().expect("the firewall's own address parses")
}

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

    fn reach(internal_network: bool, hostnames: bool) -> rules::Reach {
        if internal_network {
            rules::Reach::None
        } else if hostnames {
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

    /// What a spec asks of its programs: its reach and its own address rules, allow and deny.
    fn asked(spec: &ContainerSpec) -> Result<Asked, RuntimeError> {
        let reach = Self::reach(spec.internal_network, spec.egress.is_some());
        Self::asked_of(reach, spec.egress_addresses.as_ref())
    }

    fn asked_of(
        reach: rules::Reach,
        addresses: Option<&crate::protocol::AddressPolicy>,
    ) -> Result<Asked, RuntimeError> {
        let destinations = |list: &[crate::protocol::AddressRule]| {
            list.iter()
                .map(|rule| {
                    let (prefix, ports) =
                        rule.parsed().ok_or_else(|| error(format!("invalid address rule {}", rule.prefix)))?;
                    Ok(rules::Destination { prefix, ports })
                })
                .collect::<Result<Vec<_>, RuntimeError>>()
        };
        let (allow, deny) = match addresses {
            Some(addresses) => (destinations(&addresses.allow)?, destinations(&addresses.deny)?),
            None => (Vec::new(), Vec::new()),
        };
        Ok(Asked { reach, allow, deny })
    }

    /// Whether a request's address rules fit its programs on this host as it is now: compiled as
    /// its start would compile them, so what can't be is refused, naming the prefix, before
    /// anything is made.
    pub(super) fn check(&self, spec: &crate::protocol::WorkloadSpec) -> Result<(), String> {
        let reach = Self::reach(spec.network == crate::protocol::NetworkMode::None, spec.egress.is_some());
        let asked = Self::asked_of(reach, spec.egress_addresses.as_ref()).map_err(|problem| problem.to_string())?;
        let addresses = host::addresses().map_err(|problem| problem.to_string())?;
        self.compile(&asked, &addresses).map(drop).map_err(|problem| problem.to_string())
    }

    /// The rules for what a sandbox asked, on this host with `host` its own addresses.
    fn compile(
        &self,
        asked: &Asked,
        host: &host::Addresses,
    ) -> Result<BTreeMap<ipnet::Ipv6Net, object::Rule>, RuntimeError> {
        let private: Vec<ipnet::IpNet> =
            ranges(crate::ranges::PRIVATE_V4).chain(ranges(crate::ranges::PRIVATE_V6)).collect();
        let link_local: Vec<ipnet::IpNet> =
            ranges(crate::ranges::LINK_LOCAL_V4).chain(std::iter::once(LINK_LOCAL_V6)).collect();
        rules::compile(&rules::Inputs {
            reach: asked.reach,
            allow: &asked.allow,
            deny: &asked.deny,
            host: &host.own,
            broadcast: &host.broadcast,
            nameservers: &crate::runtime::egress::dns::nameservers(),
            served: self.served.ports.clone(),
            api_port: self.served.api_port,
            private: &private,
            link_local: &link_local,
            metadata: &[ipnet::IpNet::from(metadata_v6())],
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
            self.release(&spec.name)?;
        }
        prepared.map(|record| (record, cgroup))
    }

    async fn attach(&self, spec: &ContainerSpec, cgroup: &Path) -> Result<Record, RuntimeError> {
        let key = self.key(&spec.name);
        let asked = Self::asked(spec)?;
        let addresses = host::addresses().map_err(error)?;
        let rules = self.compile(&asked, &addresses)?;
        let gateways = match spec.egress.is_some() {
            true => Some(Gateways::bind().await?),
            false => None,
        };
        let config = Self::config(spec, gateways.as_ref())?;
        let mut programs =
            Programs::attach(object::OBJECT, &self.host.pin_directory(&key), cgroup, &rules, config).map_err(error)?;
        let guard = match (gateways, &spec.egress) {
            (Some(gateways), Some(policy)) => {
                match gateways.admit(&mut programs).and_then(|()| gateways.serve(policy.clone(), &asked.deny)) {
                    Ok(guard) => Some(guard),
                    Err(problem) => {
                        let _ = programs.detach();
                        return Err(problem);
                    }
                }
            }
            _ => None,
        };
        let record = Record { key, attachments: programs.attachments.clone(), object_sha256: object::digest() };
        self.hold(&spec.name, Held { programs, asked, config, host: addresses, stale: false, _gateways: guard })?;
        Ok(record)
    }

    /// Keeps a sandbox's programs, then rewrites its rules if the host's addresses changed since
    /// they were compiled: a change the watch heard before it was held is caught here.
    fn hold(&self, name: &str, held: Held) -> Result<(), RuntimeError> {
        self.held.lock().unwrap().insert(name.to_owned(), held);
        self.refresh();
        if self.verified(name) {
            Ok(())
        } else {
            Err(error("the sandbox's policy could not be verified before launch"))
        }
    }

    /// Joins and starts under the same lock invalidation takes: a failed refresh either refuses
    /// the start, or fences a cgroup the child has already joined, never an empty launch gap.
    pub(super) fn spawn_verified(
        &self,
        name: &str,
        command: &mut std::process::Command,
    ) -> Result<std::process::Child, RuntimeError> {
        let held = self.held.lock().unwrap();
        if !held.get(name).is_some_and(|held| !held.stale) {
            return Err(error("the sandbox's policy is no longer verified; start it again with new programs"));
        }
        host::join_on_exec(command, &self.host.cgroup(&self.key(name))).map_err(error)?;
        command.spawn().map_err(error)
    }

    fn invalidate(&self, name: &str, held: &mut Held) {
        held.stale = true;
        if let Err(problem) = host::fence_cgroup(&self.host.cgroup(&self.key(name))) {
            tracing::error!(sandbox = name, %problem, "unverified process sandbox's cgroup could not be fenced");
        }
    }

    /// Compiles again, for this host's addresses as they are now, the rules of every sandbox
    /// compiled for others, and writes them; a lookup never misses in between (`Programs::write`).
    /// Returns how many were written.
    pub(super) fn refresh(&self) -> usize {
        self.refresh_with(host::addresses())
    }

    fn refresh_with(&self, addresses: std::io::Result<host::Addresses>) -> usize {
        let addresses = match addresses {
            Ok(addresses) => addresses,
            Err(problem) => {
                for (name, held) in self.held.lock().unwrap().iter_mut() {
                    self.invalidate(name, held);
                }
                tracing::warn!(%problem, "kernel network policy no longer verified: host address tables unreadable");
                return 0;
            }
        };
        let mut written = 0;
        for (name, held) in
            self.held.lock().unwrap().iter_mut().filter(|(_, held)| !held.stale && held.host != addresses)
        {
            let rules = self.compile(&held.asked, &addresses);
            match rules.and_then(|rules| held.programs.write(&rules, held.config).map_err(error)) {
                Ok(()) => {
                    held.host.clone_from(&addresses);
                    written += 1;
                }
                Err(problem) => {
                    self.invalidate(name, held);
                    tracing::warn!(sandbox = %name, %problem, "kernel network policy no longer verified for this host's addresses");
                }
            }
        }
        written
    }

    /// Keeps every live sandbox's rules for this host's addresses as they change, on a thread of
    /// its own: the kernel wakes it as an address is added or removed, and it compiles and writes
    /// them again at once. Ends at the first change after the network is gone. Should the kernel's
    /// word stop coming, it reads the addresses every second until it can subscribe again.
    pub(super) fn watch(network: &std::sync::Weak<Self>, mut changes: AddressChanges) {
        const AGAIN: std::time::Duration = std::time::Duration::from_secs(1);
        loop {
            if let Err(problem) = changes.changed() {
                tracing::warn!(%problem, "this host's address changes went unheard; reading them every second");
                std::thread::sleep(AGAIN);
                if let Ok(again) = host::subscribe() {
                    changes = again;
                }
            }
            let Some(network) = network.upgrade() else { return };
            let woken = std::time::Instant::now();
            let written = network.refresh();
            if written > 0 {
                let micros = woken.elapsed().as_micros();
                tracing::info!(written, micros, "this host's addresses changed; sandboxes' rules written again");
            }
        }
    }

    /// After a daemon restart, for a supervisor still running: its programs found by their pins
    /// and checked, its rules written again for today's host, and its gateways served again.
    pub(super) async fn adopt(&self, spec: &ContainerSpec, record: &Record) -> Result<(), RuntimeError> {
        if record.key != self.key(&spec.name) {
            return Err(error("the journal names another sandbox's programs"));
        }
        if record.object_sha256 != object::digest() {
            return Err(error(OBJECT_MISMATCH));
        }
        let mut programs =
            Programs::adopt(&self.host.pin_directory(&record.key), &record.attachments).map_err(error)?;
        let asked = Self::asked(spec)?;
        let addresses = host::addresses().map_err(error)?;
        let rules = self.compile(&asked, &addresses)?;
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
            (Some(gateways), Some(policy)) => Some(gateways.serve(policy.clone(), &asked.deny)?),
            _ => None,
        };
        self.hold(&spec.name, Held { programs, asked, config, host: addresses, stale: false, _gateways: guard })
    }

    /// Once a sandbox's supervisor is gone: whatever is left in its cgroup is killed, then its
    /// programs are detached and the cgroup removed. Idempotent.
    pub(super) fn release(&self, name: &str) -> Result<(), RuntimeError> {
        let key = self.key(name);
        let cgroup = self.host.cgroup(&key);
        let present = match std::fs::symlink_metadata(&cgroup) {
            Ok(metadata) if metadata.is_dir() => true,
            Ok(_) => return Err(error("the sandbox cgroup is not a directory")),
            Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => false,
            Err(problem) => return Err(error(problem)),
        };
        // Failed metadata inspection is not an absence proof and must not drop its programs.
        let held = self.held.lock().unwrap().remove(name);
        if present {
            host::fence_cgroup(&cgroup).map_err(error)?;
            host::wait_empty(&cgroup).map_err(error)?;
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
        self.held.lock().unwrap().get(name).is_some_and(|held| !held.stale)
    }

    /// A lost supervisor or an incomplete journal is not proof its canonical cgroup is empty.
    /// Even an empty cgroup may accept a paused pre-exec child's late join. Only completed
    /// retirement, which removes the canonical directory, proves it cannot run again.
    pub(super) fn pending(&self, name: &str) -> bool {
        !std::fs::symlink_metadata(self.host.cgroup(&self.key(name)))
            .is_err_and(|problem| problem.kind() == std::io::ErrorKind::NotFound)
    }

    /// Canonical fixture identity without exposing network internals to runtime-owned tests.
    #[cfg(test)]
    pub(super) fn fixture_paths(&self, name: &str) -> (String, PathBuf) {
        let key = self.key(name);
        let cgroup = self.host.cgroup(&key);
        (key, cgroup)
    }

    pub(super) fn counts(&self, name: &str) -> Option<Counts> {
        self.held.lock().unwrap().get(name).and_then(|held| held.programs.counts().ok())
    }
}

/// A minimal network fixture specification, shared only by the two test owners.
#[cfg(test)]
pub(super) fn fixture_spec(root: &Path) -> ContainerSpec {
    serde_json::from_value(serde_json::json!({
        "name": "route-guard", "image": "", "env": [], "labels": {}, "user": "",
        "memory_bytes": 0, "cpu_shares": 0, "pids_limit": 0, "read_only_rootfs": true,
        "tmp_size_mb": 0, "data_dir": root, "mount_path": "/data", "ports": [],
        "stop_signal": "SIGTERM", "stop_timeout_secs": 0, "network": "",
        "internal_network": false, "log_max_size_mb": 0, "log_max_files": 0, "oom_score_adj": 0
    }))
    .unwrap()
}

#[cfg(test)]
#[path = "network/guard_tests.rs"]
mod guard_tests;

#[cfg(test)]
#[path = "network/path_tests.rs"]
mod path_tests;

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

    #[tokio::test]
    async fn legacy_and_changed_object_records_are_refused_before_opening_pins() {
        let root = tempfile::tempdir().unwrap();
        let network = Network {
            host: Host { cgroups: root.path().join("cgroups"), pins: root.path().join("pins"), delegated: false },
            served: Served { ports: 42000..=42999, api_port: 7443 },
            root: root.path().to_path_buf(),
            held: Mutex::new(HashMap::new()),
        };
        let spec: ContainerSpec = serde_json::from_value(serde_json::json!({
            "name": "old-object", "image": "", "env": [], "labels": {}, "user": "",
            "memory_bytes": 0, "cpu_shares": 0, "pids_limit": 0, "read_only_rootfs": true,
            "tmp_size_mb": 0, "data_dir": root.path(), "mount_path": "/data", "ports": [],
            "stop_signal": "SIGTERM", "stop_timeout_secs": 0, "network": "",
            "internal_network": false, "log_max_size_mb": 0, "log_max_files": 0, "oom_score_adj": 0
        }))
        .unwrap();
        for identity in [None, Some("another-object")] {
            let mut stored = serde_json::json!({ "key": network.key(&spec.name), "attachments": [] });
            if let Some(identity) = identity {
                stored["object_sha256"] = identity.into();
            }
            let record: Record = serde_json::from_value(stored).unwrap();
            let refused = network.adopt(&spec, &record).await.unwrap_err();
            assert!(refused.to_string().contains(OBJECT_MISMATCH), "{refused}");
            assert!(!network.verified(&spec.name));
            assert!(!network.host.pins.exists(), "no legacy pins were admitted or changed");
        }
    }

    #[test]
    fn rules_that_fold_into_more_ranges_than_a_prefix_holds_are_refused_before_a_start() {
        let network = Network {
            host: Host { cgroups: "/sys/fs/cgroup/x".into(), pins: "/sys/fs/bpf/pryx".into(), delegated: false },
            served: Served { ports: 42000..=42999, api_port: 7443 },
            root: "/var/lib/pryxd/process-runtime".into(),
            held: Mutex::new(HashMap::new()),
        };
        // Each prefix one bit narrower than the last, each with a port range of its own: the
        // narrowest carries all of them.
        let nested = |count: u8| {
            let deny: Vec<_> = (0..count)
                .map(|bits| {
                    let port = 1000 + u16::from(bits);
                    serde_json::json!({ "prefix": format!("203.0.113.0/{}", 8 + bits), "ports": [port, port] })
                })
                .collect();
            serde_json::from_value::<crate::protocol::WorkloadSpec>(serde_json::json!({
                "resources": { "memoryMb": 256 },
                "storage": { "mountPath": "/data", "sizeGb": 1 },
                "egressAddresses": { "deny": deny },
            }))
            .unwrap()
        };
        assert_eq!(network.check(&nested(4)), Ok(()));
        let refused = network.check(&nested(object::PORT_ENTRIES as u8 + 1)).unwrap_err();
        assert!(refused.contains("203.0.113.0/32") && refused.contains("port ranges"), "{refused}");
    }

    #[tokio::test]
    #[ignore = "loads programs into the kernel"]
    async fn a_failed_address_refresh_stops_members_and_refuses_late_starts_until_new_programs() {
        use std::os::unix::process::ExitStatusExt;
        let root = tempfile::tempdir().unwrap();
        let host = Host::find().unwrap_or_else(|problem| panic!("{}", problem.evidence));
        let served = Served { ports: 42000..=42999, api_port: 7443 };
        let network = Network::new(host, served.clone(), root.path());
        let spec: ContainerSpec = serde_json::from_value(serde_json::json!({
            "name": "refresh", "image": "", "env": [], "labels": {}, "user": "",
            "memory_bytes": 0, "cpu_shares": 0, "pids_limit": 0, "read_only_rootfs": true,
            "tmp_size_mb": 0, "data_dir": root.path(), "mount_path": "/data", "ports": [],
            "stop_signal": "SIGTERM", "stop_timeout_secs": 0, "network": "",
            "internal_network": false, "log_max_size_mb": 0, "log_max_files": 0, "oom_score_adj": 0
        }))
        .unwrap();
        let sleeper = || {
            let mut command = std::process::Command::new("sleep");
            command.arg("60");
            host::drop_ambient_on_exec(&mut command);
            command
        };
        let (original, cgroup) = network.prepare(&spec).await.unwrap();
        let mut child = network.spawn_verified(&spec.name, &mut sleeper()).unwrap();
        assert!(host::members(&cgroup).unwrap().contains(&child.id()));
        let unreadable = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "address table denied");
        assert_eq!(network.refresh_with(Err(unreadable)), 0);
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        assert!(host::members(&cgroup).unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(cgroup.join("cgroup.freeze")).unwrap().trim(), "1");
        assert!(!network.verified(&spec.name));
        assert!(network.spawn_verified(&spec.name, &mut sleeper()).is_err(), "no join after an empty cgroup's kill");
        // Readability returning does not revive that fenced identity or its old programs.
        network.refresh_with(host::addresses());
        assert!(!network.verified(&spec.name));
        assert!(network.spawn_verified(&spec.name, &mut sleeper()).is_err());
        let (fresh, fresh_cgroup) = network.prepare(&spec).await.unwrap();
        assert_ne!(fresh.attachments, original.attachments, "a restart attaches a new program set");
        assert_eq!(cgroup, fresh_cgroup);
        assert_eq!(std::fs::read_to_string(cgroup.join("cgroup.freeze")).unwrap().trim(), "0");
        let mut restarted = network.spawn_verified(&spec.name, &mut sleeper()).unwrap();
        assert!(host::members(&cgroup).unwrap().contains(&restarted.id()));
        network.release(&spec.name).unwrap();
        assert_eq!(restarted.wait().unwrap().signal(), Some(libc::SIGKILL));
        assert!(!cgroup.exists());
        assert!(!network.host.pin_directory(&network.key(&spec.name)).exists());
    }
}
