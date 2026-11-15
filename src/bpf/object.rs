// SPDX-License-Identifier: FSL-1.1-ALv2
//! The compiled programs pryxd embeds (`bpf/network.bpf.o`, from `bpf/network.bpf.c` by
//! `bpf/build.sh`) and the bytes its maps hold, as `bpf/network.h` lays them out. This is the
//! skeleton: every program and map is named here once, and a test holds the object to it.
//! Encoding only; what a rule should say is `rules.rs`, and the kernel calls are `attach.rs`.

/// The object, aligned as its ELF reader needs.
pub(crate) const OBJECT: &[u8] = aya::include_bytes_aligned!("../../bpf/network.bpf.o");

/// Each program, in the order they are attached: the decisions first, the counters last.
pub(crate) const PROGRAMS: [&str; 9] =
    ["sock_create", "connect4", "connect6", "sendmsg4", "sendmsg6", "recvmsg4", "recvmsg6", "ingress", "egress"];

pub(crate) const RULES: &str = "rules";
pub(crate) const CONFIG: &str = "config";
pub(crate) const SOCKETS: &str = "sockets";
pub(crate) const COUNTERS: &str = "counters";
pub(crate) const MAPS: [&str; 4] = [RULES, CONFIG, SOCKETS, COUNTERS];

/// `PRYX_PORT_ENTRIES` and `PRYX_RULES`.
pub(crate) const PORT_ENTRIES: usize = 24;
pub(crate) const MAX_RULES: usize = 1024;
/// `sizeof(struct pryx_ports)`, `struct pryx_rule`, `struct pryx_config`, `struct pryx_socket`.
const PORTS_BYTES: usize = 8;
pub(crate) const RULE_BYTES: usize = 8 + PORT_ENTRIES * PORTS_BYTES;
pub(crate) const CONFIG_BYTES: usize = 12;
pub(crate) const SOCKET_BYTES: usize = 20;
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
        for (slot, ports) in bytes[8..].chunks_exact_mut(PORTS_BYTES).zip(&self.ports) {
            slot[0..2].copy_from_slice(&ports.low.to_ne_bytes());
            slot[2..4].copy_from_slice(&ports.high.to_ne_bytes());
            slot[4] = ports.protocols;
            slot[5] = ports.verdict as u8;
        }
        Some(bytes)
    }
}

/// `struct pryx_config`: the gateway's IPv4 address and its HTTP, TLS and DNS ports.
pub(crate) fn config_bytes(gateway: std::net::Ipv4Addr, http: u16, tls: u16, dns: u16) -> [u8; CONFIG_BYTES] {
    let mut bytes = [0; CONFIG_BYTES];
    bytes[0..4].copy_from_slice(&gateway.octets());
    bytes[4..6].copy_from_slice(&http.to_be_bytes());
    bytes[6..8].copy_from_slice(&tls.to_be_bytes());
    bytes[8..10].copy_from_slice(&dns.to_be_bytes());
    bytes
}

/// `struct pryx_socket` for a socket pryxd serves this sandbox.
pub(crate) fn gateway_socket() -> [u8; SOCKET_BYTES] {
    let mut bytes = [0; SOCKET_BYTES];
    bytes[0] = ROLE_GATEWAY;
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use aya_obj::{Object, maps::Map};

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
        assert_eq!(object.maps[SOCKETS].value_size(), SOCKET_BYTES as u32);
        assert_eq!(sizes(&object.maps[COUNTERS]), (4, std::mem::size_of::<Counter>() as u32, 4));
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
