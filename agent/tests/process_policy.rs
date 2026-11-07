// SPDX-License-Identifier: FSL-1.1-ALv2
//! Kernel enforcement is exercised in a child, so the test runner never changes its own
//! Landlock, seccomp or rlimits. Unsupported policy fails closed rather than skipping a test.
use pryx_agent::process_policy::{POLICY_ENV, Policy};
use std::{os::unix::process::CommandExt, process::Command};

#[test]
fn process_policy_is_enforced_or_compute_is_refused() {
    let data = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret");
    std::fs::write(&secret, "private").unwrap();
    let policy = Policy {
        data: data.path().to_path_buf(),
        memory_bytes: 512 * 1024 * 1024,
        address_space_bytes: 64 * 1024 * 1024 * 1024,
        open_files: 256,
        processes: 4096,
        cpu_seconds: 60,
        nice: 1,
        network_off: false,
    };
    let output = Command::new(env!("CARGO_BIN_EXE_pryx-agent"))
        .args(["restricted", "/bin/sh", "-c", "printf allowed > permitted; cat permitted; if [ -f /etc/mime.types ]; then cat /etc/mime.types >/dev/null || exit 1; fi; if cat \"$1\" 2>/dev/null; then exit 1; fi; if unshare -Ur true 2>/dev/null; then exit 1; fi", "sh", secret.to_str().unwrap()])
        .current_dir(data.path()).env(POLICY_ENV, serde_json::to_string(&policy).unwrap()).output().unwrap();
    if pryx_agent::process_capabilities::probe().filesystem {
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, b"allowed");
    } else {
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(pryx_agent::process_capabilities::FILESYSTEM_UNAVAILABLE)
        );
        assert!(!data.path().join("permitted").exists());
    }
    assert_eq!(std::fs::read(&secret).unwrap(), b"private");
}

#[test]
fn network_off_denies_connections_without_another_namespace() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_pryx-agent"))
        .args(["network-off", "/usr/bin/python3", "-c", "import socket,sys\ntry: socket.create_connection(('127.0.0.1',int(sys.argv[1])))\nexcept PermissionError: print('filtered')\nelse: raise AssertionError('network escaped')", &address.port().to_string()])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, b"filtered\n");
}

#[test]
fn command_launcher_protects_parent_but_allows_self_signals() {
    let output = Command::new(env!("CARGO_BIN_EXE_pryx-agent"))
        .args([
            "process-child",
            &std::process::id().to_string(),
            "false",
            "python3",
            "-c",
            r#"
import errno, os
parent = os.getppid()
for target in [parent, -parent, -1] + [int(t) for t in os.listdir('/proc/%s/task' % parent)]:
    try:
        os.kill(target, 0)
    except OSError as error:
        assert error.errno == errno.EPERM, (target, error)
    else:
        raise AssertionError('supervisor signal accepted')
os.kill(os.getpid(), 0)
os.kill(0, 0)
print('protected')
"#,
        ])
        .process_group(0)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, b"protected\n");
}
