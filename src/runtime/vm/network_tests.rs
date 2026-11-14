// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Actual confined packet helper acceptance without a KVM dependency. The caller stages
//! verified programs; real sockets and kernel leases check cancellation and refusal cleanup.
use super::*;
use std::{
    fs::OpenOptions,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    time::Duration,
};
use tokio::net::{TcpStream, UnixStream};

fn inputs(root: &Path) -> (PathBuf, PathBuf) {
    let stage = |variable, name| {
        let source = std::env::var_os(variable).expect("Stage the actual pinned program");
        let destination = root.join(name);
        fs::copy(source, &destination).unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o500)).unwrap();
        destination
    };
    (stage("PRYX_VM_HOST_LAUNCHER", "launcher"), stage("PRYX_VM_HOST_PROXY", "packet-helper"))
}
fn layout(root: &Path) -> (PathBuf, PathBuf) {
    let boots = root.join("boots");
    let leases = root.join("leases");
    for path in [&boots, &leases] {
        fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    (boots, leases.join("packet.lock"))
}
fn policy() -> Option<EgressPolicy> {
    Some(EgressPolicy { allow: vec![], deny: vec![] })
}
fn lease_held(path: &Path) -> bool {
    let lease = OpenOptions::new().read(true).write(true).open(path).unwrap();
    match rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => false,
        Err(problem) if problem == rustix::io::Errno::WOULDBLOCK => true,
        Err(problem) => panic!("Cannot inspect packet lease: {problem}"),
    }
}
fn bind(port: u16) -> io::Result<tokio::net::TcpListener> {
    let socket = tokio::net::TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    socket.bind((Ipv4Addr::LOCALHOST, port).into())?;
    socket.listen(1)
}
async fn retired(directory: &Path, lease: &Path, ports: (u16, u16, u16)) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if !directory.exists() && !lease_held(lease) {
            let tcp = [ports.0, ports.1, ports.2].map(bind);
            if tcp.iter().all(Result::is_ok) && std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, ports.2)).is_ok() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "Packet helper retained its files, lease or gateways");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
#[ignore = "requires the staged actual launcher and maintained packet helper"]
async fn actual_helper_and_gateways_retire_after_cancelled_stop_and_drop() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let root = tempfile::tempdir().unwrap();
    let (launcher, program) = inputs(root.path());
    let (boots, lease) = layout(root.path());
    for external in [false, true] {
        for cancel_stop in [true, false] {
            let external_directory = tempfile::Builder::new()
                .prefix("guest-")
                .permissions(fs::Permissions::from_mode(0o700))
                .tempdir_in(&boots)
                .unwrap();
            let packet = external.then(|| external_directory.path().join("packet"));
            let mut network = Network::start(
                Paths {
                    launcher: &launcher,
                    program: &program,
                    boots: &boots,
                    lease: &lease,
                    socket: packet.as_deref(),
                },
                policy(),
                2,
                Instant::now() + Duration::from_secs(5),
            )
            .await
            .unwrap_or_else(|problem| {
                panic!("External packet path={external}, cancelled stop={cancel_stop}: {problem}")
            });
            let directory = network.resources.as_ref().unwrap()._directory.path().to_path_buf();
            let packet_path = network.socket().to_owned();
            let ports = network.ports();
            assert!(network.exited().unwrap().is_none());
            assert!(lease_held(&lease));
            assert_eq!(fs::metadata(network.socket()).unwrap().mode() & 0o777, 0o600);
            assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o777, 0o700);
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let response = client
                .get(format!("http://{}:{}/", Ipv4Addr::LOCALHOST, ports.0))
                .header(hyper::header::HOST, "denied.pryx.invalid")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), hyper::StatusCode::FORBIDDEN);
            let packet = UnixStream::connect(network.socket()).await.unwrap();
            let http = TcpStream::connect((Ipv4Addr::LOCALHOST, ports.0)).await.unwrap();
            let tls = TcpStream::connect((Ipv4Addr::LOCALHOST, ports.1)).await.unwrap();
            if cancel_stop {
                let mut stop = Box::pin(network.stop());
                // Poll exactly once to start owned retirement, then cancel the caller's wait.
                std::future::poll_fn(|context| {
                    let _ = std::future::Future::poll(stop.as_mut(), context);
                    std::task::Poll::Ready(())
                })
                .await;
                drop(stop);
                network.stop().await.unwrap();
                network.stop().await.unwrap();
                assert!(!lease_held(&lease));
                assert!(!directory.exists());
            }
            drop(network);
            retired(&directory, &lease, ports).await;
            assert!(UnixStream::connect(packet_path).await.is_err());
            drop((packet, http, tls, response, client));
        }
    }
}

#[tokio::test]
#[ignore = "requires the staged actual launcher and maintained packet helper"]
async fn occupied_helper_lease_refuses_and_retires_only_new_gateways() {
    let root = tempfile::tempdir().unwrap();
    let (launcher, program) = inputs(root.path());
    let (boots, lease) = layout(root.path());
    let mut first = Network::start(
        Paths { launcher: &launcher, program: &program, boots: &boots, lease: &lease, socket: None },
        policy(),
        2,
        Instant::now() + Duration::from_secs(5),
    )
    .await
    .unwrap();
    let socket = first.socket().to_path_buf();
    let result = Network::start(
        Paths { launcher: &launcher, program: &program, boots: &boots, lease: &lease, socket: None },
        policy(),
        2,
        Instant::now() + Duration::from_secs(5),
    )
    .await;
    assert!(result.is_err());
    assert!(first.exited().unwrap().is_none());
    assert!(UnixStream::connect(&socket).await.is_ok());
    let deadline = Instant::now() + Duration::from_secs(2);
    while fs::read_dir(&boots).unwrap().count() != 1 {
        assert!(Instant::now() < deadline, "Refused helper retained its private directory");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    first.stop().await.unwrap();
    assert!(fs::read_dir(&boots).unwrap().next().is_none());
    assert!(!lease_held(&lease));
}
