// SPDX-License-Identifier: FSL-1.1-ALv2
//! Kernel regressions for where a policed sandbox's packets actually go, past the address its
//! hooks were shown: a subnet's broadcast address, a host address's NAT64 twin, an address the
//! rules still call this host's, another owner's reuse-port group, a source route's first hop,
//! and a listener of its own whose served side waits in TIME-WAIT. Root only: each adds a link to
//! this host. Retirement and stale-map routes stay in `guard_tests.rs`.
use super::*;
use std::{
    net::{TcpListener, UdpSocket},
    process::{Command, Stdio},
};

/// The fixture's served ports, as `Served` hands them out.
const SERVED: RangeInclusive<u16> = 42000..=42999;
/// The link standing in for a host's own network, its peer, and what this host holds on it.
const LINK: &str = "pryxrv0";
const PEER: &str = "pryxrv1";
const LINK_V4: &str = "10.214.0.1/24";
const LINK_V6: &str = "fd00:214::1/64";
/// That subnet's broadcast address, and an address on it that is no host's.
const BROADCAST: &str = "10.214.0.255";
const NEIGHBOUR_V4: &str = "10.214.0.7";
const NEIGHBOUR_V6: &str = "fd00:214::7";

fn ip(arguments: &[&str]) {
    let status = Command::new("ip").args(arguments).status().unwrap();
    assert!(status.success(), "ip {arguments:?}: {status}");
}

/// A veth pair up on this host with an IPv4 and an IPv6 subnet, removed with the test.
struct Link;

impl Link {
    fn add() -> Self {
        let _ = Command::new("ip").args(["link", "del", LINK]).status();
        ip(&["link", "add", LINK, "type", "veth", "peer", "name", PEER]);
        let link = Self;
        let _ = std::fs::write(format!("/proc/sys/net/ipv6/conf/{LINK}/disable_ipv6"), "0");
        ip(&["address", "add", LINK_V4, "dev", LINK]);
        ip(&["address", "add", LINK_V6, "dev", LINK, "nodad"]);
        ip(&["link", "set", PEER, "up"]);
        ip(&["link", "set", LINK, "up"]);
        link
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = Command::new("ip").args(["link", "del", LINK]).status();
    }
}

/// One policed sandbox cgroup, released with the test even after a failed assertion.
struct Policed(Network);

impl Policed {
    async fn prepare(root: &Path) -> Self {
        let host = Host::find().unwrap_or_else(|problem| panic!("{}", problem.evidence));
        let policed = Self(Network::new(host, Served { ports: SERVED, api_port: 7443 }, root));
        policed.0.prepare(&fixture_spec(root)).await.unwrap();
        policed
    }

    /// One mode of `paths_fixture.py` inside the cgroup, and the outcomes it printed.
    fn run(&self, arguments: &[&str]) -> String {
        let mut command = Command::new("python3");
        command.args(["-I", "-u", "-c", include_str!("paths_fixture.py")]).args(arguments);
        command.stdout(Stdio::piped()).stderr(Stdio::inherit());
        host::drop_ambient_on_exec(&mut command);
        let output = self.0.spawn_verified("route-guard", &mut command).unwrap().wait_with_output().unwrap();
        assert!(output.status.success(), "{arguments:?}: {}", output.status);
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn refused(&self) -> u64 {
        self.0.counts("route-guard").expect("attached programs count").refused_addresses
    }
}

impl Drop for Policed {
    fn drop(&mut self) {
        let _ = self.0.release("route-guard");
    }
}

fn root() -> bool {
    let root = rustix::process::geteuid().is_root();
    if !root {
        eprintln!("only root adds a link to this host: the root run proves this case");
    }
    root
}

/// A host service's port on every address, outside the fixture's served ports.
fn unserved<T>(bind: impl Fn() -> std::io::Result<T>, port: impl Fn(&T) -> u16) -> T {
    loop {
        let socket = bind().unwrap();
        if !SERVED.contains(&port(&socket)) {
            return socket;
        }
    }
}

#[tokio::test]
#[ignore = "adds a host link and loads programs into the kernel"]
async fn a_subnet_broadcast_never_reaches_a_host_service() {
    if !root() {
        return;
    }
    let _link = Link::add();
    let root = tempfile::tempdir().unwrap();
    let policed = Policed::prepare(root.path()).await;
    let service = unserved(|| UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)), |s| s.local_addr().unwrap().port());
    service.set_nonblocking(true).unwrap();
    let port = service.local_addr().unwrap().port().to_string();
    let before = policed.refused();
    let sent = policed.run(&["broadcast", BROADCAST, &port]);
    std::thread::sleep(std::time::Duration::from_millis(200));
    let heard = service.recv_from(&mut [0; 64]);
    assert!(
        heard.as_ref().is_err_and(|problem| problem.kind() == std::io::ErrorKind::WouldBlock),
        "{heard:?}: a broadcast from the sandbox ({sent}) reached the host's own UDP service"
    );
    assert_eq!(sent, ["refused 1"; 4].join(" "), "each is refused before it is sent");
    assert!(policed.refused() > before);
}

#[tokio::test]
#[ignore = "adds a host link and loads programs into the kernel"]
async fn a_host_address_nat64_twin_is_refused_even_at_a_listener_of_its_own() {
    if !root() {
        return;
    }
    let _link = Link::add();
    let root = tempfile::tempdir().unwrap();
    let policed = Policed::prepare(root.path()).await;
    // RFC 6052 §2.1: the well-known prefix, 64:ff9b::/96, with the IPv4 address in its last 32 bits.
    let host: Ipv4Addr = LINK_V4.split('/').next().unwrap().parse().unwrap();
    let twin = std::net::Ipv6Addr::from((0x64_ff9b_u128 << 96) | u128::from(u32::from(host)));
    assert_eq!(policed.run(&["twin", &twin.to_string()]), "refused 1", "{twin} reaches {host} through NAT64");
}

#[tokio::test]
#[ignore = "adds a host link and loads programs into the kernel"]
async fn an_owned_verdict_never_leaves_the_host() {
    if !root() {
        return;
    }
    let _link = Link::add();
    // This host's when the rules are compiled; gone, and routed off the host, when dialled.
    let stale = "10.214.0.9";
    ip(&["address", "add", &format!("{stale}/32"), "dev", LINK]);
    let root = tempfile::tempdir().unwrap();
    let policed = Policed::prepare(root.path()).await;
    ip(&["address", "del", &format!("{stale}/32"), "dev", LINK]);
    let before = policed.refused();
    let dialled = policed.run(&["stale", stale]);
    assert_ne!(dialled, "ok", "nothing answers there");
    assert!(policed.refused() > before, "the packet hook refused the SYN off the host ({dialled})");
}

#[tokio::test]
#[ignore = "adds a host link and loads programs into the kernel"]
async fn another_owners_reuse_port_group_never_takes_a_sandbox_connection() {
    if !root() {
        return;
    }
    use rustix::net::{AddressFamily, SocketType, sockopt};
    let root = tempfile::tempdir().unwrap();
    let policed = Policed::prepare(root.path()).await;
    // A host service of the same user, in a reuse-port group the sandbox may join.
    let service = unserved(
        || {
            let socket = rustix::net::socket(AddressFamily::INET, SocketType::STREAM, None)?;
            sockopt::set_socket_reuseport(&socket, true)?;
            rustix::net::bind(&socket, &std::net::SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
            rustix::net::listen(&socket, 128)?;
            Ok(TcpListener::from(socket))
        },
        |listener| listener.local_addr().unwrap().port(),
    );
    service.set_nonblocking(true).unwrap();
    let port = service.local_addr().unwrap().port().to_string();
    let tried = policed.run(&["reuseport", &port]);
    let mut taken = 0;
    while service.accept().is_ok() {
        taken += 1;
    }
    assert_eq!(taken, 0, "the host's listener took {taken} of the sandbox's connections ({tried})");
}

#[tokio::test]
#[ignore = "adds a host link and loads programs into the kernel"]
async fn a_source_route_is_held_to_its_first_hop() {
    if !root() {
        return;
    }
    let _link = Link::add();
    let root = tempfile::tempdir().unwrap();
    let policed = Policed::prepare(root.path()).await;
    // A private neighbour on a served port is refused, wherever the route ends.
    let served = SERVED.start().saturating_add(1).to_string();
    assert_eq!(policed.run(&["source-route", NEIGHBOUR_V4, NEIGHBOUR_V6, &served]), "refused 1 refused 1");
    // The same neighbour on an ordinary port may be a first hop.
    assert_eq!(policed.run(&["source-route", NEIGHBOUR_V4, NEIGHBOUR_V6, "5432"]), "ok ok");
}

#[tokio::test]
#[ignore = "adds a host link and loads programs into the kernel"]
async fn a_listener_of_its_own_is_dialled_again_while_its_served_side_waits() {
    if !root() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let policed = Policed::prepare(root.path()).await;
    assert_eq!(policed.run(&["time-wait"]), "ok");
}
