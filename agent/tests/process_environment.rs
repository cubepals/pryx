// SPDX-License-Identifier: FSL-1.1-ALv2
//! The actual readable agent wrapper cannot load guest code before its child network filter.
//! Only owned loopback listeners and marker files are used; full filesystem proof runs hosted.
use pryx_agent::process_environment::{ARGUMENT, CHILD_MODE, Pending, VERSION, WORKLOAD_ARGUMENT};
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, TcpListener};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

#[expect(unsafe_code, reason = "Only the owned child's socket descriptor is opened across its exec")]
fn inherited(command: &mut Command, descriptor: i32) {
    // SAFETY: the parent holds this socket until spawn returns. The child allocates nothing;
    // fcntl is async-signal-safe and changes only this child's descriptor table.
    unsafe {
        command.pre_exec(move || {
            rustix::io::fcntl_setfd(std::os::fd::BorrowedFd::borrow_raw(descriptor), rustix::io::FdFlags::empty())?;
            Ok(())
        });
    }
}

#[test]
fn guest_constructor_runs_only_after_the_actual_child_network_filter() {
    let root = tempfile::tempdir().unwrap();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/process-loader.c");
    let library = root.path().join("owned-loader.so");
    assert!(
        Command::new("cc")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .args(["-Wall", "-Wextra", "-Werror", "-fPIC", "-shared"])
            .arg(source)
            .arg("-o")
            .arg(&library)
            .status()
            .unwrap()
            .success()
    );
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let marker = root.path().join("owned-confined-loader-marker");
    let environment = BTreeMap::from([
        ("LD_PRELOAD".into(), library.to_string_lossy().into_owned()),
        ("PRYX_TEST_LOADER_PORT".into(), listener.local_addr().unwrap().port().to_string()),
        ("PRYX_TEST_LOADER_MARKER".into(), marker.to_string_lossy().into_owned()),
    ]);
    let binary = env!("CARGO_BIN_EXE_pryx-agent");
    assert!(
        Command::new(binary).arg("bootstrap-probe").env_clear().envs(&environment).output().unwrap().status.success()
    );
    assert!(marker.exists(), "the old real-loader positive control must execute");
    let _owned_connection = listener.accept().expect("the old constructor reaches the owned loopback listener");
    std::fs::remove_file(&marker).unwrap();
    let mut bootstrap = environment.clone();
    bootstrap.insert(pryx_agent::process_policy::POLICY_ENV.into(), "{}".into());
    bootstrap.insert("OWNED_BOOTSTRAP".into(), "owned-private-marker".into());
    let socket = Pending::new().unwrap().queue(&bootstrap).unwrap();
    let mut command = Command::new(binary);
    command.env_clear().arg(ARGUMENT).arg(socket.as_raw_fd().to_string()).arg("bootstrap-probe");
    inherited(&mut command, socket.as_raw_fd());
    let result = command.output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let metadata: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(metadata["processEnvironmentVersion"], VERSION);
    assert!(!marker.exists(), "the actual bootstrap loader received guest values");
    assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    drop(socket);
    let socket = Pending::new().unwrap().queue(&environment).unwrap();
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .process_group(0)
        .arg(WORKLOAD_ARGUMENT)
        .arg(socket.as_raw_fd().to_string())
        .arg(CHILD_MODE)
        .arg(std::process::id().to_string())
        .arg("true")
        .arg("/bin/true");
    inherited(&mut command, socket.as_raw_fd());
    let result = command.output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert!(marker.exists(), "the final command must retain its requested loader variable");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "a guest constructor ran before the real child network filter"
    );
}
