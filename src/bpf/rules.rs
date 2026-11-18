// SPDX-License-Identifier: FSL-1.1-ALv2
//! A process sandbox's network policy as the prefix rules its programs read. Every decision is
//! made here: the programs look up the longest matching prefix and take the first port range
//! that matches, so each prefix carries its own ranges and those of every wider prefix, in the
//! order they apply. The host's protections come first and a request's rules can't relax them:
//! the host's own addresses reach only the sandbox's own sockets, link-local, multicast and
//! this-host aliases are refused, and private addresses never on the ports pryxd serves.
//! Pure: the host's addresses, resolvers and ranges are the caller's; the kernel is `attach.rs`.
use super::object::{MAX_RULES, PORT_ENTRIES, Ports, Rule, TCP, UDP, Verdict};
use ipnet::{IpNet, Ipv6Net};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv6Addr},
    ops::RangeInclusive,
};

/// What the sandbox may reach before its own rules: nothing, the public internet, or only the
/// hostnames its gateway admits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    None,
    Public,
    Hostnames,
}

/// One address rule: a prefix and, optionally, an inclusive port range (TCP and UDP).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Destination {
    pub(crate) prefix: IpNet,
    pub(crate) ports: Option<(u16, u16)>,
}

pub(crate) struct Inputs<'a> {
    pub(crate) reach: Reach,
    pub(crate) allow: &'a [Destination],
    pub(crate) deny: &'a [Destination],
    /// This host's own addresses.
    pub(crate) host: &'a [IpAddr],
    /// The resolvers /etc/resolv.conf names, reached on DNS's port when the internet is public.
    pub(crate) nameservers: &'a [IpAddr],
    /// The ports pryxd hands out, and its API's.
    pub(crate) served: RangeInclusive<u16>,
    pub(crate) api_port: u16,
    /// Private ranges, where pryxd's served ports belong to other nodes, link-local ones, where
    /// clouds serve metadata, and the metadata addresses outside them: the host firewall's own
    /// (`ranges.rs`).
    pub(crate) private: &'a [IpNet],
    pub(crate) link_local: &'a [IpNet],
    pub(crate) metadata: &'a [IpNet],
    /// DNS's port.
    pub(crate) dns: u16,
}

/// IPv4-mapped IPv6 (RFC 4291 §2.5.5.2) and the NAT64 well-known prefix (RFC 6052 §2.1).
const MAPPED: Ipv6Addr = Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0, 0);
const NAT64: Ipv6Addr = Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, 0, 0);
/// HTTP's and TLS's ports (RFC 9110 §4.2).
const HTTP: u16 = 80;
const TLS: u16 = 443;

/// Highest first: a request's deny, DNS, the host, the host's served ports, a request's allow,
/// the gateways, then what `Reach` gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Precedence {
    Base,
    Gateway,
    Allowed,
    Served,
    Host,
    Resolver,
    Denied,
}

#[derive(Clone, Copy)]
struct Layer {
    prefix: Ipv6Net,
    ports: Option<(u16, u16)>,
    protocols: u8,
    verdict: Verdict,
    precedence: Precedence,
}

/// An address as the programs key it: IPv6, with IPv4 mapped.
pub(crate) fn mapped(prefix: IpNet) -> Ipv6Net {
    match prefix {
        IpNet::V4(net) => {
            let octets = net.network().octets();
            let address = Ipv6Addr::from(u128::from(MAPPED) | u128::from(u32::from_be_bytes(octets)));
            Ipv6Net::new(address, 96 + net.prefix_len()).expect("an IPv4 prefix fits a mapped one")
        }
        IpNet::V6(net) => net.trunc(),
    }
}

fn host(address: IpAddr) -> IpNet {
    IpNet::from(address)
}

/// Every prefix the programs need and its rule. An error names what can't be expressed.
pub(crate) fn compile(inputs: &Inputs<'_>) -> Result<BTreeMap<Ipv6Net, Rule>, String> {
    let mut layers = layers(inputs);
    // NAT64 reaches the IPv4 address it embeds, so it takes that address's rules.
    let translated: Vec<Layer> = layers.iter().filter_map(translated).collect();
    layers.extend(translated);
    let mut rules = BTreeMap::new();
    for prefix in layers.iter().map(|layer| layer.prefix) {
        if let std::collections::btree_map::Entry::Vacant(slot) = rules.entry(prefix) {
            slot.insert(fold(&layers, prefix)?);
        }
    }
    if rules.len() > MAX_RULES {
        return Err(format!("the policy needs {} prefixes; at most {MAX_RULES} fit", rules.len()));
    }
    Ok(rules)
}

/// What each part of the policy says, before folding: base reach, the host's protections, the
/// resolvers or gateways, then a request's rules.
fn layers(inputs: &Inputs<'_>) -> Vec<Layer> {
    let both = TCP | UDP;
    let mut layers = Vec::new();
    let mut add = |prefix: IpNet, ports, protocols, verdict, precedence| {
        layers.push(Layer { prefix: mapped(prefix), ports, protocols, verdict, precedence });
    };
    let public = if inputs.reach == Reach::Public { Verdict::Allow } else { Verdict::Deny };
    for base in ["::/0", "::ffff:0:0/96"] {
        add(base.parse().unwrap(), None, both, public, Precedence::Base);
    }
    for private in inputs.private.iter().copied() {
        add(private, None, both, public, Precedence::Base);
        add(private, Some((*inputs.served.start(), *inputs.served.end())), both, Verdict::Deny, Precedence::Served);
        add(private, Some((inputs.api_port, inputs.api_port)), both, Verdict::Deny, Precedence::Served);
    }
    let refused = ["0.0.0.0/8", "224.0.0.0/3", "::/96", "ff00::/8"].into_iter().map(|net| net.parse().unwrap());
    for refused in refused.chain(inputs.link_local.iter().copied()).chain(inputs.metadata.iter().copied()) {
        add(refused, None, both, Verdict::Deny, Precedence::Host);
    }
    let own = ["127.0.0.0/8".parse().unwrap(), host(Ipv6Addr::LOCALHOST.into())];
    for own in own.into_iter().chain(inputs.host.iter().copied().map(host)) {
        add(own, None, both, Verdict::Owned, Precedence::Host);
    }
    let dns = Some((inputs.dns, inputs.dns));
    match inputs.reach {
        Reach::Hostnames => {
            add("::/0".parse().unwrap(), dns, both, Verdict::Dns, Precedence::Resolver);
            add("::/0".parse().unwrap(), Some((HTTP, HTTP)), TCP, Verdict::Http, Precedence::Gateway);
            add("::/0".parse().unwrap(), Some((TLS, TLS)), TCP, Verdict::Tls, Precedence::Gateway);
        }
        Reach::Public => {
            for nameserver in inputs.nameservers {
                add(host(*nameserver), dns, both, Verdict::Resolver, Precedence::Resolver);
            }
        }
        Reach::None => {}
    }
    for rule in inputs.allow {
        add(rule.prefix, rule.ports, both, Verdict::Allow, Precedence::Allowed);
    }
    for rule in inputs.deny {
        add(rule.prefix, rule.ports, both, Verdict::Deny, Precedence::Denied);
    }
    layers
}

/// The NAT64 twin of a layer about an IPv4 prefix.
fn translated(layer: &Layer) -> Option<Layer> {
    let mapped = Ipv6Net::new(MAPPED, 96).unwrap();
    if layer.prefix.prefix_len() < 96 || !mapped.contains(&layer.prefix.network()) {
        return None;
    }
    let embedded = u128::from(layer.prefix.network()) & u128::from(u32::MAX);
    let prefix = Ipv6Net::new(Ipv6Addr::from(u128::from(NAT64) | embedded), layer.prefix.prefix_len()).unwrap();
    Some(Layer { prefix, ..*layer })
}

/// One prefix's rule: the port ranges of every layer covering it, highest precedence and then
/// narrowest first, up to the first that applies to every port.
fn fold(layers: &[Layer], prefix: Ipv6Net) -> Result<Rule, String> {
    let mut covering: Vec<&Layer> = layers.iter().filter(|layer| layer.prefix.contains(&prefix)).collect();
    covering.sort_by(|a, b| b.precedence.cmp(&a.precedence).then(b.prefix.prefix_len().cmp(&a.prefix.prefix_len())));
    let mut rule = Rule { fallback: Verdict::Deny, ports: Vec::new() };
    for layer in covering {
        let Some((low, high)) = layer.ports else {
            rule.fallback = layer.verdict;
            break;
        };
        rule.ports.push(Ports { low, high, protocols: layer.protocols, verdict: layer.verdict });
    }
    if rule.ports.len() > PORT_ENTRIES {
        return Err(format!(
            "{} would need {} port ranges; at most {PORT_ENTRIES} can apply to one address",
            shown(prefix),
            rule.ports.len()
        ));
    }
    Ok(rule)
}

/// A prefix as a request names it: an IPv4-mapped one as IPv4.
fn shown(prefix: Ipv6Net) -> String {
    let mapped = Ipv6Net::new(MAPPED, 96).expect("a valid prefix");
    match prefix.network().to_ipv4_mapped() {
        Some(address) if mapped.contains(&prefix) => format!("{address}/{}", prefix.prefix_len() - 96),
        _ => prefix.to_string(),
    }
}

/// What the programs decide for one address and port, read from compiled rules as they do.
#[cfg(test)]
pub(crate) fn decide(rules: &BTreeMap<Ipv6Net, Rule>, address: IpAddr, port: u16, protocol: u8) -> Verdict {
    let key = mapped(host(address)).network();
    let rule = rules
        .iter()
        .filter(|(prefix, _)| prefix.contains(&key))
        .max_by_key(|(prefix, _)| prefix.prefix_len())
        .map(|(_, rule)| rule)
        .expect("::/0 is always written");
    rule.ports
        .iter()
        .find(|ports| ports.protocols & protocol != 0 && (ports.low..=ports.high).contains(&port))
        .map_or(rule.fallback, |ports| ports.verdict)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    const HOST: [IpAddr; 2] =
        [IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)), IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9))];
    const RESOLVER: [IpAddr; 1] = [IpAddr::V4(Ipv4Addr::new(127, 0, 0, 53))];

    fn nets(list: &[&str]) -> &'static [IpNet] {
        list.iter().map(|net| net.parse().unwrap()).collect::<Vec<_>>().leak()
    }

    fn inputs<'a>(reach: Reach, allow: &'a [Destination], deny: &'a [Destination]) -> Inputs<'a> {
        Inputs {
            reach,
            allow,
            deny,
            host: &HOST,
            nameservers: &RESOLVER,
            served: 42000..=42999,
            api_port: 7443,
            private: nets(&["10.0.0.0/8", "100.64.0.0/10", "172.16.0.0/12", "192.168.0.0/16", "fc00::/7"]),
            link_local: nets(&["169.254.0.0/16", "fe80::/10"]),
            metadata: vec![IpNet::from(crate::ranges::METADATA_V6.parse::<IpAddr>().unwrap())].leak(),
            dns: 53,
        }
    }
    fn at(prefix: &str, ports: Option<(u16, u16)>) -> Destination {
        Destination { prefix: prefix.parse().unwrap(), ports }
    }
    fn decide_in(inputs: &Inputs<'_>, address: &str, port: u16, protocol: u8) -> Verdict {
        decide(&compile(inputs).unwrap(), address.parse().unwrap(), port, protocol)
    }

    #[test]
    fn public_reach_keeps_the_host_its_neighbours_and_metadata_off() {
        let policy = inputs(Reach::Public, &[], &[]);
        for (address, port, expected) in [
            ("1.1.1.1", 443, Verdict::Allow),
            ("2606:4700::1111", 443, Verdict::Allow),
            ("10.1.2.3", 5432, Verdict::Allow),
            ("10.1.2.3", 42001, Verdict::Deny),
            ("10.1.2.3", 7443, Verdict::Deny),
            ("fd00::1", 42999, Verdict::Deny),
            ("169.254.169.254", 80, Verdict::Deny),
            ("fe80::1", 80, Verdict::Deny),
            ("fd00:ec2::254", 80, Verdict::Deny),
            ("fd00:ec2::254", 5432, Verdict::Deny),
            ("fd00:ec2::253", 53, Verdict::Allow),
            ("fd00:ec2::1", 5432, Verdict::Allow),
            ("224.0.0.251", 5353, Verdict::Deny),
            ("0.0.0.0", 8080, Verdict::Deny),
            ("127.0.0.1", 8080, Verdict::Owned),
            ("127.0.0.53", 53, Verdict::Resolver),
            ("127.0.0.53", 54, Verdict::Owned),
            ("::1", 8080, Verdict::Owned),
            ("::ffff:127.0.0.1", 8080, Verdict::Owned),
            ("::ffff:169.254.169.254", 80, Verdict::Deny),
            ("64:ff9b::a9fe:a9fe", 80, Verdict::Deny),
            ("64:ff9b::a01:203", 42001, Verdict::Deny),
            ("64:ff9b::101:101", 443, Verdict::Allow),
            ("203.0.113.9", 42001, Verdict::Owned),
            ("2001:db8::9", 22, Verdict::Owned),
        ] {
            assert_eq!(decide_in(&policy, address, port, TCP), expected, "{address}:{port}");
        }
    }

    #[test]
    fn resolver_authority_is_exact_and_a_requested_deny_still_wins() {
        let deny = [at("127.0.0.53/32", Some((53, 53)))];
        for protocol in [TCP, UDP] {
            let public = inputs(Reach::Public, &[], &[]);
            assert_eq!(decide_in(&public, "127.0.0.53", 53, protocol), Verdict::Resolver);
            assert_eq!(decide_in(&public, "127.0.0.54", 53, protocol), Verdict::Owned);
            assert_eq!(decide_in(&public, "127.0.0.53", 54, protocol), Verdict::Owned);
            let refused = inputs(Reach::Public, &[], &deny);
            assert_eq!(decide_in(&refused, "127.0.0.53", 53, protocol), Verdict::Deny);
        }
    }

    #[test]
    fn without_public_reach_only_allowed_destinations_and_own_sockets_remain() {
        let allow = [at("172.17.0.2/32", Some((8000, 8000)))];
        let policy = inputs(Reach::None, &allow, &[]);
        for (address, port, expected) in [
            ("1.1.1.1", 443, Verdict::Deny),
            ("10.1.2.3", 5432, Verdict::Deny),
            ("172.17.0.2", 8000, Verdict::Allow),
            ("172.17.0.2", 8001, Verdict::Deny),
            ("172.17.0.3", 8000, Verdict::Deny),
            ("127.0.0.53", 53, Verdict::Owned),
            ("127.0.0.1", 9000, Verdict::Owned),
        ] {
            assert_eq!(decide_in(&policy, address, port, UDP), expected, "{address}:{port}");
        }
    }

    #[test]
    fn hostname_reach_sends_web_and_dns_to_the_gateway_and_nothing_else_out() {
        let allow = [at("198.51.100.7/32", Some((443, 443)))];
        let policy = inputs(Reach::Hostnames, &allow, &[]);
        for (address, port, protocol, expected) in [
            ("1.1.1.1", 443, TCP, Verdict::Tls),
            ("1.1.1.1", 80, TCP, Verdict::Http),
            ("1.1.1.1", 443, UDP, Verdict::Deny),
            ("1.1.1.1", 22, TCP, Verdict::Deny),
            ("127.0.0.53", 53, UDP, Verdict::Dns),
            ("8.8.8.8", 53, TCP, Verdict::Dns),
            ("2001:4860:4860::8888", 53, UDP, Verdict::Dns),
            ("10.0.0.1", 443, TCP, Verdict::Tls),
            ("127.0.0.1", 443, TCP, Verdict::Owned),
            ("198.51.100.7", 443, TCP, Verdict::Allow),
        ] {
            assert_eq!(decide_in(&policy, address, port, protocol), expected, "{address}:{port}");
        }
    }

    #[test]
    fn a_request_denies_more_but_never_opens_what_the_host_keeps() {
        let allow = [at("10.0.0.0/8", None), at("203.0.113.0/24", None), at("fd00:ec2::/64", None)];
        let deny = [at("10.9.0.0/16", None), at("1.1.1.0/24", Some((53, 53)))];
        let policy = inputs(Reach::Public, &allow, &deny);
        for (address, port, expected) in [
            ("10.1.0.1", 42001, Verdict::Deny),
            ("10.1.0.1", 22, Verdict::Allow),
            ("10.9.0.1", 22, Verdict::Deny),
            ("1.1.1.1", 53, Verdict::Deny),
            ("1.1.1.1", 443, Verdict::Allow),
            ("203.0.113.9", 22, Verdict::Owned),
            ("203.0.113.10", 22, Verdict::Allow),
            ("fd00:ec2::254", 80, Verdict::Deny),
            ("fd00:ec2::253", 53, Verdict::Allow),
        ] {
            assert_eq!(decide_in(&policy, address, port, TCP), expected, "{address}:{port}");
        }
    }

    #[test]
    fn too_many_ranges_on_one_address_are_refused_by_name() {
        let deny: Vec<_> = (0..=PORT_ENTRIES as u16).map(|port| at("192.0.2.0/24", Some((port, port)))).collect();
        let problem = compile(&inputs(Reach::Public, &[], &deny)).unwrap_err();
        // Named as the request spells it, IPv4 as IPv4.
        assert!(problem.starts_with("192.0.2.0/24 would need 25 port ranges"), "{problem}");
    }
}
