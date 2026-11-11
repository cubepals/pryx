// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real supervisor policy scope is tested separately from the disposable primitive probe.
//! An unscoped caller is observed explicitly; its access must never imply legacy-peer privacy.
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read},
    os::{
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr, UnixStream},
    },
    process::{Child, Stdio},
    sync::mpsc,
    time::Duration,
};

struct Owner(Child);
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn observe(supported: bool) -> Value {
    let directory = tempfile::tempdir().unwrap();
    if !supported {
        let output = super::restricted(directory.path())
            .args(["restricted", "/usr/bin/python3", "-c", "print('filesystem-only')"])
            .output()
            .unwrap();
        if pryx_agent::process_capabilities::probe().filesystem {
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            assert_eq!(output.stdout, b"filesystem-only\n");
        } else {
            assert!(!output.status.success());
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains(pryx_agent::process_capabilities::FILESYSTEM_UNAVAILABLE)
            );
        }
        return json!({"status":"unavailable", "positiveIsolationClaim":false});
    }
    let name = format!("pryx-scoped-agent-{}", uuid::Uuid::new_v4());
    let mut child = Owner(
        super::restricted(directory.path())
            .args([
                "restricted",
                "/usr/bin/python3",
                "-c",
                r#"
import json, os, socket, sys
address = '\0' + sys.argv[1]
server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.settimeout(2)
server.bind(address)
server.listen(4)
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.settimeout(2)
client.connect(address)
client.sendall(b'own!')
peer, _ = server.accept()
assert peer.recv(4) == b'own!'
print(json.dumps({'uid':os.geteuid(),'ownerReachable':True}), flush=True)
sys.stdin.read(1)
"#,
            ])
            .arg(&name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(result.map(|_| line));
    });
    let ready = rx.recv_timeout(Duration::from_secs(5)).expect("supervisor readiness deadline").unwrap();
    assert!(!ready.is_empty(), "supervisor exited before its owner control");
    let owner: Value = serde_json::from_str(&ready).unwrap();
    assert_eq!(owner["uid"], rustix::process::geteuid().as_raw());
    assert_eq!(owner["ownerReachable"], true);
    let address = SocketAddr::from_abstract_name(&name).unwrap();
    let unscoped_reachable = UnixStream::connect_addr(&address).is_ok();
    let neighbour = tempfile::tempdir().unwrap();
    let output = super::restricted(neighbour.path())
        .args([
            "restricted",
            "/usr/bin/python3",
            "-c",
            r#"
import json, os, socket, sys
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.settimeout(2)
try: client.connect('\0' + sys.argv[1])
except PermissionError: print(json.dumps({'uid':os.geteuid(),'refused':True}))
else: raise AssertionError('independent supervisor reached owner socket')
"#,
        ])
        .arg(&name)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let peer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(peer["uid"], owner["uid"]);
    assert_eq!(peer["refused"], true);
    drop(child.0.stdin.take());
    assert!(child.0.wait().unwrap().success());
    let mut stderr = String::new();
    child.0.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
    assert!(stderr.is_empty(), "{stderr}");
    json!({"status":"enforced", "owner":owner, "neighbour":peer,
        "unscopedCallerReachable":unscoped_reachable,
        "scope":"all untrusted peers must enforce their own domains; no legacy readiness claim"})
}
