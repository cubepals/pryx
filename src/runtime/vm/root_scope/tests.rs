// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real kernel descriptor ownership and separately bounded root helper acceptance.
//! The root fixture proves namespace gateway adoption and lease retirement, not guest packets.
use super::*;
use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags};
use std::time::Duration;
use std::{
    io::IoSlice,
    mem::MaybeUninit,
    os::{
        fd::AsFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
};

#[test]
fn rejected_rights_close_every_delivered_file() {
    let file = tempfile::tempfile().unwrap();
    let identity = file.metadata().unwrap();
    let count = || {
        fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| fs::metadata(entry.path()).ok())
            .filter(|meta| (meta.dev(), meta.ino()) == (identity.dev(), identity.ino()))
            .count()
    };
    let before = count();
    let metadata = Advertisement {
        host_firewall: false,
        bridge_host_firewall: false,
        bridge_descriptors: Vec::new(),
        bridge_agent_argument: String::new(),
        bridge_packet_argument: String::new(),
        descriptors: vec![handoff::Role::Namespace],
        max_descriptors: 1,
        max_metadata_bytes: 64,
        version: 0,
    };
    for truncated in [true, false] {
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        let files = vec![file.as_fd(); if truncated { 4 } else { 1 }];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(4))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        assert!(control.push(SendAncillaryMessage::ScmRights(&files)));
        rustix::net::sendmsg(
            &sender,
            &[IoSlice::new(uuid::Uuid::new_v4().as_bytes())],
            &mut control,
            SendFlags::empty(),
        )
        .unwrap();
        assert!(handoff::receive(&receiver, &metadata).is_err());
        assert_eq!(count(), before, "Rejected SCM_RIGHTS leaked file references");
    }
}

#[tokio::test]
#[ignore = "requires actual root/TUN and the pinned scope/CNI programs; optional VM CI supplies them"]
async fn actual_root_gateway_adoption_and_lease_retirement() {
    assert!(rustix::process::geteuid().is_root());
    let _ = rustls::crypto::ring::default_provider().install_default();
    let before = fs::read_link("/proc/self/ns/net").unwrap();
    let root = tempfile::tempdir().unwrap();
    let lease_path = root.path().join("monitor.lock");
    let lease = fs::OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&lease_path).unwrap();
    rustix::fs::flock(&lease, rustix::fs::FlockOperation::LockExclusive).unwrap();
    let program = PathBuf::from(std::env::var_os("PRYX_VM_ROOT_SCOPE_TEST_PROGRAM").unwrap());
    let mut scope = Scope::start(
        Paths {
            program: &program,
            plugins: program.parent().unwrap(),
            boots: root.path(),
            lease: &lease_path,
            owner: (
                std::env::var("PRYX_VM_ROOT_TEST_UID").unwrap().parse().unwrap(),
                std::env::var("PRYX_VM_ROOT_TEST_GID").unwrap().parse().unwrap(),
            ),
        },
        EgressPolicy { allow: Vec::new(), deny: Vec::new() },
        8,
        Instant::now() + Duration::from_secs(30),
    )
    .await
    .unwrap();
    let namespace = scope.namespace().unwrap();
    assert_ne!(fs::read_link(&namespace).unwrap(), before);
    let http = format!("http://{}:{}/", scope.profile()["host"].as_str().unwrap(), scope.ports().0);
    let client = Command::new("/usr/bin/python3")
        .args([
            "-c",
            r#"
import os,sys,urllib.request,urllib.error
with open(sys.argv[1], 'rb') as namespace:
    os.setns(namespace.fileno(), os.CLONE_NEWNET)
request=urllib.request.Request(sys.argv[2], headers={'Host':sys.argv[3]})
try:
    urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=5)
    raise RuntimeError('Unexpectedly admitted a denied name')
except urllib.error.HTTPError as response:
    print(response.code)
"#,
            namespace.to_str().unwrap(),
            &http,
            &format!("{}.pryx.invalid", uuid::Uuid::new_v4()),
        ])
        .env_clear()
        .output()
        .await
        .unwrap();
    assert!(client.status.success(), "{}", String::from_utf8_lossy(&client.stderr));
    assert_eq!(String::from_utf8_lossy(&client.stdout).trim(), hyper::StatusCode::FORBIDDEN.as_str());
    assert_eq!(fs::read_link("/proc/self/ns/net").unwrap(), before);
    let mut stop = Box::pin(scope.stop());
    assert!(tokio::time::timeout(Duration::from_millis(200), &mut stop).await.is_err());
    rustix::fs::flock(&lease, rustix::fs::FlockOperation::Unlock).unwrap();
    tokio::time::timeout(Duration::from_secs(10), &mut stop).await.unwrap().unwrap();
    drop(stop);
    assert!(scope.namespace().is_err());
    assert_eq!(fs::read_link("/proc/self/ns/net").unwrap(), before);
    bridge_adoption(root.path(), &program, &lease, &before).await;
}

async fn bridge_adoption(root: &Path, program: &Path, lease: &fs::File, before: &Path) {
    let lease_path = root.join("monitor.lock");
    rustix::fs::flock(&lease, rustix::fs::FlockOperation::LockExclusive).unwrap();
    let programs = program.parent().unwrap();
    let mut bridge = Scope::start_bridge(
        Paths {
            program: &program,
            plugins: programs,
            boots: root,
            lease: &lease_path,
            owner: (
                std::env::var("PRYX_VM_ROOT_TEST_UID").unwrap().parse().unwrap(),
                std::env::var("PRYX_VM_ROOT_TEST_GID").unwrap().parse().unwrap(),
            ),
        },
        BridgePrograms { agent: &programs.join("bridge-agent"), packet: &programs.join("bridge-packet") },
        Instant::now() + Duration::from_secs(30),
    )
    .await
    .unwrap();
    assert_ne!(fs::read_link(bridge.namespace().unwrap()).unwrap(), before);
    let queue = bridge.packet_queue().unwrap();
    assert_eq!(rustix::io::fcntl_getfd(&queue).unwrap(), rustix::io::FdFlags::CLOEXEC);
    assert!(!bridge.packet_tap().is_empty());
    assert_eq!(fs::read_link("/proc/self/ns/net").unwrap(), before);
    let mut stop = Box::pin(bridge.stop());
    assert!(tokio::time::timeout(Duration::from_millis(200), &mut stop).await.is_err());
    rustix::fs::flock(&lease, rustix::fs::FlockOperation::Unlock).unwrap();
    tokio::time::timeout(Duration::from_secs(10), &mut stop).await.unwrap().unwrap();
    drop(stop);
    assert!(bridge.packet_queue().is_err());
    assert!(bridge.namespace().is_err());
    assert_eq!(fs::read_link("/proc/self/ns/net").unwrap(), before);
}
