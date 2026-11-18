// SPDX-License-Identifier: FSL-1.1-ALv2
//! Pure network retirement and packet-route regressions, independent of runtime journals and launches.
use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv6Addr, TcpListener, UdpSocket},
    process::{Child, Command, Stdio},
};

/// Retire the cgroup and host addresses even when an assertion fails midway through a fixture.
struct Fixture {
    network: Network,
    child: Option<Child>,
    addresses: Vec<&'static str>,
}

impl Fixture {
    fn add(&mut self, cidr: &'static str) {
        let mut command = Command::new("ip");
        command.args(["address", "add", cidr, "dev", "lo"]);
        if cidr.contains(':') {
            command.arg("nodad");
        }
        let status = command.status().unwrap();
        assert!(status.success(), "ip address add {cidr}: {status}");
        self.addresses.push(cidr);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.network.release("route-guard");
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        for cidr in self.addresses.drain(..) {
            let _ = Command::new("ip").args(["address", "del", cidr, "dev", "lo"]).status();
        }
    }
}

#[test]
fn cgroup_metadata_uncertainty_cannot_detach_recorded_defenses() {
    let root = tempfile::tempdir().unwrap();
    let cgroups = root.path().join("cgroups");
    std::fs::write(&cgroups, b"metadata lookup must fail with ENOTDIR").unwrap();
    let network = Network::new(
        Host { cgroups, pins: root.path().join("pins"), delegated: false },
        Served { ports: 42000..=42999, api_port: 7443 },
        root.path(),
    );
    let pins = network.host.pin_directory(&network.key("route-guard"));
    std::fs::create_dir_all(&pins).unwrap();
    let retained = pins.join("link-fixture");
    std::fs::write(&retained, b"fixture pin must remain").unwrap();
    assert!(network.release("route-guard").is_err());
    assert!(retained.is_file(), "metadata failure cannot authorize detaching the old programs");
}

#[tokio::test]
#[ignore = "adds host addresses and loads programs into the kernel"]
async fn newly_local_routes_are_refused_on_every_attempt_with_address_refresh_blocked() {
    if !rustix::process::geteuid().is_root() {
        eprintln!("root adds host addresses; the root run proves the stale-map route guard");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let host = Host::find().unwrap_or_else(|problem| panic!("{}", problem.evidence));
    let mut fixture = Fixture {
        network: Network::new(host, Served { ports: 42000..=42999, api_port: 7443 }, root.path()),
        child: None,
        addresses: Vec::new(),
    };
    let tcp4 = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    let udp4 = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    let tcp6 = TcpListener::bind((Ipv6Addr::UNSPECIFIED, 0)).unwrap();
    let udp6 = UdpSocket::bind((Ipv6Addr::UNSPECIFIED, 0)).unwrap();
    for socket in [&tcp4, &tcp6] {
        socket.set_nonblocking(true).unwrap();
    }
    for socket in [&udp4, &udp6] {
        socket.set_nonblocking(true).unwrap();
    }
    let spec = fixture_spec(root.path());
    let (_, cgroup) = fixture.network.prepare(&spec).await.unwrap();
    // Deliberately do not start Network::watch: the map stays at the old snapshot for the
    // whole test. Unlike an eventual-refusal retry, the very first packet must be refused.
    let snapshot = fixture.network.held.lock().unwrap()[&spec.name].host.clone();
    for address in ["198.51.100.79", "2001:db8::79"] {
        assert!(!snapshot.contains(&address.parse::<IpAddr>().unwrap()));
    }
    let ports = [
        tcp4.local_addr().unwrap().port(),
        udp4.local_addr().unwrap().port(),
        tcp6.local_addr().unwrap().port(),
        udp6.local_addr().unwrap().port(),
    ]
    .map(|port| port.to_string());
    let mut command = Command::new("python3");
    command.args(["-I", "-u", "-c", include_str!("route_fixture.py"), "198.51.100.79", "2001:db8::79"]);
    command.args(&ports).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
    host::drop_ambient_on_exec(&mut command);
    fixture.child = Some(fixture.network.spawn_verified(&spec.name, &mut command).unwrap());
    let child = fixture.child.as_mut().unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "ready", "owned IPv4/IPv6 TCP replies and IPv6 UDP options work");
    fixture.add("198.51.100.79/32");
    fixture.add("2001:db8::79/128");
    writeln!(input, "go").unwrap();
    for attempt in 0..10 {
        line.clear();
        output.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), format!("refused {attempt}"), "first and every repeated dial is refused");
        for socket in [&tcp4, &tcp6] {
            assert!(
                socket.accept().is_err_and(|problem| problem.kind() == std::io::ErrorKind::WouldBlock),
                "no stale-rule TCP connection reached the host on attempt {attempt}"
            );
        }
        for socket in [&udp4, &udp6] {
            assert!(
                socket.recv_from(&mut [0; 64]).is_err_and(|problem| problem.kind() == std::io::ErrorKind::WouldBlock),
                "no connected or unconnected UDP datagram reached the host on attempt {attempt}"
            );
        }
        assert_eq!(fixture.network.held.lock().unwrap()[&spec.name].host, snapshot);
        writeln!(input, "checked").unwrap();
    }
    assert!(fixture.child.as_mut().unwrap().wait().unwrap().success());
    fixture.network.release(&spec.name).unwrap();
    assert!(!cgroup.exists());
    eprintln!("K31_CURRENT_ROUTE first and all 10 attempts refused with address refresh blocked");
}
