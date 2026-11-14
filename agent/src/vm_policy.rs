// SPDX-License-Identifier: FSL-1.1-ALv2
//! Close the guest packet-transport bypass after trusted ingress listeners are bound.
//! Every agent thread and future image process inherits this irreversible filter; existing
//! authenticated ingress sockets remain usable. Trusted root init and its forwarder are separate.
use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule,
};
use std::collections::BTreeMap;

pub const VERSION: u16 = 1;

pub(crate) fn enter() -> Result<(), Box<dyn std::error::Error>> {
    let domain = SeccompCondition::new(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, libc::AF_VSOCK as u64)?;
    let rule = SeccompRule::new(vec![domain])?;
    let rules = BTreeMap::from([
        (libc::SYS_socket, vec![rule.clone()]),
        (libc::SYS_socketpair, vec![rule]),
        // io_uring's socket/connect operations must not bypass the socket syscall filter.
        (libc::SYS_io_uring_setup, vec![]),
    ]);
    let program: BpfProgram = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        std::env::consts::ARCH.try_into()?,
    )?
    .try_into()?;
    crate::syscall_abi::apply_all_threads(&program)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{io, os::unix::net::UnixStream, process::Command};

    const MODE: &str = "PRYX_TEST_GUEST_SOCKET_POLICY";
    const TEST: &str = "vm_policy::tests::blocks_sockets_across_threads_and_exec_without_closing_ingress";

    fn filters() -> usize {
        // proc_pid_status(5): thread-self observes this thread's installed filter count.
        std::fs::read_to_string("/proc/thread-self/status")
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("Seccomp_filters:").map(|n| n.trim().parse().unwrap()))
            .unwrap()
    }

    fn refused() {
        let error =
            vsock::VsockStream::connect_with_cid_port(tokio_vsock::VMADDR_CID_HOST, crate::vm_boot::PORT).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let mut command = Command::new("python3");
        command
            .args([
                "-c",
                r#"
import ctypes, errno, socket, sys
for operation in [lambda: socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM),
                  lambda: socket.socketpair(socket.AF_VSOCK, socket.SOCK_STREAM)]:
    try:
        operation()
    except OSError as error:
        assert error.errno == errno.EPERM, error
    else:
        raise AssertionError('guest packet socket was allowed')
libc = ctypes.CDLL(None, use_errno=True)
libc.syscall.restype = ctypes.c_long
assert libc.syscall(int(sys.argv[1]), 1, ctypes.c_void_p()) == -1
assert ctypes.get_errno() == errno.EPERM
if len(sys.argv) > 2:
    assert libc.syscall(int(sys.argv[3])) > 0
    assert libc.syscall(int(sys.argv[2]) | int(sys.argv[3])) == -1
    assert ctypes.get_errno() == errno.EPERM
"#,
            ])
            .arg(libc::SYS_io_uring_setup.to_string());
        #[cfg(target_arch = "x86_64")]
        command.args([linux_raw_sys::general::__X32_SYSCALL_BIT.to_string(), libc::SYS_getpid.to_string()]);
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).is_ok());
    }

    #[test]
    fn blocks_sockets_across_threads_and_exec_without_closing_ingress() {
        use std::io::{Read, Write};
        let Some(mode) = std::env::var_os(MODE) else {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env(MODE, "install")
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
            return;
        };
        if mode == "inherited" {
            refused();
            return;
        }
        let (mut input, mut output) = UnixStream::pair().unwrap();
        let (ready, waiting) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            ready.send(filters()).unwrap();
            released.recv().unwrap();
            refused();
            filters()
        });
        let before = waiting.recv().unwrap();
        super::enter().unwrap();
        release.send(()).unwrap();
        assert!(worker.join().unwrap() > before, "TSYNC must filter already existing worker threads");
        refused();
        input.write_all(b"existing socket").unwrap();
        let mut bytes = [0; 15];
        output.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"existing socket");
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(MODE, "inherited")
            .output()
            .unwrap();
        assert!(child.status.success(), "{}", String::from_utf8_lossy(&child.stdout));
    }
}
