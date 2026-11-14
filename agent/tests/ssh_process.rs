// SPDX-License-Identifier: FSL-1.1-ALv2
//! Explicit actual OpenSSH command/PTY and same-UID privacy acceptance. Client keys enter
//! the maintained dedicated ssh-agent through stdin and never reach argv or disk.
use pryx_agent::{process_policy, process_tool_policy, process_tools::ssh};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ssh_key::{Algorithm, LineEnding, PrivateKey};
use std::{
    fs,
    io::{Read, Write},
    os::{
        linux::net::SocketAddrExt,
        unix::{
            fs::PermissionsExt,
            net::{SocketAddr, UnixStream},
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const INPUT: &str = "PRYX_NATIVE_SSH_WORKER";
#[derive(Clone, Deserialize, Serialize)]
struct Input {
    root: PathBuf,
    bundle: PathBuf,
    address: String,
    neighbour: bool,
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn policy(input: &Input) -> process_policy::Policy {
    process_policy::Policy {
        data: input.root.join("data"),
        memory_bytes: 512 * 1024 * 1024,
        address_space_bytes: 64 * 1024 * 1024 * 1024,
        open_files: 256,
        processes: 4096,
        cpu_seconds: 60,
        nice: 1,
        network_off: false,
        require_ipc_scope: Some(true),
        tools: Some(process_tool_policy::Policy {
            cache: input.bundle.clone(),
            downloads: input.root.join("downloads"),
            scratch: input.root.join("scratch"),
            artifact: None,
            unavailable: None,
            browser_on: true,
            desktop_on: None,
            desktop_unavailable: None,
        }),
    }
}
fn worker(input: &Input) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "native_ssh_worker", "--ignored", "--nocapture"])
        .env_clear()
        .env(INPUT, serde_json::to_string(input).unwrap());
    command
}
#[test]
#[ignore = "isolated subprocess entry for the actual native SSH acceptance"]
fn native_ssh_worker() {
    let input: Input = serde_json::from_str(&std::env::var(INPUT).unwrap()).unwrap();
    if input.neighbour {
        process_policy::enter(&policy(&input)).unwrap();
        assert!(fs::read(input.root.parent().unwrap().join("owner/scratch/ssh/host_key")).is_err());
        assert!(fs::read(input.root.parent().unwrap().join("owner/data/marker")).is_err());
        let error = UnixStream::connect_addr(&SocketAddr::from_abstract_name(&input.address).unwrap()).unwrap_err();
        fs::write(input.root.join("data/neighbour.json"),json!({"uid":rustix::process::geteuid().as_raw(),"ownerFilesDenied":true,"abstractDenied":error.to_string()}).to_string()).unwrap();
        return;
    }
    process_policy::scope().unwrap();
    let launch = ssh::Launch { directory: input.root.join("scratch/ssh"), address: input.address.clone() };
    let mut running = Running(
        ssh::start_at(&policy(&input), launch, &input.bundle, Path::new(env!("CARGO_BIN_EXE_pryx-agent"))).unwrap(),
    );
    process_policy::enter(&policy(&input)).unwrap();
    let deadline = Instant::now() + pryx_agent::contract::TOOL_STARTUP;
    while !input.root.join("scratch/ssh/ready").exists() {
        assert!(running.0.try_wait().unwrap().is_none() && Instant::now() < deadline, "Private SSH worker exited");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(UnixStream::connect_addr(&SocketAddr::from_abstract_name(&input.address).unwrap()).is_ok());
    fs::write(input.root.join("data/ready"), b"ready").unwrap();
    while !input.root.join("data/stop").exists() {
        std::thread::sleep(Duration::from_millis(20))
    }
}
fn quote(value: &Path) -> String {
    format!("'{}'", value.to_string_lossy().replace('\'', "'\\''"))
}
fn config_word(value: &Path) -> String {
    value.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"")
}
fn finish(mut child: Child) -> std::process::Output {
    let deadline = Instant::now() + pryx_agent::contract::TOOL_STARTUP;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("OpenSSH fixture deadline: {}", String::from_utf8_lossy(&output.stderr));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}
fn output(mut command: Command) -> std::process::Output {
    finish(command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap())
}
fn client(input: &Input, root: &Path, command: Option<&str>) -> Command {
    let mut process = Command::new(input.bundle.join("desktop/bin/ssh"));
    process
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("SSH_AUTH_SOCK", root.join("client-agent.sock"))
        .args([
            "-F",
            "/dev/null",
            "-o",
            "BatchMode=yes",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "GlobalKnownHostsFile=/dev/null",
        ])
        .arg("-o")
        .arg(format!("UserKnownHostsFile={}", root.join("known_hosts").display()))
        .arg("-o")
        .arg(format!(
            "ProxyCommand={} {} {}",
            quote(&input.bundle.join("bin/node")),
            quote(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ssh-unix-proxy.mjs")),
            input.address
        ))
        .arg("-i")
        .arg(root.join("client.pub"))
        .arg(if command.is_some() { "-T" } else { "-tt" })
        .arg("pryx@pryx-native");
    if let Some(command) = command {
        process.arg(command);
    }
    process
}
#[test]
#[ignore = "explicit complete tool bundle, namespaces and fully enforced private process policy"]
fn real_native_ssh_command_pty_and_same_uid_privacy() {
    assert!(!rustix::process::geteuid().is_root());
    assert!(pryx_agent::process_capabilities::probe().filesystem);
    assert!(pryx_agent::process_capabilities::private_ipc_probe().supported());
    let root = tempfile::tempdir().unwrap();
    let bundle = PathBuf::from(std::env::var_os("PRYX_DESKTOP_APPLICATION_BUNDLE").unwrap()).canonicalize().unwrap();
    let mut input = Input {
        root: root.path().join("owner"),
        bundle,
        address: format!("pryx-ssh-{}", uuid::Uuid::new_v4()),
        neighbour: false,
    };
    for role in ["owner", "neighbour"] {
        for directory in ["data", "scratch", "downloads"] {
            fs::create_dir_all(root.path().join(role).join(directory)).unwrap();
        }
    }
    fs::create_dir(input.root.join("scratch/ssh")).unwrap();
    fs::create_dir(input.root.join("data/.ssh-session")).unwrap();
    fs::write(input.root.join("data/marker"), b"native-owned-data").unwrap();
    fs::write(root.path().join("neighbour/data/marker"), b"neighbour-private-data").unwrap();
    let private = PrivateKey::random(&mut ssh_key::rand_core::OsRng, Algorithm::Ed25519).unwrap();
    let host = PrivateKey::random(&mut ssh_key::rand_core::OsRng, Algorithm::Ed25519).unwrap();
    let files = input.root.join("scratch/ssh");
    fs::write(files.join("host_key"), host.to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
    fs::set_permissions(files.join("host_key"), fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(files.join("authorized_keys"), private.public_key().to_openssh().unwrap()).unwrap();
    fs::set_permissions(files.join("authorized_keys"), fs::Permissions::from_mode(0o600)).unwrap();
    let mount = PathBuf::from(process_tool_policy::SSH_AUTH_DIRECTORY).join("ssh");
    let configuration = format!(
        "HostKey \"{}/host_key\"\nAuthorizedKeysFile \"{}/authorized_keys\"\nAllowUsers pryx\nUsePAM no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nAuthenticationMethods publickey\nPubkeyAuthentication yes\nPermitRootLogin no\nPermitEmptyPasswords no\nAllowAgentForwarding no\nAllowTcpForwarding no\nX11Forwarding no\nPermitTunnel no\nPermitUserEnvironment no\nPermitUserRC no\nUseDNS no\nStrictModes yes\nLogLevel ERROR\nSetEnv PRYX_SSH_EXECUTABLE=\"{}\"\n",
        config_word(&mount),
        config_word(&mount),
        config_word(Path::new(env!("CARGO_BIN_EXE_pryx-agent")))
    );
    fs::write(files.join("config"), configuration).unwrap();
    fs::write(root.path().join("client.pub"), private.public_key().to_openssh().unwrap()).unwrap();
    fs::write(root.path().join("known_hosts"), format!("pryx-native {}\n", host.public_key().to_openssh().unwrap()))
        .unwrap();
    let mut owner = Running(
        worker(&input)
            .stdout(Stdio::null())
            .stderr(fs::File::create(root.path().join("owner.stderr")).unwrap())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + pryx_agent::contract::TOOL_STARTUP;
    while !input.root.join("data/ready").exists() {
        assert!(
            owner.0.try_wait().unwrap().is_none() && Instant::now() < deadline,
            "{}\n{}",
            fs::read_to_string(root.path().join("owner.stderr")).unwrap(),
            fs::read_to_string(files.join("server.stderr")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut agent = Running(
        Command::new(input.bundle.join("desktop/bin/ssh-agent"))
            .args(["-D", "-a"])
            .arg(root.path().join("client-agent.sock"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    while !root.path().join("client-agent.sock").exists() {
        assert!(agent.0.try_wait().unwrap().is_none() && Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut add = Command::new(input.bundle.join("desktop/bin/ssh-add"))
        .arg("-")
        .env_clear()
        .env("SSH_AUTH_SOCK", root.path().join("client-agent.sock"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    add.stdin.take().unwrap().write_all(private.to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
    assert!(finish(add).status.success());
    let own = output(client(&input, root.path(), Some("cat marker")));
    assert!(
        own.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&own.stderr),
        fs::read_to_string(files.join("server.stderr")).unwrap()
    );
    assert_eq!(own.stdout, b"native-owned-data");
    let denied = output(client(
        &input,
        root.path(),
        Some(&format!("cat {}", quote(&root.path().join("neighbour/data/marker")))),
    ));
    assert!(!denied.status.success());
    assert!(denied.stdout.is_empty());
    let mut terminal = client(&input, root.path(), None)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    terminal.stdin.take().unwrap().write_all(b"printf 'native-pty-observed\\n'\nexit\n").unwrap();
    let terminal = finish(terminal);
    assert!(terminal.status.success(), "{}", String::from_utf8_lossy(&terminal.stderr));
    assert!(String::from_utf8_lossy(&terminal.stdout).contains("native-pty-observed\r\n"));
    assert_eq!(
        fs::read_dir(input.root.join("data/.ssh-session")).unwrap().count(),
        0,
        "No key/configuration may enter host persistent data"
    );
    input.neighbour = true;
    input.root = root.path().join("neighbour");
    let mut neighbour = Running(worker(&input).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = neighbour.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut diagnostic = String::new();
    neighbour.0.stderr.take().unwrap().read_to_string(&mut diagnostic).unwrap();
    assert!(status.success(), "{diagnostic}");
    let neighbour: serde_json::Value =
        serde_json::from_slice(&fs::read(input.root.join("data/neighbour.json")).unwrap()).unwrap();
    let report = json!({"scope":"Real staged OpenSSH worker; installed SDK/HTTP-edge wake and thirty samples still required","uid":rustix::process::geteuid().as_raw(),"command":true,"pty":true,"strictHostKey":true,"clientKeyOnlyInAgentMemory":true,"neighbourFilesDeniedThroughSsh":true,"persistentDataHasNoAuthorization":true,"neighbour":neighbour});
    if let Some(path) = std::env::var_os("PRYX_NATIVE_SSH_REPORT") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap()
    }
    fs::write(root.path().join("owner/data/stop"), b"stop").unwrap();
    println!("{report}");
}
