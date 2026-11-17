// SPDX-License-Identifier: FSL-1.1-ALv2
//! The production launch command keeps guest values out of real dynamic loaders. The test
//! fixture writes an owned marker only; it never reads a host credential or arbitrary path.
use super::*;
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;

fn dynamic_fixture(root: &std::path::Path) -> (PathBuf, PathBuf) {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/process-loader.c");
    let program = root.join("owned-agent");
    let library = root.join("owned-loader.so");
    for (output, arguments) in [(&program, vec!["-DPRYX_LOADER_PROGRAM"]), (&library, vec!["-fPIC", "-shared"])] {
        let result = Command::new("cc")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .args(arguments)
            .arg(&source)
            .arg("-o")
            .arg(output)
            .output()
            .unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    }
    (program, library)
}

fn spec(data: &std::path::Path, binary: &std::path::Path) -> ContainerSpec {
    serde_json::from_value(serde_json::json!({
        "name":"owned-environment", "image":"", "env":[], "labels":{},
        "agent":{"binary":binary, "sandbox":"owned-environment", "epoch":1,
            "secret_env":"OWNED_BOOTSTRAP", "domain":"pryx.test",
            "agent_port":pryx_agent::contract::AGENT_PORT, "spare_ports":[], "keep_alive":true},
        "user":"", "memory_bytes":0, "cpu_shares":0, "pids_limit":0,
        "read_only_rootfs":true, "tmp_size_mb":0, "data_dir":data, "mount_path":"/data",
        "ports":[{"container_port":pryx_agent::contract::AGENT_PORT,"host_port":12345,
            "host_ips":["127.0.0.1"],"protocol":"tcp"}],
        "stop_signal":"SIGTERM", "stop_timeout_secs":0, "network":"", "internal_network":false,
        "log_max_size_mb":0, "log_max_files":0, "oom_score_adj":0
    }))
    .unwrap()
}

fn frame(descriptor: &std::os::fd::OwnedFd) -> BTreeMap<String, String> {
    let mut bytes = [0; 128];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = RecvAncillaryBuffer::new(&mut space);
    let received = rustix::net::recvmsg(
        descriptor,
        &mut [std::io::IoSliceMut::new(&mut bytes)],
        &mut control,
        RecvFlags::CMSG_CLOEXEC | RecvFlags::DONTWAIT,
    )
    .unwrap();
    assert!(received.bytes > 0);
    let mut files = Vec::new();
    for message in control.drain() {
        if let RecvAncillaryMessage::ScmRights(rights) = message {
            files.extend(rights);
        }
    }
    assert_eq!(files.len(), 1);
    let file = fs::File::from(files.pop().unwrap());
    serde_json::from_reader(file).unwrap()
}

fn isolated_descriptors() -> bool {
    const MARKER: &str = "PRYX_TEST_OWNED_AMBIENT_DESCRIPTORS";
    if std::env::var_os(MARKER).is_some() {
        return false;
    }
    let thread = std::thread::current();
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", thread.name().unwrap()])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(MARKER, "owned")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
    true
}

#[expect(unsafe_code, reason = "Only this isolated owned test process creates and inherits its own pipe")]
fn ambient_descriptors(root: &std::path::Path) -> Vec<OwnedFd> {
    let mut pipe = [-1; 2];
    // SAFETY: pipe2 initializes both valid descriptors on success, and each is owned once.
    let mut held = unsafe {
        assert_eq!(libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC), 0);
        vec![OwnedFd::from_raw_fd(pipe[0]), OwnedFd::from_raw_fd(pipe[1])]
    };
    held.push(fs::File::open(root).unwrap().into());
    for descriptor in &held {
        rustix::io::fcntl_setfd(descriptor, rustix::io::FdFlags::empty()).unwrap();
    }
    held
}

fn check_descriptors(command: &mut Command, ambient: &[OwnedFd], held: &[OwnedFd]) {
    command.arg("--verify-descriptors");
    for descriptor in ambient {
        command.arg(descriptor.as_raw_fd().to_string()).arg("closed");
    }
    for descriptor in held {
        command.arg(descriptor.as_raw_fd().to_string()).arg("open");
    }
}

#[test]
fn guest_loader_values_are_private_data_and_host_bootstrap_overrides_win() {
    if isolated_descriptors() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let (binary, library) = dynamic_fixture(root.path());
    let ambient = ambient_descriptors(root.path());
    let outside = root.path().join("outside-data");
    fs::create_dir(&outside).unwrap();
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o777)).unwrap();
    let marker = outside.join("outside-data-marker");
    let data = root.path().join("data");
    fs::create_dir(&data).unwrap();
    let runtime = ProcessRuntime::new(root.path(), ProcessConfig::default()).unwrap();
    let mut spec = spec(&data, &binary);
    spec.env = vec![
        ("LD_PRELOAD".into(), library.to_string_lossy().into_owned()),
        ("PRYX_TEST_LOADER_MARKER".into(), marker.to_string_lossy().into_owned()),
        ("APP_VALUE".into(), "owned-value".into()),
        ("OWNED_BOOTSTRAP".into(), "owned-private-value".into()),
        (pryx_agent::process_policy::POLICY_ENV.into(), "guest-policy-must-not-win".into()),
    ];
    assert!(Command::new(&binary).env_clear().envs(spec.env.iter().cloned()).status().unwrap().success());
    assert!(marker.exists(), "the real loader control must execute its constructor");
    fs::remove_file(&marker).unwrap();
    runtime.create_entry(&spec).unwrap();
    let entry = runtime.entry(&spec.name).unwrap();
    let (mut command, held) = runtime.launch_command(&entry, None, false).unwrap();
    check_descriptors(&mut command, &ambient, &held);
    let visible =
        command.get_envs().filter_map(|(name, value)| value.map(|_| name.to_str().unwrap())).collect::<Vec<_>>();
    assert_eq!(visible, ["HOME", "PATH", "TERM", "TMPDIR"]);
    let args = command.get_args().map(|value| value.to_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(args.first().copied(), Some(pryx_agent::process_environment::ARGUMENT));
    let number: i32 = args[1].parse().unwrap();
    let socket = held.iter().find(|value| value.as_raw_fd() == number).unwrap();
    let resolved = frame(socket);
    assert_eq!(
        resolved.get("LD_PRELOAD"),
        spec.env.iter().find(|(key, _)| key == "LD_PRELOAD").map(|(_, value)| value)
    );
    assert_eq!(resolved.get("APP_VALUE").map(String::as_str), Some("owned-value"));
    assert_eq!(resolved.get("OWNED_BOOTSTRAP").map(String::as_str), Some("owned-private-value"));
    let policy: serde_json::Value = serde_json::from_str(&resolved[pryx_agent::process_policy::POLICY_ENV]).unwrap();
    assert_eq!(policy["data"], data.to_string_lossy().as_ref());
    assert!(command.status().unwrap().success());
    for descriptor in &ambient {
        assert_eq!(rustix::io::fcntl_getfd(descriptor).unwrap(), rustix::io::FdFlags::empty());
    }
    for descriptor in &held {
        assert!(rustix::io::fcntl_getfd(descriptor).unwrap().contains(rustix::io::FdFlags::CLOEXEC));
    }
    assert!(!marker.exists(), "guest environment reached a trusted dynamic loader");
}

#[test]
fn bootstrap_keeps_every_owned_listener_and_only_its_explicit_descriptors() {
    if isolated_descriptors() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let (binary, _) = dynamic_fixture(root.path());
    let ambient = ambient_descriptors(root.path());
    let spec = spec(root.path(), &binary);
    let bindings = BTreeMap::from([(0, vec![std::net::Ipv4Addr::LOCALHOST.into(); 2])]);
    let mut command = Command::new(&binary);
    let held = pass_bootstrap(
        &mut command,
        &spec,
        root.path(),
        &serde_json::json!({}),
        true,
        &bindings,
        pryx_agent::process_environment::Pending::new().unwrap(),
    )
    .unwrap();
    assert_eq!(held.len(), 3);
    pryx_agent::process_environment::inherit_only(&mut command, held.iter().map(AsRawFd::as_raw_fd).collect()).unwrap();
    check_descriptors(&mut command, &ambient, &held);
    let result = command.output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    for descriptor in &held {
        assert!(rustix::io::fcntl_getfd(descriptor).unwrap().contains(rustix::io::FdFlags::CLOEXEC));
    }
}

#[tokio::test]
async fn old_agent_metadata_refuses_before_any_guest_environment_launch() {
    let root = tempfile::tempdir().unwrap();
    let (binary, library) = dynamic_fixture(root.path());
    let marker = root.path().join("unsupported-agent-marker");
    let data = root.path().join("data");
    fs::create_dir(&data).unwrap();
    let runtime = ProcessRuntime::new(root.path(), ProcessConfig::default()).unwrap();
    let mut spec = spec(&data, &binary);
    spec.env = vec![
        ("LD_PRELOAD".into(), library.to_string_lossy().into_owned()),
        ("PRYX_TEST_LOADER_MARKER".into(), marker.to_string_lossy().into_owned()),
    ];
    runtime.create_entry(&spec).unwrap();
    assert!(matches!(runtime.launch(&spec.name).await,
        Err(RuntimeError::Other(message)) if message==pryx_agent::process_environment::UNSUPPORTED));
    assert!(runtime.entry(&spec.name).unwrap().pid.is_none());
    assert!(!marker.exists());
}
