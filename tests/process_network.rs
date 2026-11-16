// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The process tier's kernel network policy with real supervisors, programs and destinations.
//! The ignored cases need a host that grants it: CI runs them as root and as an ordinary user
//! with a delegated cgroup, one at a time, with `PRYX_TEST_AGENT` naming a trusted agent,
//! `PRYX_TEST_PRIVATE` a private address with two listening ports, and `PRYX_TEST_REMOTE` a
//! machine elsewhere (another network namespace) that dials this host at `PRYX_TEST_EDGE`, or
//! sends it datagrams from the source ports named, when asked. With `PRYX_TEST_NAMESPACES=on`
//! every sandbox must also have started in namespaces of its own (the agent's helper beside it). The case that runs everywhere holds a host without the capability to its refusal.
mod support;

use pryxd::manager::{Manager, Precondition, Records};
use pryxd::protocol::{DataDisposition, ExecRequest, Guarantee, WorkloadState};
use pryxd::runtime::ContainerRuntime;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;
use support::{config, id, manager_on, until};

/// A connection attempt from inside a sandbox: `reached`, or `refused` with the errno.
const CONNECT: &str = "import socket,sys
try:
    socket.create_connection((sys.argv[1], int(sys.argv[2])), timeout=5).close()
    print('reached')
except OSError as e:
    print('refused', e.errno, e)
";
/// A listener the sandbox makes and then reaches over loopback.
const OWN_LISTENER: &str = "import socket
l = socket.socket()
l.bind(('127.0.0.1', 0))
l.listen()
socket.create_connection(l.getsockname(), timeout=5).close()
print('reached')
";
/// A DNS question sent as one unconnected datagram, and where its answer says it came from.
const DATAGRAM_DNS: &str = "import socket,struct,sys
q = struct.pack('>HHHHHH', 0x1234, 0x0100, 1, 0, 0, 0)
q += b''.join(bytes([len(p)]) + p.encode() for p in sys.argv[1].split('.')) + b'\\x00' + struct.pack('>HH', 1, 1)
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.settimeout(10)
s.sendto(q, (sys.argv[2], 53))
data, source = s.recvfrom(4096)
print('answered', source[0], source[1], data[3] & 15)
";

/// A listener of the sandbox's own on every address, reached from this host at the edge's
/// address, then dialled by the machine elsewhere; prints what that machine got.
const WIDE_LISTENER: &str = "import socket,sys,urllib.request
l = socket.socket()
l.bind(('0.0.0.0', 0))
l.listen()
port = l.getsockname()[1]
socket.create_connection((sys.argv[2], port), timeout=5).close()
print(urllib.request.urlopen(f'http://{sys.argv[1]}/dial?port={port}', timeout=20).read().decode())
";

/// Two unconnected datagram sockets, IPv4 and dual-stack IPv6, each sending one datagram to the
/// machine elsewhere on port 9001 (its IPv4-mapped address for the second), then asking it to
/// send back from 9001 and from 9002; prints the source ports each heard.
const UDP_PEERS: &str = "import socket,sys,urllib.request
remote = sys.argv[1]
host = remote.split(':')[0]
heard = []
for family, peer, bound in ((socket.AF_INET, host, '0.0.0.0'), (socket.AF_INET6, '::ffff:' + host, '::')):
    s = socket.socket(family, socket.SOCK_DGRAM)
    if family == socket.AF_INET6:
        s.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
    s.bind((bound, 0))
    s.sendto(b'hello', (peer, 9001))
    port = s.getsockname()[1]
    urllib.request.urlopen(f'http://{remote}/udp?port={port}&from=9001,9002', timeout=20).read()
    s.settimeout(3)
    sources = []
    try:
        while True:
            sources.append(str(s.recvfrom(64)[1][1]))
    except socket.timeout:
        pass
    heard.append('+'.join(sorted(sources)) or 'nothing')
print(' '.join(heard))
";

/// The embedded object with its last program one the verifier refuses (`bpf/build.sh`).
static REJECTED: &[u8] = aya::include_bytes_aligned!("../bpf/rejected.bpf.o");

fn agent() -> String {
    std::env::var("PRYX_TEST_AGENT").expect("PRYX_TEST_AGENT names a trusted pryx-agent")
}

/// The private address and its two listening ports, `address:allowed:other`.
fn private() -> (String, u16, u16) {
    let value = std::env::var("PRYX_TEST_PRIVATE").expect("PRYX_TEST_PRIVATE is address:port:port");
    let mut parts = value.split(':');
    let address = parts.next().unwrap().to_owned();
    let mut port = || parts.next().unwrap().parse().unwrap();
    (address, port(), port())
}

/// The machine elsewhere that dials on request, and this host's address it dials, as the edge does.
fn remote() -> (String, String) {
    let remote = std::env::var("PRYX_TEST_REMOTE").expect("PRYX_TEST_REMOTE is address:port");
    (remote, std::env::var("PRYX_TEST_EDGE").expect("PRYX_TEST_EDGE is this host's address there"))
}

/// What the machine elsewhere got dialling `port` at the edge's address.
fn dial(remote: &str, port: u16) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(remote).unwrap();
    write!(stream, "GET /dial?port={port} HTTP/1.0\r\n\r\n").unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer.split_once("\r\n\r\n").map(|(_, body)| body.to_owned()).unwrap_or(answer)
}

/// Ports below Linux's ephemeral range (32768 and up), so no outgoing connection holds one.
/// Agents listen at `edge` as well as loopback when it is given.
fn extra(ports: u16, edge: Option<&str>) -> String {
    let edge = edge.map(|address| format!("edge_ips = [\"127.0.0.1\", \"{address}\"]\n")).unwrap_or_default();
    format!(
        "runtime = \"process\"\n[workloads]\nagent_binary = \"{}\"\n[network]\nport_range = [{ports}, {}]\nport_quarantine_seconds = 0\n{edge}",
        agent(),
        ports + 99
    )
}

async fn node(dir: &std::path::Path, ports: u16) -> Arc<Manager> {
    node_at(dir, ports, None).await
}

async fn node_at(dir: &std::path::Path, ports: u16, edge: Option<&str>) -> Arc<Manager> {
    // As root each sandbox has a user of its own, which must pass through to its data.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o711)).unwrap();
    let config = Arc::new(config(dir, &extra(ports, edge)));
    // In the order `pryxd serve` takes them: the store, then the runtime over it.
    let store = pryxd::store::Store::open(&config.state_dir).unwrap();
    let runtime = pryxd::runtime::configured::create(&config).await.expect("the process tier starts");
    let capability = runtime.kernel_network_policy().expect("the process tier reports kernel network policy");
    assert_eq!(capability.state, pryxd::probe::State::On, "{capability:?}");
    // A port another program holds is skipped, as serve skips it.
    let probe = pryxd::ports::bind_probe(config.network.edge_ips.clone());
    let manager = Manager::new(config, runtime, store, Arc::new(pryxd::metrics::Metrics::new()), probe);
    manager.refresh_host_probe().await;
    let report = manager.reconcile(Records::FromDisk).await;
    assert!(report.error.is_none(), "{report:?}");
    manager
}

fn sandbox(policy: serde_json::Value) -> pryxd::protocol::WorkloadSpec {
    let mut spec = serde_json::json!({
        "resources": { "memoryMb": 256 },
        "storage": { "mountPath": "/data", "sizeGb": 1 },
        "agent": { "domain": "pryx.test", "keepAlive": true },
        "stop": { "timeoutSeconds": 2 },
    });
    for (key, value) in policy.as_object().unwrap() {
        spec[key] = value.clone();
    }
    serde_json::from_value(spec).unwrap()
}

async fn start(manager: &Arc<Manager>, name: &str, policy: serde_json::Value) {
    manager
        .ensure(id(name), sandbox(policy), Precondition::None, None)
        .await
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    if let Err(problem) = manager.start(id(name), None).await {
        panic!("starting {name}: {problem:?}\n{}", diagnostics(manager));
    }
    let view = until(manager, name, "a running supervisor", |view| view.state == WorkloadState::Running).await;
    assert!(view.guarantees[&Guarantee::HostFirewall], "{name}'s programs are attached and verified");
    // Where CI starts every supervisor in namespaces of its own, through the helper.
    if std::env::var("PRYX_TEST_NAMESPACES").as_deref() == Ok("on") && !view.guarantees[&Guarantee::PrivatePids] {
        let report = pryxd::probe::for_config(&manager.config).await;
        let mode = &report.capabilities[pryxd::runtime::process::NAMESPACES];
        panic!("{name} did not start namespaced: {mode:?}\n{view:?}");
    }
}

async fn run(manager: &Arc<Manager>, name: &str, command: &[&str]) -> String {
    let request = ExecRequest { command: command.iter().map(|part| (*part).to_owned()).collect(), timeout_seconds: 40 };
    let response = match manager.exec(id(name), request, None, None).await {
        Ok(response) => response,
        Err(problem) => panic!("{name} running {command:?}: {problem:?}\n{}", diagnostics(manager)),
    };
    response.stdout.trim().to_owned()
}

async fn connect(manager: &Arc<Manager>, name: &str, address: &str, port: u16) -> String {
    run(manager, name, &["python3", "-c", CONNECT, address, &port.to_string()]).await
}

async fn assert_refused(manager: &Arc<Manager>, name: &str, address: &str, port: u16) {
    let outcome = connect(manager, name, address, port).await;
    // EPERM: the connect hook refused it before any packet left.
    assert!(outcome.starts_with("refused 1 "), "{name} -> {address}:{port}: {outcome}");
}

async fn assert_reached(manager: &Arc<Manager>, name: &str, address: &str, port: u16) {
    let outcome = connect(manager, name, address, port).await;
    assert_eq!(outcome, "reached", "{name} -> {address}:{port}");
}

async fn curl(manager: &Arc<Manager>, name: &str, url: &str) -> String {
    run(
        manager,
        name,
        &["curl", "--silent", "--output", "/dev/null", "--write-out", "%{http_code}", "--max-time", "20", url],
    )
    .await
}

/// `curl` from inside a sandbox with the name pinned to an address (`--resolve`'s `host:port:address`).
async fn curl_at(manager: &Arc<Manager>, name: &str, url: &str, pinned: &str) -> String {
    let command = ["curl", "--silent", "--output", "/dev/null", "--write-out", "%{http_code}", "--max-time", "20"];
    let command: Vec<&str> = command.into_iter().chain(["--resolve", pinned, url]).collect();
    run(manager, name, &command).await
}

fn agent_port(manager: &Arc<Manager>, name: &str) -> u16 {
    manager.view(&id(name)).unwrap().agent.expect("a supervisor").port.host_port
}

/// A service of this host's own, on loopback: no sandbox may reach it.
fn host_service() -> (TcpListener, u16) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

async fn remove(manager: &Arc<Manager>, names: &[&str]) {
    for name in names {
        manager.delete(id(name), DataDisposition::Delete, Some((*name).to_owned()), None).await.unwrap();
    }
}

/// The sandbox cgroups under this process's own, and the pin directories, that exist now.
fn kernel_state() -> std::collections::BTreeSet<std::path::PathBuf> {
    let path = std::fs::read_to_string("/proc/self/cgroup").unwrap();
    let path = path.lines().find_map(|line| line.strip_prefix("0::")).unwrap().trim().to_owned();
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap();
    let mount = mountinfo
        .lines()
        .find_map(|line| {
            let (before, after) = line.split_once(" - ")?;
            after.starts_with("cgroup2 ").then(|| before.split(' ').nth(4).unwrap().to_owned())
        })
        .unwrap();
    let own = std::path::Path::new(&mount).join(path.trim_start_matches('/'));
    let pins = std::path::Path::new("/sys/fs/bpf/pryx");
    [own.as_path(), pins]
        .into_iter()
        .filter_map(|directory| std::fs::read_dir(directory).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            if path.parent() == Some(pins) {
                !name.starts_with("probe-")
            } else {
                name.starts_with("pryx-") && !name.starts_with("pryx-probe-")
            }
        })
        .collect()
}

/// Nothing a test's sandboxes made in the kernel outlives them: their cgroups and pins are gone.
fn nothing_left(before: &std::collections::BTreeSet<std::path::PathBuf>) {
    let left: Vec<_> = kernel_state().difference(before).cloned().collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}

/// The supervisors' own diagnostics, for a command the agent could not run.
fn diagnostics(manager: &Arc<Manager>) -> String {
    let root = manager.config.state_dir.join("process-runtime");
    std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "stderr"))
        .map(|entry| {
            let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
            let tail: String = text.chars().rev().take(4000).collect::<Vec<_>>().into_iter().rev().collect();
            format!("{}:\n{tail}", entry.path().display())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn without_the_capability_the_guarantee_is_off_and_policy_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let extra = "runtime = \"process\"\n";
    let runtime = pryxd::runtime::configured::create(&config(dir.path(), extra)).await.unwrap();
    let capability = runtime.kernel_network_policy().expect("the process tier reports kernel network policy");
    let manager = manager_on(dir.path(), runtime, extra);
    manager.refresh_host_probe().await;
    let features = manager.features();
    if capability.state == pryxd::probe::State::On {
        // A host that grants it; the ignored cases below prove what it gives.
        assert!(features.iter().any(|feature| feature == "egress-addresses"), "{features:?}");
        return;
    }
    assert!(!capability.evidence.is_empty() && !capability.action.is_empty(), "{capability:?}");
    for absent in ["egress-addresses", "egress-hostnames", "host-firewall"] {
        assert!(!features.iter().any(|feature| feature == absent), "{absent} in {features:?}");
    }
    for (field, policy) in [
        (
            "egressAddresses",
            serde_json::json!({ "egressAddresses": { "allow": [{ "prefix": "192.0.2.1", "ports": [443, 443] }] } }),
        ),
        ("egress", serde_json::json!({ "egress": { "allow": ["example.com"] } })),
    ] {
        let refused = manager.ensure(id("refused"), sandbox(policy), Precondition::None, None).await.unwrap_err();
        let pryxd::manager::NodeError::Invalid(fields) = refused else { panic!("{field}: {refused:?}") };
        let fields = serde_json::to_value(&fields).unwrap();
        assert!(
            fields.as_array().unwrap().iter().any(|f| f["field"] == field
                && f["problem"].as_str().is_some_and(|problem| problem.contains("kernel-network-policy"))),
            "{field}: {fields}"
        );
    }
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn without_public_internet_a_sandbox_reaches_only_its_allowed_destination() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    let manager = node(dir.path(), 21100).await;
    let (private, allowed, other) = private();
    let (_service, service) = host_service();
    start(
        &manager,
        "closed",
        serde_json::json!({
            "egress": { "allow": [] },
            "egressAddresses": { "allow": [{ "prefix": format!("{private}/32"), "ports": [allowed, allowed] }] },
        }),
    )
    .await;
    start(&manager, "neighbour", serde_json::json!({})).await;
    assert_reached(&manager, "closed", &private, allowed).await;
    assert_refused(&manager, "closed", &private, other).await;
    for (address, port) in [("1.1.1.1", 853), ("8.8.8.8", 22), ("169.254.169.254", 8080), ("10.255.255.1", 5432)] {
        assert_refused(&manager, "closed", address, port).await;
    }
    for url in ["https://example.com", "http://example.com", "http://1.1.1.1"] {
        assert_ne!(curl(&manager, "closed", url).await, "200", "{url} through a gateway that names no host");
    }
    assert_refused(&manager, "closed", "127.0.0.1", service).await;
    assert_refused(&manager, "closed", "127.0.0.1", agent_port(&manager, "neighbour")).await;
    assert_refused(&manager, "neighbour", "127.0.0.1", agent_port(&manager, "closed")).await;
    assert_reached(&manager, "closed", "127.0.0.1", agent_port(&manager, "closed")).await;
    assert_eq!(run(&manager, "closed", &["python3", "-c", OWN_LISTENER]).await, "reached");
    remove(&manager, &["closed", "neighbour"]).await;
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn hostname_policy_reaches_only_the_named_host_through_the_gateway() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    let manager = node(dir.path(), 21200).await;
    start(&manager, "named", serde_json::json!({ "egress": { "allow": ["example.com"], "deny": [] } })).await;
    assert_eq!(curl(&manager, "named", "https://example.com").await, "200");
    for url in ["https://example.org", "http://example.org", "http://1.1.1.1", "https://1.1.1.1"] {
        assert_ne!(curl(&manager, "named", url).await, "200", "{url}");
    }
    // A datagram to any resolver goes to the gateway's, and its answer names the resolver asked.
    assert_eq!(
        run(&manager, "named", &["python3", "-c", DATAGRAM_DNS, "example.com", "8.8.8.8"]).await,
        "answered 8.8.8.8 53 0"
    );
    assert_eq!(
        run(&manager, "named", &["python3", "-c", DATAGRAM_DNS, "example.org", "8.8.8.8"]).await,
        "answered 8.8.8.8 53 5"
    );
    assert_refused(&manager, "named", "1.1.1.1", 853).await;
    remove(&manager, &["named"]).await;
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn a_named_host_is_not_dialled_at_an_address_the_sandbox_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    let manager = node(dir.path(), 21700).await;
    // What example.com resolves to here, as the gateway's own lookup finds it: refused on 443.
    let resolved: Vec<std::net::IpAddr> =
        tokio::net::lookup_host(("example.com", 443)).await.unwrap().map(|address| address.ip()).collect();
    let deny: Vec<_> = resolved
        .iter()
        .map(|address| {
            let prefix = match address {
                std::net::IpAddr::V4(_) => ipnet::IpNet::new(*address, 24),
                std::net::IpAddr::V6(_) => ipnet::IpNet::new(*address, 48),
            };
            serde_json::json!({ "prefix": prefix.unwrap().trunc().to_string(), "ports": [443, 443] })
        })
        .collect();
    start(
        &manager,
        "refusing",
        serde_json::json!({ "egress": { "allow": ["example.com"] }, "egressAddresses": { "deny": deny } }),
    )
    .await;
    // Dialled at an address nothing refuses, the connection reaches the gateway, which resolves
    // the name into a refused upstream and dials nothing; on port 80 nothing is refused.
    let https = curl_at(&manager, "refusing", "https://example.com", "example.com:443:1.1.1.1").await;
    assert_ne!(https, "200", "{resolved:?} is refused on 443");
    let http = curl_at(&manager, "refusing", "http://example.com", "example.com:80:1.1.1.1").await;
    assert_eq!(http, "200", "the same name on a port nothing refuses");
    remove(&manager, &["refusing"]).await;
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn public_internet_keeps_the_host_metadata_and_other_sandboxes_off() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    let manager = node(dir.path(), 21300).await;
    let (private, allowed, _) = private();
    let (_service, service) = host_service();
    start(&manager, "public", serde_json::json!({})).await;
    start(&manager, "neighbour", serde_json::json!({})).await;
    assert_reached(&manager, "public", "1.1.1.1", 443).await;
    assert_eq!(curl(&manager, "public", "https://example.com").await, "200");
    assert_reached(&manager, "public", &private, allowed).await;
    assert_refused(&manager, "public", "169.254.169.254", 80).await;
    assert_refused(&manager, "public", &private, 21301).await;
    assert_refused(&manager, "public", "127.0.0.1", service).await;
    assert_refused(&manager, "public", "127.0.0.1", agent_port(&manager, "neighbour")).await;
    if let Ok(address) = std::env::var("PRYX_TEST_HOST_ADDRESS") {
        let wide = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let port = wide.local_addr().unwrap().port();
        assert!(std::net::TcpStream::connect(SocketAddr::new(address.parse().unwrap(), port)).is_ok());
        assert_refused(&manager, "public", &address, port).await;
    }
    assert_eq!(run(&manager, "public", &["python3", "-c", OWN_LISTENER]).await, "reached");
    let refused = manager.ensure(
        id("hole"),
        sandbox(serde_json::json!({ "egressAddresses": { "allow": [{ "prefix": "127.0.0.0/8" }] } })),
        Precondition::None,
        None,
    );
    let refused = refused.await.unwrap_err();
    assert!(format!("{refused:?}").contains("can't open this host"), "{refused:?}");
    // Rules that fold into more port ranges than one prefix holds are refused, naming the field.
    let nested: Vec<_> = (0..25u16)
        .map(|bits| serde_json::json!({ "prefix": format!("203.0.113.0/{}", 8 + bits), "ports": [1000 + bits, 1000 + bits] }))
        .collect();
    let policy = serde_json::json!({ "egressAddresses": { "deny": nested } });
    let refused = manager.ensure(id("nested"), sandbox(policy), Precondition::None, None).await.unwrap_err();
    let pryxd::manager::NodeError::Invalid(fields) = refused else { panic!("{refused:?}") };
    let fields = serde_json::to_value(&fields).unwrap();
    assert!(
        fields.as_array().unwrap().iter().any(|f| f["field"] == "egressAddresses"
            && f["problem"]
                .as_str()
                .is_some_and(|problem| problem.contains("203.0.113.0/32 would need 25 port ranges"))),
        "{fields}"
    );
    remove(&manager, &["public", "neighbour"]).await;
    nothing_left(&before);
}

/// An address on this host's loopback device for as long as it lives, as `ip` adds one.
struct HostAddress(&'static str);

impl HostAddress {
    fn add(address: &'static str) -> Self {
        let added = std::process::Command::new("ip").args(["address", "add", address, "dev", "lo"]).status().unwrap();
        assert!(added.success(), "ip address add {address}: {added}");
        Self(address)
    }
}

impl Drop for HostAddress {
    fn drop(&mut self) {
        let _ = std::process::Command::new("ip").args(["address", "del", self.0, "dev", "lo"]).status();
    }
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn an_address_the_host_gains_is_kept_off_as_its_others_are() {
    if !rustix::process::geteuid().is_root() {
        eprintln!("only root adds an address to this host: the root run proves this case");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    let manager = node(dir.path(), 21800).await;
    start(&manager, "public", serde_json::json!({})).await;
    // A service of this host's on every address, the new one included once it is added.
    let wide = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    let port = wide.local_addr().unwrap().port();
    // Not a private address, so with public internet on only its being this host's refuses it.
    let added = HostAddress::add("198.51.100.7/32");
    let since = std::time::Instant::now();
    let mut attempts = 0;
    loop {
        attempts += 1;
        let outcome = connect(&manager, "public", "198.51.100.7", port).await;
        if outcome.starts_with("refused 1 ") {
            break;
        }
        assert!(
            since.elapsed() < std::time::Duration::from_secs(5),
            "{outcome} {:?} after it was added",
            since.elapsed()
        );
    }
    // The dial's own start (an exec and a Python) bounds the window from above.
    eprintln!(
        "K31_ADDRESS_WATCH refused on dial {attempts}, {} ms after the address was added",
        since.elapsed().as_millis()
    );
    drop(added);
    remove(&manager, &["public"]).await;
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn the_policy_outlives_a_daemon_restart_and_nothing_outlives_the_sandbox() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    let (_service, service) = host_service();
    {
        let manager = node(dir.path(), 21400).await;
        start(&manager, "kept", serde_json::json!({ "egress": { "allow": ["example.com"] } })).await;
        assert_refused(&manager, "kept", "127.0.0.1", service).await;
    }
    // A new daemon over the same state: the supervisor ran on, and its programs with it.
    let manager = node(dir.path(), 21400).await;
    let view = until(&manager, "kept", "the adopted supervisor", |view| view.state == WorkloadState::Running).await;
    assert!(view.guarantees[&Guarantee::HostFirewall], "adopted programs are verified again");
    assert_refused(&manager, "kept", "127.0.0.1", service).await;
    assert_eq!(curl(&manager, "kept", "https://example.com").await, "200", "the gateway is served again");
    assert_ne!(curl(&manager, "kept", "https://example.org").await, "200");
    let stats = serde_json::to_value(manager.stats(&id("kept")).await.unwrap()).unwrap();
    for counted in ["networkRxBytes", "networkTxBytes"] {
        assert!(stats[counted].as_u64().is_some_and(|bytes| bytes > 0), "the programs count traffic: {stats}");
    }
    remove(&manager, &["kept"]).await;
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn a_sandbox_whose_programs_are_gone_after_a_restart_is_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    {
        // Public internet and no policy of its own: its programs are all that keep it off the host.
        let manager = node(dir.path(), 22000).await;
        start(&manager, "plain", serde_json::json!({})).await;
    }
    // While no daemon runs, its pins go, and its programs with them: what a lost pin leaves.
    let pins = std::path::Path::new("/sys/fs/bpf/pryx");
    for directory in kernel_state().difference(&before).filter(|path| path.parent() == Some(pins)) {
        for pin in std::fs::read_dir(directory).unwrap() {
            std::fs::remove_file(pin.unwrap().path()).unwrap();
        }
    }
    let manager = node(dir.path(), 22000).await;
    let view =
        until(&manager, "plain", "the unverified supervisor stopped", |view| view.state != WorkloadState::Running)
            .await;
    assert!(!view.guarantees[&Guarantee::HostFirewall], "{view:?}");
    remove(&manager, &["plain"]).await;
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn a_failed_probe_on_restart_stops_recorded_sandboxes_and_refuses_a_plain_relaunch() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    {
        let manager = node(dir.path(), 22200).await;
        start(&manager, "recorded", serde_json::json!({})).await;
    }
    let extra = extra(22200, None);
    let config = config(dir.path(), &extra);
    let served = pryxd::runtime::process::Served { ports: 22200..=22299, api_port: config.api.listen.port() };
    let runtime = pryxd::runtime::process::ProcessRuntime::new(&config.state_dir, config.process.clone())
        .unwrap()
        .with_network_programs(served, REJECTED)
        .await;
    assert_eq!(runtime.kernel_network_policy().unwrap().state, pryxd::probe::State::Off);
    let manager = manager_on(dir.path(), Arc::new(runtime), &extra);
    manager.refresh_host_probe().await;
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    let view = until(&manager, "recorded", "the previously policed supervisor stopped", |view| {
        view.state != WorkloadState::Running
    })
    .await;
    assert!(!view.guarantees[&Guarantee::HostFirewall]);
    let refused = manager.start(id("recorded"), None).await.unwrap_err();
    assert!(format!("{refused:?}").contains("kernel-network-policy"), "{refused:?}");
    remove(&manager, &["recorded"]).await;
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn a_sandbox_started_without_programs_keeps_the_tier_guarantee_off_while_it_runs() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o711)).unwrap();
    let extra = extra(22100, None);
    let daemon = |runtime: Arc<dyn ContainerRuntime>| {
        let config = Arc::new(config(dir.path(), &extra));
        let store = pryxd::store::Store::open(&config.state_dir).unwrap();
        let probe = pryxd::ports::bind_probe(config.network.edge_ips.clone());
        Manager::new(config, runtime, store, Arc::new(pryxd::metrics::Metrics::new()), probe)
    };
    {
        // A daemon from before this host had the capability: the process runtime without it.
        let config = config(dir.path(), &extra);
        let runtime = pryxd::runtime::process::ProcessRuntime::new(&config.state_dir, config.process).unwrap();
        let manager = daemon(Arc::new(runtime));
        manager.refresh_host_probe().await;
        assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
        manager.ensure(id("legacy"), sandbox(serde_json::json!({})), Precondition::None, None).await.unwrap();
        manager.start(id("legacy"), None).await.unwrap();
        until(&manager, "legacy", "a running supervisor", |view| view.state == WorkloadState::Running).await;
    }
    // The same state under a daemon with the capability: the old supervisor ran on, with none.
    let runtime = pryxd::runtime::configured::create(&config(dir.path(), &extra)).await.unwrap();
    let capability = runtime.kernel_network_policy().unwrap();
    assert_eq!(capability.state, pryxd::probe::State::Off, "{capability:?}");
    assert!(capability.evidence.starts_with("1 process sandbox(es) run without"), "{capability:?}");
    let manager = daemon(runtime);
    manager.refresh_host_probe().await;
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    let view = until(&manager, "legacy", "the adopted supervisor", |view| view.state == WorkloadState::Running).await;
    assert!(!view.guarantees[&Guarantee::HostFirewall], "{view:?}");
    assert!(!manager.features().iter().any(|feature| feature == "host-firewall"), "{:?}", manager.features());
    // Once it stops, nothing runs unpoliced and the tier's guarantee is back.
    manager.stop(id("legacy"), None, None).await.unwrap();
    until(&manager, "legacy", "the stopped supervisor", |view| view.state != WorkloadState::Running).await;
    assert!(manager.features().iter().any(|feature| feature == "host-firewall"), "{:?}", manager.features());
    // Started again, it gets its programs as any sandbox does.
    manager.start(id("legacy"), None).await.unwrap();
    let view = until(&manager, "legacy", "the restarted supervisor", |view| view.state == WorkloadState::Running).await;
    assert!(view.guarantees[&Guarantee::HostFirewall], "{view:?}");
    remove(&manager, &["legacy"]).await;
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "needs a host that grants kernel network policy"]
async fn from_off_the_host_only_the_published_ports_are_reached() {
    let dir = tempfile::tempdir().unwrap();
    let before = kernel_state();
    let (remote, edge) = remote();
    let manager = node_at(dir.path(), 21500, Some(&edge)).await;
    start(&manager, "listening", serde_json::json!({})).await;
    let elsewhere = run(&manager, "listening", &["python3", "-c", WIDE_LISTENER, &remote, &edge]).await;
    assert!(elsewhere.starts_with("unreached "), "a port nothing published, from off the host: {elsewhere}");
    // The agent's port is published: the edge dials it there and its agent answers.
    let published = dial(&remote, agent_port(&manager, "listening"));
    assert!(published.starts_with("reached HTTP/1."), "the agent's port from off the host: {published}");
    // An unconnected datagram socket hears, from off the host, only the peer it sent to.
    let heard = run(&manager, "listening", &["python3", "-c", UDP_PEERS, &remote]).await;
    assert_eq!(heard, "9001 9001", "IPv4, then IPv4-mapped IPv6: only peer 9001 was sent to");
    let stats = serde_json::to_value(manager.stats(&id("listening")).await.unwrap()).unwrap();
    assert!(stats["networkRxBytes"].as_u64().is_some_and(|bytes| bytes > 0), "{stats}");
    remove(&manager, &["listening"]).await;
    nothing_left(&before);
}

#[tokio::test]
#[ignore = "loads programs into the kernel"]
async fn a_program_the_verifier_rejects_turns_the_guarantee_off_and_policy_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let extra = extra(21600, None);
    let config = config(dir.path(), &extra);
    let served = pryxd::runtime::process::Served { ports: 21600..=21699, api_port: config.api.listen.port() };
    let runtime = pryxd::runtime::process::ProcessRuntime::new(&config.state_dir, config.process.clone())
        .unwrap()
        .with_network_programs(served, REJECTED)
        .await;
    let capability = runtime.kernel_network_policy().expect("the process tier reports kernel network policy");
    assert_eq!(capability.state, pryxd::probe::State::Off, "{capability:?}");
    // The verifier's own words, about the set's last program.
    assert!(capability.evidence.contains("the verifier rejected egress"), "{capability:?}");
    assert!(capability.evidence.contains("R0"), "{capability:?}");
    assert!(capability.action.contains("verifier"), "{capability:?}");
    // The probe left no program of the set attached: its pins and cgroup are gone.
    let pins = std::path::Path::new("/sys/fs/bpf/pryx").join(format!("probe-{}", std::process::id()));
    assert!(!pins.exists(), "{} was left behind", pins.display());
    let manager = manager_on(dir.path(), Arc::new(runtime), &extra);
    manager.refresh_host_probe().await;
    assert!(!manager.features().iter().any(|feature| feature == "egress-addresses"));
    let policy = serde_json::json!({ "egressAddresses": { "allow": [{ "prefix": "192.0.2.1/32" }] } });
    let refused = manager.ensure(id("refused"), sandbox(policy), Precondition::None, None).await.unwrap_err();
    assert!(format!("{refused:?}").contains("kernel-network-policy"), "{refused:?}");
}
