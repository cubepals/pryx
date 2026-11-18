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

async fn refused_proxy(plan: &Plan) {
    let proxy = launch(plan).await;
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(5), proxy.wait_with_output()).await.unwrap().unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty(), "A refused packet helper must not publish readiness");
    if let Role::PacketProxy { socket, .. } = &plan.role
        && let Err(error) = fs::remove_file(socket)
    {
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }
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
import ctypes,errno,fcntl,os,socket,sys,termios,time

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
denied(lambda: fcntl.ioctl(0,termios.FIOCLEX))
try: os.write(0,b'lease write must fail')
except OSError as error: assert error.errno==errno.EBADF
else: raise AssertionError('Inherited lease permitted writes')
null=os.open('/dev/null',os.O_RDWR)
denied(lambda: os.dup2(null,0))
os.close(null)
if len(sys.argv)>3:
    libc=ctypes.CDLL(None,use_errno=True)
    libc.syscall.restype=ctypes.c_long
    assert libc.syscall(int(sys.argv[4]))>0
    assert libc.syscall(int(sys.argv[3])|int(sys.argv[4]))==-1
    assert ctypes.get_errno()==errno.EPERM
with open('owned','w') as output: output.write('allowed')
print('confined and lease retained',flush=True)
time.sleep(30)
"#;
    let arguments = vec!["-c".into(), script.into(), secret.display().to_string(), std::process::id().to_string()];
    #[cfg(target_arch = "x86_64")]
    let arguments = arguments
        .into_iter()
        .chain([linux_raw_sys::general::__X32_SYSCALL_BIT.to_string(), libc::SYS_getpid.to_string()])
        .collect();
    let plan = Plan {
        program,
        arguments,
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
import errno,fcntl,os,socket,sys,termios,time

def denied(call):
    try: call()
    except PermissionError as error: assert error.errno in (errno.EACCES,errno.EPERM)
    else: raise AssertionError('Packet proxy escaped host policy')

denied(lambda: open(sys.argv[1]).read())
denied(lambda: os.kill(int(sys.argv[2]),0))
denied(lambda: fcntl.ioctl(0,termios.FIOCLEX))
try: os.write(0,b'lease write must fail')
except OSError as error: assert error.errno==errno.EBADF
else: raise AssertionError('Inherited packet lease permitted writes')
for family in (socket.AF_UNIX,socket.AF_VSOCK):
    denied(lambda: socket.socket(family,socket.SOCK_STREAM))
for protocol in (socket.NETLINK_USERSOCK,socket.NETLINK_XFRM,socket.NETLINK_CRYPTO):
    denied(lambda: socket.socket(socket.AF_NETLINK,socket.SOCK_RAW,protocol))
# Routing netlink alone, which admission asks how a destination is routed; never bound.
route=socket.socket(socket.AF_NETLINK,socket.SOCK_RAW,socket.NETLINK_ROUTE)
denied(lambda: route.bind((0,0)))
route.close()
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

#[tokio::test]
#[ignore = "requires the separately built pinned Go packet helper; optional VM CI supplies it"]
async fn actual_packet_proxy_serves_its_inherited_private_socket() {
    use std::{collections::BTreeSet, os::unix::fs::PermissionsExt, time::Duration};
    use tokio::io::AsyncReadExt;
    assert!(!rustix::process::geteuid().is_root());
    let _ = rustls::crypto::ring::default_provider().install_default();
    let root = tempfile::tempdir().unwrap();
    // The runner's download/build directory can have writable ancestors. Exercise
    // the actual launcher only after staging into the same private boundary as an install.
    let source = PathBuf::from(std::env::var_os("PRYX_VM_HOST_PROXY").expect("Stage the pinned packet helper"));
    let program = root.path().join("network-proxy");
    fs::copy(source, &program).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o500)).unwrap();
    let metadata = std::process::Command::new(&program).arg("--metadata").env_clear().output().unwrap();
    assert!(metadata.status.success());
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout).unwrap();
    let profile = &metadata["production"];
    let work = root.path().join("proxy");
    let locks = root.path().join("locks");
    for path in [&work, &locks] {
        fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    let configuration = work.join("network.json");
    let non_public: Vec<String> = pryx_agent::transfer_target::non_public_ipv4_networks()
        .map(|(address, bits)| format!("{address}/{bits}"))
        .collect();
    let valid = serde_json::json!({
        "parent":std::process::id(),"host":std::net::Ipv4Addr::LOCALHOST.to_string(),"concurrency":2,
        "egress":{"public":true,"tcp":[5353],"udp":[5353],"nonPublic":non_public}
    });
    fs::write(&configuration, serde_json::to_vec(&valid).unwrap()).unwrap();
    fs::set_permissions(&configuration, fs::Permissions::from_mode(0o600)).unwrap();
    // Names are resolved by the independent policy gateways, never this packet helper.
    let read = vec![configuration.clone()];
    let socket = work.join("packet");
    let plan = Plan {
        program,
        arguments: vec![profile["configurationArgument"].as_str().unwrap().into(), configuration.display().to_string()],
        directory: work,
        lock: locks.join("packet.lock"),
        parent: std::process::id(),
        read,
        write: Vec::new(),
        role: Role::PacketProxy {
            socket: socket.clone(),
            descriptor_argument: profile["descriptorArgument"].as_str().unwrap().into(),
        },
    };
    let mut proxy = launch(&plan).await;
    let output = proxy.stdout.take().unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        output.take(pryx_agent::contract::MAX_REQUEST_BYTES as u64 + 1).read_to_end(&mut bytes),
    )
    .await
    .unwrap()
    .unwrap();
    if bytes.is_empty() {
        let result = proxy.wait_with_output().await.unwrap();
        panic!("Actual packet helper exited: {}", String::from_utf8_lossy(&result.stderr));
    }
    assert!(bytes.len() <= pryx_agent::contract::MAX_REQUEST_BYTES);
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(), *profile);
    assert!(proxy.try_wait().unwrap().is_none());
    let owned = procfs::process::Process::new(proxy.id().unwrap() as i32)
        .unwrap()
        .fd()
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter_map(|entry| match entry.target {
            procfs::process::FDTarget::Socket(inode) => Some(inode),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let tcp = procfs::net::tcp().unwrap().into_iter().map(|entry| entry.inode);
    let udp = procfs::net::udp().unwrap().into_iter().map(|entry| entry.inode);
    assert!(!tcp.chain(udp).any(|inode| owned.contains(&inode)), "Packet helper opened a host IPv4 socket at startup");
    let client = reqwest::Client::builder()
        .no_proxy()
        .unix_socket(socket.as_path())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    for (method, path) in [
        (reqwest::Method::GET, "/"),
        (reqwest::Method::POST, metadata["exposePath"].as_str().unwrap()),
        (reqwest::Method::GET, metadata["statsPath"].as_str().unwrap()),
    ] {
        let response = client.request(method, format!("http://packet{path}")).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    }
    let lease = fs::OpenOptions::new().read(true).write(true).open(&plan.lock).unwrap();
    assert_eq!(
        rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap_err(),
        rustix::io::Errno::WOULDBLOCK
    );
    proxy.kill().await.unwrap();
    proxy.wait().await.unwrap();
    rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    drop(lease);
    drop(client);
    fs::remove_file(&socket).unwrap();
    for (field, value) in [
        ("parent", serde_json::json!(0)),
        ("host", serde_json::json!(std::net::Ipv4Addr::BROADCAST.to_string())),
        ("concurrency", serde_json::json!(0)),
        ("extra", serde_json::json!(true)),
        // Public egress needs the boundary it must never dial, and a gateway port is never zero.
        ("egress", serde_json::json!({"public":true,"tcp":[],"udp":[],"nonPublic":[]})),
        ("egress", serde_json::json!({"public":false,"tcp":[0],"udp":[],"nonPublic":[]})),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        fs::write(&configuration, serde_json::to_vec(&invalid).unwrap()).unwrap();
        refused_proxy(&plan).await;
    }
    let mut trailing = serde_json::to_vec(&valid).unwrap();
    trailing.extend_from_slice(b"{}");
    fs::write(&configuration, trailing).unwrap();
    refused_proxy(&plan).await;
    let limit = usize::try_from(profile["maxConfigurationBytes"].as_u64().unwrap()).unwrap();
    fs::write(&configuration, vec![b' '; limit + 1]).unwrap();
    refused_proxy(&plan).await;
    fs::write(&configuration, serde_json::to_vec(&valid).unwrap()).unwrap();
    fs::set_permissions(&configuration, fs::Permissions::from_mode(0o644)).unwrap();
    refused_proxy(&plan).await;
}

#[tokio::test]
async fn packet_tap_rejects_a_regular_carrier_without_writing_it() {
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    let locks = root.path().join("locks");
    for path in [&work, &locks] {
        fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    let proof = root.path().as_os_str().as_encoded_bytes();
    let carrier = root.path().join("carrier");
    fs::write(&carrier, proof).unwrap();
    let file = fs::OpenOptions::new().read(true).write(true).open(&carrier).unwrap();
    let python = std::process::Command::new("python3")
        .args(["-c", "import os,sys; print(os.path.realpath(sys.executable))"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let plan = Plan {
        program: String::from_utf8(python.stdout).unwrap().trim().into(),
        arguments: vec!["-c".into(), "print('unexpected execution', flush=True)".into()],
        directory: work,
        lock: locks.join("packet.lock"),
        parent: std::process::id(),
        read: ["/usr", "/bin", "/lib", "/lib64", "/etc/ld.so.cache"]
            .into_iter()
            .filter_map(|path| fs::canonicalize(path).ok())
            .collect(),
        write: vec![],
        role: Role::PacketTap { descriptor_argument: "--queue".into() },
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_pryx-agent"))
        .arg(MODE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(file))
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(&serde_json::to_vec(&plan).unwrap()).await.unwrap();
    input.shutdown().await.unwrap();
    drop(input);
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output()).await.unwrap().unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert_eq!(fs::read(carrier).unwrap(), proof);
}

#[tokio::test]
async fn filesystem_tool_reads_only_its_inputs_with_no_socket_capability_or_large_file() {
    let root = tempfile::tempdir().unwrap();
    let (work, locks) = (root.path().join("work"), root.path().join("locks"));
    for path in [&work, &locks] {
        fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    let (input, output, secret) = (root.path().join("disk"), root.path().join("copy"), root.path().join("outside"));
    let configuration = root.path().join("mke2fs.conf");
    fs::write(&input, "guest-written image").unwrap();
    fs::write(&configuration, "[defaults]\n").unwrap();
    fs::write(&secret, "must stay outside the tool grant").unwrap();
    for path in [&input, &configuration, &secret] {
        fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    }
    fs::OpenOptions::new().write(true).create_new(true).open(&output).unwrap();
    fs::set_permissions(&output, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    let program = std::process::Command::new("python3")
        .args(["-c", "import os,sys; print(os.path.realpath(sys.executable))"])
        .output()
        .unwrap();
    let program = PathBuf::from(String::from_utf8(program.stdout).unwrap().trim());
    let mut read: Vec<PathBuf> = ["/usr", "/bin", "/lib", "/lib64", "/etc/ld.so.cache"]
        .into_iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .collect();
    read.extend([input.clone(), configuration.clone()]);
    let script = r#"
import errno,os,resource,socket,sys

def denied(call, codes=(errno.EACCES,errno.EPERM)):
    try: call()
    except OSError as error: assert error.errno in codes, error
    else: raise AssertionError('Filesystem tool escaped its policy')

assert open(sys.argv[1]).read()=='guest-written image'
assert os.environ.get('MKE2FS_CONFIG')==sys.argv[4] and open(sys.argv[4]).read()=='[defaults]\n'
denied(lambda: open(sys.argv[3]).read())
denied(lambda: open(sys.argv[1],'a'))
for domain in (socket.AF_UNIX,socket.AF_INET,socket.AF_INET6):
    denied(lambda: socket.socket(domain,socket.SOCK_STREAM))
denied(lambda: socket.socketpair())
assert resource.getrlimit(resource.RLIMIT_CORE)==(0,0)
for line in open('/proc/self/status'):
    if line.startswith(('CapEff:','CapPrm:','CapAmb:')): assert int(line.split()[1],16)==0, line
with open(sys.argv[2],'wb',buffering=0) as copy:
    copy.write(b'x'*4096)
    denied(lambda: copy.write(b'x'), (errno.EFBIG,))
print('confined filesystem tool',flush=True)
"#;
    let plan = Plan {
        program,
        arguments: vec![
            "-c".into(),
            script.into(),
            input.display().to_string(),
            output.display().to_string(),
            secret.display().to_string(),
            configuration.display().to_string(),
        ],
        directory: work,
        lock: locks.join("tool.lock"),
        parent: std::process::id(),
        read,
        write: vec![output.clone()],
        role: Role::FilesystemTool { file_bytes: 4096, configuration: Some(configuration) },
    };
    let tool = launch(&plan).await;
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(10), tool.wait_with_output()).await.unwrap().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "confined filesystem tool");
    assert_eq!(fs::metadata(&output).unwrap().len(), 4096);
}

#[tokio::test]
async fn filesystem_tool_bound_admits_an_erofs_device_reservation_and_nothing_past_it() {
    let root = tempfile::tempdir().unwrap();
    let (work, locks) = (root.path().join("work"), root.path().join("locks"));
    for path in [&work, &locks] {
        fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    let output = root.path().join("layer.erofs");
    fs::OpenOptions::new().write(true).create_new(true).open(&output).unwrap();
    fs::set_permissions(&output, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    let program = std::process::Command::new("python3")
        .args(["-c", "import os,sys; print(os.path.realpath(sys.executable))"])
        .output()
        .unwrap();
    let program = PathBuf::from(String::from_utf8(program.stdout).unwrap().trim());
    let read = ["/usr", "/bin", "/lib", "/lib64", "/etc/ld.so.cache"]
        .into_iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .collect();
    // erofs-utils' mkfs reserves its whole 2 TiB address space, then shrinks to the image.
    let reservation: u64 = 1 << 41;
    let script = r#"
import errno,os,sys
size=int(sys.argv[2])
with open(sys.argv[1],'r+b',buffering=0) as image:
    os.ftruncate(image.fileno(),size)
    try: os.ftruncate(image.fileno(),size+1)
    except OSError as error: assert error.errno==errno.EFBIG, error
    else: raise AssertionError('The written-file bound was not enforced')
    os.ftruncate(image.fileno(),0)
    image.write(b'e'*65536)
print('reserved and shrunk',flush=True)
"#;
    let plan = Plan {
        program,
        arguments: vec!["-c".into(), script.into(), output.display().to_string(), reservation.to_string()],
        directory: work,
        lock: locks.join("tool.lock"),
        parent: std::process::id(),
        read,
        write: vec![output.clone()],
        role: Role::FilesystemTool { file_bytes: reservation, configuration: None },
    };
    let tool = launch(&plan).await;
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(10), tool.wait_with_output()).await.unwrap().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "reserved and shrunk");
    assert_eq!(fs::metadata(&output).unwrap().len(), 65536);
}
