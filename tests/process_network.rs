// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The process tier's kernel network policy with real supervisors, programs and destinations.
//! The ignored cases need a host that grants it: CI runs them as root and as an ordinary user
//! with a delegated cgroup, one at a time, with `PRYX_TEST_AGENT` naming a trusted agent,
//! `PRYX_TEST_PRIVATE` a private address with two listening ports, and `PRYX_TEST_REMOTE` a
//! machine elsewhere (another network namespace) that dials this host at `PRYX_TEST_EDGE` when
//! asked. The case that runs everywhere holds a host without the capability to its refusal.
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
    remove(&manager, &["public", "neighbour"]).await;
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
