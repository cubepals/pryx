// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real host kernel controls around the monitor launcher, using a disposable Python process.
//! These checks prove confinement and lease lifetime; real Firecracker/jailer/KVM acceptance
//! remains in the VM workflow and is never inferred from this ordinary process fixture.
use pryx_agent::vm_monitor::{MODE, Plan, Role};
use std::{fs, os::unix::fs::DirBuilderExt, path::PathBuf, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    process::{Child, Command},
};

async fn launch(plan: &Plan) -> Child {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pryx-agent"))
        .arg(MODE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(&serde_json::to_vec(plan).unwrap()).await.unwrap();
    input.shutdown().await.unwrap();
    child
}

#[tokio::test]
async fn confined_host_process_cannot_escape_or_release_its_lease() {
    assert!(!rustix::process::geteuid().is_root(), "Run ordinary-user monitor checks without root");
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    let locks = root.path().join("locks");
    for path in [&work, &locks] {
        fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    let secret = root.path().join("outside");
    fs::write(&secret, "must stay outside the monitor grant").unwrap();
    let program = std::process::Command::new("python3")
        .args(["-c", "import os,sys; print(os.path.realpath(sys.executable))"])
        .output()
        .unwrap();
    assert!(program.status.success());
    let program = PathBuf::from(String::from_utf8(program.stdout).unwrap().trim());
    let read = ["/usr", "/bin", "/lib", "/lib64", "/etc/ld.so.cache"]
        .into_iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .collect();
    let script = r#"
import errno,fcntl,os,socket,sys,time

def denied(call):
    try: call()
    except PermissionError as error: assert error.errno in (errno.EACCES,errno.EPERM)
    else: raise AssertionError('Host operation escaped monitor policy')

denied(lambda: open(sys.argv[1]).read())
denied(lambda: os.kill(int(sys.argv[2]),0))
denied(lambda: socket.socket(socket.AF_INET,socket.SOCK_STREAM))
denied(lambda: os.close(0))
denied(lambda: fcntl.flock(0,fcntl.LOCK_UN))
denied(lambda: fcntl.fcntl(0,fcntl.F_SETFD,fcntl.FD_CLOEXEC))
null=os.open('/dev/null',os.O_RDWR)
denied(lambda: os.dup2(null,0))
os.close(null)
with open('owned','w') as output: output.write('allowed')
print('confined and lease retained',flush=True)
time.sleep(30)
"#;
    let plan = Plan {
        program,
        arguments: vec!["-c".into(), script.into(), secret.display().to_string(), std::process::id().to_string()],
        directory: work.clone(),
        lock: locks.join("monitor.lock"),
        parent: std::process::id(),
        read,
        write: vec![],
        role: Role::Ordinary,
    };
    let mut monitor = launch(&plan).await;
    let mut output = tokio::io::BufReader::new(monitor.stdout.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), output.read_line(&mut line)).await.unwrap().unwrap();
    if line.is_empty() {
        let result = monitor.wait_with_output().await.unwrap();
        panic!("Confinement fixture exited: {}", String::from_utf8_lossy(&result.stderr));
    }
    assert_eq!(line.trim(), "confined and lease retained");
    assert_eq!(fs::read_to_string(work.join("owned")).unwrap(), "allowed");
    let lease = fs::OpenOptions::new().read(true).write(true).open(&plan.lock).unwrap();
    assert_eq!(
        rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap_err(),
        rustix::io::Errno::WOULDBLOCK
    );
    let competing = launch(&plan).await.wait_with_output().await.unwrap();
    assert!(!competing.status.success());
    assert!(competing.stdout.is_empty());
    monitor.kill().await.unwrap();
    monitor.wait().await.unwrap();
    rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
}

#[tokio::test]
async fn packet_proxy_inherits_only_its_listener_and_cannot_bind() {
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncReadExt;
    assert!(!rustix::process::geteuid().is_root());
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("proxy");
    let locks = root.path().join("locks");
    for path in [&work, &locks] {
        fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    let secret = root.path().join("outside");
    fs::write(&secret, "outside the packet proxy grant").unwrap();
    let program = std::process::Command::new("python3")
        .args(["-c", "import os,sys; print(os.path.realpath(sys.executable))"])
        .output()
        .unwrap();
    assert!(program.status.success());
    let program = PathBuf::from(String::from_utf8(program.stdout).unwrap().trim());
    let read = ["/usr", "/bin", "/lib", "/lib64", "/etc/ld.so.cache"]
        .into_iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .collect();
    let peer = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let script = r#"
import errno,os,socket,sys,time

def denied(call):
    try: call()
    except PermissionError as error: assert error.errno in (errno.EACCES,errno.EPERM)
    else: raise AssertionError('Packet proxy escaped host policy')

denied(lambda: open(sys.argv[1]).read())
denied(lambda: os.kill(int(sys.argv[2]),0))
for family in (socket.AF_UNIX,socket.AF_VSOCK,socket.AF_NETLINK):
    denied(lambda: socket.socket(family,socket.SOCK_STREAM))
denied(lambda: socket.socketpair())
client=socket.socket(socket.AF_INET,socket.SOCK_STREAM)
denied(lambda: client.bind(('127.0.0.1',0)))
denied(lambda: client.listen(1))
client.connect(('127.0.0.1',int(sys.argv[3])))
client.sendall(b'outbound')
listener=socket.socket(fileno=int(sys.argv[-1]))
print('packet proxy confined',flush=True)
accepted,_=listener.accept()
assert accepted.recv(64)==b'inbound'
accepted.sendall(b'relayed')
time.sleep(30)
"#;
    let socket = work.join("packet");
    let plan = Plan {
        program,
        arguments: vec![
            "-c".into(),
            script.into(),
            secret.display().to_string(),
            std::process::id().to_string(),
            peer.local_addr().unwrap().port().to_string(),
        ],
        directory: work.clone(),
        lock: locks.join("packet.lock"),
        parent: std::process::id(),
        read,
        write: Vec::new(),
        role: Role::PacketProxy { socket: socket.clone(), descriptor_argument: "--inherited-listener".into() },
    };
    let mut proxy = launch(&plan).await;
    let mut output = tokio::io::BufReader::new(proxy.stdout.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), output.read_line(&mut line)).await.unwrap().unwrap();
    if line.is_empty() {
        let result = proxy.wait_with_output().await.unwrap();
        panic!("Packet fixture exited: {}", String::from_utf8_lossy(&result.stderr));
    }
    assert_eq!(line.trim(), "packet proxy confined");
    assert_eq!(fs::metadata(&socket).unwrap().permissions().mode() & 0o777, 0o600);
    let (mut connected, _) = peer.accept().await.unwrap();
    let mut bytes = [0; 8];
    connected.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"outbound");
    let mut connected = tokio::net::UnixStream::connect(&socket).await.unwrap();
    connected.write_all(b"inbound").await.unwrap();
    let mut bytes = [0; 7];
    connected.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"relayed");
    let lease = fs::OpenOptions::new().read(true).write(true).open(&plan.lock).unwrap();
    assert_eq!(
        rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap_err(),
        rustix::io::Errno::WOULDBLOCK
    );
    proxy.kill().await.unwrap();
    proxy.wait().await.unwrap();
    rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
}
