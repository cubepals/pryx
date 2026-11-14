// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Kernel confinement around an actual inherited CNI packet queue in the bounded root fixture.
//! The ordinary test process receives the queue on stderr and restores diagnostics before
//! running the real launcher. No guest packet forwarding or production worker is inferred.
use pryx_agent::{
    bootstrap_capabilities::Capabilities,
    vm_monitor::{MODE, Plan, Role, TUN_DEVICE},
};
use std::{
    fs,
    net::{Ipv4Addr, SocketAddr},
    os::{fd::OwnedFd, unix::fs::DirBuilderExt},
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    process::Command,
};

const READY: &str = "attached and confined";

fn packet_plan(root: &Path, port: u16) -> Plan {
    let work = root.join("work");
    let locks = root.join("locks");
    for path in [&work, &locks] {
        fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    let outside = root.join("outside");
    fs::write(&outside, root.as_os_str().as_encoded_bytes()).unwrap();
    let python = std::process::Command::new("python3")
        .args(["-c", "import os,sys; print(os.path.realpath(sys.executable))"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let script = r#"
import errno,fcntl,os,socket,stat,sys,time

def denied(call):
    try: call()
    except PermissionError as error: assert error.errno in (errno.EACCES,errno.EPERM)
    else: raise AssertionError('Host operation escaped packet policy')

queue=int(sys.argv[-1])
assert stat.S_ISCHR(os.fstat(queue).st_mode)
assert os.fstat(queue).st_rdev==os.stat(sys.argv[5]).st_rdev
assert fcntl.fcntl(queue,fcntl.F_GETFD)&fcntl.FD_CLOEXEC==0
denied(lambda: open(sys.argv[1]).read())
denied(lambda: os.kill(int(sys.argv[2]),0))
denied(lambda: socket.socket(socket.AF_UNIX,socket.SOCK_STREAM))
client=socket.socket(socket.AF_INET,socket.SOCK_STREAM)
denied(lambda: client.bind((sys.argv[3],0)))
client.connect((sys.argv[3],int(sys.argv[4])))
client.close()
denied(lambda: os.close(0))
denied(lambda: fcntl.flock(0,fcntl.LOCK_UN))
with open('owned','w') as output: output.write('allowed')
print(sys.argv[6],flush=True)
time.sleep(30)
"#;
    Plan {
        program: String::from_utf8(python.stdout).unwrap().trim().into(),
        arguments: vec![
            "-c".into(),
            script.into(),
            outside.display().to_string(),
            std::process::id().to_string(),
            Ipv4Addr::LOCALHOST.to_string(),
            port.to_string(),
            TUN_DEVICE.into(),
            READY.into(),
        ],
        directory: work,
        lock: locks.join("packet.lock"),
        parent: std::process::id(),
        read: ["/usr", "/bin", "/lib", "/lib64", "/etc/ld.so.cache"]
            .into_iter()
            .filter_map(|path| fs::canonicalize(path).ok())
            .collect(),
        write: vec![],
        role: Role::PacketTap { descriptor_argument: "--queue".into() },
    }
}

#[tokio::test]
#[ignore = "actual root CNI queue supplied by the bounded authority"]
async fn attached_queue_inherits_host_confinement_and_lease() {
    let queue: OwnedFd = rustix::io::dup(rustix::stdio::stderr()).unwrap();
    rustix::stdio::dup2_stderr(rustix::stdio::stdout()).unwrap();
    assert!(!rustix::process::geteuid().is_root(), "Run packet confinement as the fixed ordinary owner");
    let agent = PathBuf::from(std::env::var_os("PRYX_VM_PACKET_TEST_AGENT").expect("actual copied static agent"));
    let metadata = std::process::Command::new(&agent).arg(pryx_agent::bootstrap_capabilities::PROBE).output().unwrap();
    assert!(metadata.status.success());
    let capabilities: Capabilities = serde_json::from_slice(&metadata.stdout).unwrap();
    assert!(capabilities.vm_packet_tap, "The actual agent must advertise the new launch role");
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await.unwrap();
    let plan = packet_plan(root.path(), listener.local_addr().unwrap().port());
    let mut child = Command::new(agent)
        .arg(MODE)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(queue))
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(&serde_json::to_vec(&plan).unwrap()).await.unwrap();
    input.shutdown().await.unwrap();
    drop(input);
    let output = child.stdout.take().unwrap();
    let mut output = tokio::io::BufReader::new(output);
    let mut line = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), output.read_line(&mut line)).await.unwrap().unwrap();
    assert_eq!(line.trim(), READY);
    tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept()).await.unwrap().unwrap();
    assert_eq!(fs::read_to_string(plan.directory.join("owned")).unwrap(), "allowed");
    let lease = fs::OpenOptions::new().read(true).write(true).open(&plan.lock).unwrap();
    assert_eq!(
        rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap_err(),
        rustix::io::Errno::WOULDBLOCK
    );
    child.kill().await.unwrap();
    rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
}
