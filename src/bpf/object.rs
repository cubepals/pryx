// SPDX-License-Identifier: FSL-1.1-ALv2
//! The compiled programs pryxd embeds (`bpf/network.bpf.o`, from `bpf/network.bpf.c` by
//! `bpf/build.sh`) and the bytes its maps hold, as `bpf/network.h` lays them out. This is the
//! skeleton: every program and map is named here once, and a test holds the object to it.
//! Encoding only; what a rule should say is `rules.rs`, and the kernel calls are `attach.rs`.

/// The object, aligned as its ELF reader needs.
pub(crate) const OBJECT: &[u8] = aya::include_bytes_aligned!("../../bpf/network.bpf.o");

/// Identity of the complete object, including policy instructions, not merely its map ABI.
pub(crate) fn digest() -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(OBJECT))
}

/// Each program, in the order they are attached: the decisions first, the counters last.
pub(crate) const PROGRAMS: [&str; 9] =
    ["sock_create", "connect4", "connect6", "sendmsg4", "sendmsg6", "recvmsg4", "recvmsg6", "ingress", "egress"];

pub(crate) const RULES: &str = "rules";
pub(crate) const CONFIG: &str = "config";
pub(crate) const OWNERS: &str = "owners";
pub(crate) const SOCKETS: &str = "sockets";
pub(crate) const PEERS: &str = "peers";
pub(crate) const COUNTERS: &str = "counters";
/// Every map, in the order they are pinned and opened again.
pub(crate) const MAPS: [&str; 6] = [RULES, CONFIG, OWNERS, SOCKETS, PEERS, COUNTERS];

/// `PRYX_PORT_ENTRIES` and `PRYX_RULES`.
pub(crate) const PORT_ENTRIES: usize = 24;
pub(crate) const MAX_RULES: usize = 1024;
/// `PRYX_PUBLISHED_PORTS`: a request's own ports, its agent's and every spare it may hold.
pub(crate) const PUBLISHED_PORTS: usize =
    crate::protocol::MAX_PORTS + pryx_agent::contract::MAX_SPARE_PORTS as usize + 1;
/// `sizeof(struct pryx_ports)`, `struct pryx_rule`, `struct pryx_config`, `struct pryx_socket`.
const PORTS_BYTES: usize = 8;
pub(crate) const RULE_BYTES: usize = 8 + PORT_ENTRIES * PORTS_BYTES;
pub(crate) const CONFIG_BYTES: usize = 12 + PUBLISHED_PORTS * 2 + 2;
pub(crate) const SOCKET_BYTES: usize = 24;
/// `struct pryx_counter`: packets, then bytes.
pub(crate) type Counter = [u64; 2];

/// `enum pryx_verdict`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Deny = 0,
    Allow = 1,
    Owned = 2,
    Http = 3,
    Tls = 4,
    Dns = 5,
    Resolver = 6,
}

/// `PRYX_TCP` and `PRYX_UDP`.
pub(crate) const TCP: u8 = 1;
pub(crate) const UDP: u8 = 2;

/// `PRYX_ROLE_GATEWAY`: a listener pryxd serves for one sandbox.
pub(crate) const ROLE_GATEWAY: u8 = 2;

/// `enum pryx_counter_index`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CounterIndex {
    Ingress = 0,
    Egress = 1,
    RefusedAddresses = 2,
    RefusedSockets = 3,
    RefusedIngress = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ports {
    pub(crate) low: u16,
    pub(crate) high: u16,
    pub(crate) protocols: u8,
    pub(crate) verdict: Verdict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rule {
    pub(crate) fallback: Verdict,
    pub(crate) ports: Vec<Ports>,
}

impl Rule {
    /// `struct pryx_rule`, in this host's byte order; None past `PORT_ENTRIES`.
    pub(crate) fn bytes(&self) -> Option<[u8; RULE_BYTES]> {
        if self.ports.len() > PORT_ENTRIES {
            return None;
        }
        let mut bytes = [0; RULE_BYTES];
        bytes[0] = self.fallback as u8;
        bytes[1] = self.ports.len() as u8;
        for (index, ports) in self.ports.iter().enumerate() {
            let slot = &mut bytes[8 + index * PORTS_BYTES..][..PORTS_BYTES];
            slot[0..2].copy_from_slice(&ports.low.to_ne_bytes());
            slot[2..4].copy_from_slice(&ports.high.to_ne_bytes());
            slot[4] = ports.protocols;
            slot[5] = ports.verdict as u8;
        }
        Some(bytes)
    }
}

/// `struct pryx_config`: the gateway's IPv4 address and its HTTP, TLS and DNS ports, and the
/// ports published for the sandbox; None past `PUBLISHED_PORTS`.
pub(crate) fn config_bytes(
    gateway: std::net::Ipv4Addr,
    http: u16,
    tls: u16,
    dns: u16,
    published: &[u16],
) -> Option<[u8; CONFIG_BYTES]> {
    if published.len() > PUBLISHED_PORTS {
        return None;
    }
    let mut bytes = [0; CONFIG_BYTES];
    bytes[0..4].copy_from_slice(&gateway.octets());
    bytes[4..6].copy_from_slice(&http.to_be_bytes());
    bytes[6..8].copy_from_slice(&tls.to_be_bytes());
    bytes[8..10].copy_from_slice(&dns.to_be_bytes());
    bytes[10..12].copy_from_slice(&(published.len() as u16).to_ne_bytes());
    for (index, port) in published.iter().enumerate() {
        bytes[12 + 2 * index..][..2].copy_from_slice(&port.to_ne_bytes());
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aya_obj::{
        Object,
        generated::{BPF_F_CLONE, BPF_F_NO_PREALLOC},
        maps::Map,
    };

    /// `PRYX_COUNTERS`: one slot per `CounterIndex`.
    const COUNTER_SLOTS: u32 = CounterIndex::RefusedIngress as u32 + 1;
    /// `PRYX_PEERS`, and `sizeof(struct pryx_peer)`: a socket's cookie, a peer's address and port.
    /// Only the programs read the map, so nothing else names them.
    const MAX_PEERS: u32 = 4096;
    const PEER_BYTES: u32 = 32;

    fn sizes(map: &Map) -> (u32, u32, u32) {
        (map.key_size(), map.value_size(), map.max_entries())
    }

    #[test]
    fn the_object_has_every_program_and_map_this_names() {
        let object = Object::parse(OBJECT).expect("the embedded object parses");
        let mut programs: Vec<&str> = object.programs.keys().map(String::as_str).collect();
        let mut named = PROGRAMS.to_vec();
        programs.sort_unstable();
        named.sort_unstable();
        assert_eq!(programs, named);
        let mut maps: Vec<&str> = object.maps.keys().map(String::as_str).collect();
        let mut named = MAPS.to_vec();
        maps.sort_unstable();
        named.sort_unstable();
        assert_eq!(maps, named);
        assert_eq!(object.license.to_str().unwrap(), "Dual BSD/GPL");
        assert_eq!(sizes(&object.maps[RULES]), (4 + 16, RULE_BYTES as u32, MAX_RULES as u32));
        assert_eq!(sizes(&object.maps[CONFIG]), (4, CONFIG_BYTES as u32, 1));
        assert_eq!(object.maps[OWNERS].value_size(), std::mem::size_of::<u8>() as u32);
        assert_eq!(object.maps[OWNERS].map_flags(), BPF_F_NO_PREALLOC | BPF_F_CLONE);
        assert_eq!(object.maps[SOCKETS].value_size(), SOCKET_BYTES as u32);
        assert_eq!(object.maps[SOCKETS].map_flags(), BPF_F_NO_PREALLOC);
        assert_eq!(sizes(&object.maps[PEERS]), (PEER_BYTES, 1, MAX_PEERS));
        assert_eq!(sizes(&object.maps[COUNTERS]), (4, std::mem::size_of::<Counter>() as u32, COUNTER_SLOTS));
    }

    #[test]
    fn the_rejected_object_differs_only_in_its_last_program() {
        let rejected: &[u8] = aya::include_bytes_aligned!("../../bpf/rejected.bpf.o");
        let (ours, theirs) = (Object::parse(OBJECT).unwrap(), Object::parse(rejected).unwrap());
        let names = |object: &Object| object.programs.keys().cloned().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names(&ours), names(&theirs));
        assert_eq!(PROGRAMS.last(), Some(&"egress"), "the verifier refuses the set's last program");
    }

    #[test]
    fn the_configuration_holds_every_published_port_and_refuses_more() {
        let ports: Vec<u16> = (1..=PUBLISHED_PORTS as u16).collect();
        let bytes = config_bytes(std::net::Ipv4Addr::LOCALHOST, 1, 2, 3, &ports).unwrap();
        assert_eq!(&bytes[10..12], &(PUBLISHED_PORTS as u16).to_ne_bytes());
        assert_eq!(&bytes[12..14], &1u16.to_ne_bytes());
        assert_eq!(&bytes[CONFIG_BYTES - 4..CONFIG_BYTES - 2], &(PUBLISHED_PORTS as u16).to_ne_bytes());
        let too_many: Vec<u16> = (0..=PUBLISHED_PORTS as u16).collect();
        assert!(config_bytes(std::net::Ipv4Addr::LOCALHOST, 0, 0, 0, &too_many).is_none());
    }

    #[test]
    fn a_rule_encodes_its_ports_in_order_and_refuses_too_many() {
        let ports = Ports { low: 80, high: 443, protocols: TCP | UDP, verdict: Verdict::Allow };
        let bytes = Rule { fallback: Verdict::Owned, ports: vec![ports] }.bytes().unwrap();
        assert_eq!(&bytes[..2], &[Verdict::Owned as u8, 1]);
        assert_eq!(&bytes[8..14], &[80u16.to_ne_bytes(), 443u16.to_ne_bytes(), [3, 1]].concat()[..]);
        assert!(Rule { fallback: Verdict::Deny, ports: vec![ports; PORT_ENTRIES + 1] }.bytes().is_none());
    }
}
