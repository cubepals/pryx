// SPDX-License-Identifier: FSL-1.1-ALv2
//! Record the actual shared-UID pathname socket boundary before choosing a native tool transport.
//! The existing agent installs its real policy in separate children; no tool is enabled here.
//!
//! Parts (`private_ipc/`):
//! - `scoped.rs`: installed supervisor domains, a live owner and a same-UID peer.
#[path = "private_ipc/scoped.rs"]
mod scoped;
use pryx_agent::process_policy::{POLICY_ENV, Policy};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    os::unix::{
        fs::OpenOptionsExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    process::{Command, Output},
};

#[test]
fn pathname_ipc_design_proves_file_ownership_and_records_connect_privacy() {
    let owner = tempfile::tempdir().unwrap();
    let neighbour = tempfile::tempdir().unwrap();
    let socket = owner.path().join("kernel.sock");
    let secret = owner.path().join("secret");
    std::fs::write(&secret, b"private-design-fixture").unwrap();
    let listener = UnixListener::bind(&socket).unwrap();
    // A live endpoint, not a failed lookup or a dead server, is the positive control.
    let mut host = UnixStream::connect(&socket).unwrap();
    host.write_all(b"host").unwrap();
    let mut received = [0; 4];
    listener.accept().unwrap().0.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"host");
    drop(host);
    let own = connect(owner.path(), &socket, &secret, false);
    let peer = connect(neighbour.path(), &socket, &secret, true);
    let filesystem = pryx_agent::process_capabilities::probe().filesystem;
    let result = if filesystem {
        assert!(own.status.success(), "{}", String::from_utf8_lossy(&own.stderr));
        assert!(peer.status.success(), "{}", String::from_utf8_lossy(&peer.stderr));
        let own: Value = serde_json::from_slice(&own.stdout).unwrap();
        let peer: Value = serde_json::from_slice(&peer.stdout).unwrap();
        let uid = rustix::process::geteuid().as_raw();
        assert_eq!(own["uid"], uid);
        assert_eq!(peer["uid"], uid);
        assert_eq!(own["connection"], "reachable");
        assert_eq!(peer["fileRead"], "blocked");
        assert!(matches!(peer["connection"].as_str(), Some("reachable" | "blocked")));
        json!({"filesystem": "enforced", "sameUid": true, "owner": own, "neighbour": peer,
            "scope": "design observation only; native process tools remain refused"})
    } else {
        for output in [&own, &peer] {
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains(pryx_agent::process_capabilities::FILESYSTEM_UNAVAILABLE)
            );
        }
        json!({"filesystem": "unavailable", "status": "refused", "sameUid": true,
            "scope": "no positive IPC confinement claim"})
    };
    assert_eq!(std::fs::read(&secret).unwrap(), b"private-design-fixture");
    report(result);
}

fn connect(data: &Path, socket: &Path, secret: &Path, neighbour: bool) -> Output {
    restricted(data)
        .args([
            "restricted",
            "/usr/bin/python3",
            "-c",
            r#"
import json, os, pathlib, socket, sys
file_read = 'not-requested'
if sys.argv[3] == 'true':
    try: pathlib.Path(sys.argv[2]).read_bytes()
    except PermissionError: file_read = 'blocked'
    else: raise AssertionError('neighbour file was readable')
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.settimeout(2)
try: client.connect(sys.argv[1])
except PermissionError: connection = 'blocked'
else: connection = 'reachable'
print(json.dumps({'uid':os.geteuid(),'fileRead':file_read,'connection':connection}))
"#,
        ])
        .arg(socket)
        .arg(secret)
        .arg(neighbour.to_string())
        .output()
        .unwrap()
}

fn report(mut value: Value) {
    let ipc = pryx_agent::process_capabilities::private_ipc_probe();
    if ipc.scope_available {
        assert!(ipc.completed, "abstract socket probe did not finish: {ipc:?}");
        assert_eq!(ipc.owner_reachable, Some(true));
        assert_eq!(ipc.neighbour_refused, Some(true));
    }
    assert_eq!(pryx_agent::process_capabilities::probe().abstract_unix_ipc, ipc.supported());
    value["abstract"] = serde_json::to_value(ipc).unwrap();
    value["supervisorAbstract"] = scoped::observe(ipc.supported());
    if let Some(path) = std::env::var_os("PRYX_PRIVATE_IPC_REPORT") {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).unwrap();
        file.write_all(serde_json::to_string_pretty(&value).unwrap().as_bytes()).unwrap();
        file.sync_all().unwrap();
    }
    println!("{value}");
}

fn restricted(data: &Path) -> Command {
    let policy = Policy {
        tools: None,
        require_ipc_scope: None,
        data: data.to_owned(),
        memory_bytes: 512 * 1024 * 1024,
        address_space_bytes: 64 * 1024 * 1024 * 1024,
        open_files: 256,
        processes: 4096,
        cpu_seconds: 60,
        nice: 1,
        network_off: false,
    };
    let mut command = Command::new(env!("CARGO_BIN_EXE_pryx-agent"));
    command.current_dir(data).env(POLICY_ENV, serde_json::to_string(&policy).unwrap());
    command
}

#[test]
fn ipc_probe_stays_available_when_it_cannot_create_a_worker() {
    let output = Command::new("/usr/bin/python3")
        .args([
            "-c",
            r#"
import os, resource, sys
_, hard = resource.getrlimit(resource.RLIMIT_NPROC)
resource.setrlimit(resource.RLIMIT_NPROC, (1, hard))
os.execv(sys.argv[1], [sys.argv[1], 'process-probe'])
"#,
        ])
        .arg(env!("CARGO_BIN_EXE_pryx-agent"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    // Root can bypass RLIMIT_NPROC. Ordinary-user CI must actually withhold this capability.
    if rustix::process::geteuid().as_raw() != 0 {
        assert!(!value["abstractUnixIpc"].as_bool().unwrap_or(false));
    }
}
