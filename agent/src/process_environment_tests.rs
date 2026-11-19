// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real anonymous-file and Unix-rights controls for the private environment contract. Values
//! are owned test markers; no host credentials or process environments are inspected.
use super::*;
use std::io::Write;

#[expect(unsafe_code, reason = "Only this isolated owned test process creates and inherits its own pipe")]
fn ambient_descriptors(root: &std::path::Path) -> Vec<OwnedFd> {
    let mut pipe = [-1; 2];
    // SAFETY: pipe2 initializes both valid descriptors on success, and each is owned once.
    let mut held = unsafe {
        assert_eq!(libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC), 0);
        vec![OwnedFd::from_raw_fd(pipe[0]), OwnedFd::from_raw_fd(pipe[1])]
    };
    held.push(File::open(root).unwrap().into());
    for descriptor in &held {
        rustix::io::fcntl_setfd(descriptor, rustix::io::FdFlags::empty()).unwrap();
    }
    held
}

fn descriptor_fixture(root: &std::path::Path) -> std::path::PathBuf {
    let program = root.join("owned-descriptor-check");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/process-loader.c");
    let result = std::process::Command::new("cc")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args(["-DPRYX_LOADER_PROGRAM", "-Wall", "-Wextra", "-Werror"])
        .arg(source)
        .arg("-o")
        .arg(&program)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    program
}

fn isolated_descriptor_case(marker: &str, value: &str) {
    let thread = std::thread::current();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", thread.name().unwrap()])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(marker, value)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
}

#[test]
fn workload_inherits_only_its_owned_handoff_and_keeps_spawn_errors() {
    const MARKER: &str = "PRYX_TEST_OWNED_WORKLOAD_DESCRIPTORS";
    if std::env::var_os(MARKER).is_none() {
        isolated_descriptor_case(MARKER, "owned");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let program = descriptor_fixture(root.path());
    let ambient = ambient_descriptors(root.path());
    let socket = Pending::new().unwrap().queue(&BTreeMap::new()).unwrap();
    let command = |closed| {
        let mut command = std::process::Command::new(&program);
        command.env_clear().arg("--verify-descriptors");
        for descriptor in &ambient {
            command.arg(descriptor.as_raw_fd().to_string()).arg(if closed { "closed" } else { "open" });
        }
        command
    };
    // Real exec proves these capabilities were inheritable before the production boundary.
    assert!(command(false).status().unwrap().success());
    let mut child = command(true);
    child.arg(socket.as_raw_fd().to_string()).arg("open");
    inherit(&mut child, &socket);
    let result = child.output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    for descriptor in &ambient {
        assert_eq!(rustix::io::fcntl_getfd(descriptor).unwrap(), rustix::io::FdFlags::empty());
    }
    assert!(rustix::io::fcntl_getfd(&socket).unwrap().contains(rustix::io::FdFlags::CLOEXEC));
    let mut missing = std::process::Command::new(root.path().join("absent-executable"));
    inherit(&mut missing, &socket);
    assert_eq!(missing.spawn().unwrap_err().kind(), io::ErrorKind::NotFound);
}

#[test]
fn a_host_that_refuses_the_descriptor_call_starts_nothing() {
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule,
    };
    use std::os::unix::process::ExitStatusExt;
    const MARKER: &str = "PRYX_TEST_OWNED_DESCRIPTOR_DENIAL";
    let Some(case) = std::env::var_os(MARKER) else {
        for case in ["unsupported", "denied", "argument", "kill", "trap"] {
            isolated_descriptor_case(MARKER, case);
        }
        return;
    };
    // An argument-sensitive policy refuses only the child's actual call, from stderr upwards.
    let rules = if case == "argument" {
        vec![
            SeccompRule::new(vec![
                SeccompCondition::new(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, (libc::STDERR_FILENO + 1) as u64)
                    .unwrap(),
            ])
            .unwrap(),
        ]
    } else {
        vec![]
    };
    let action = match case.to_str().unwrap() {
        "unsupported" => SeccompAction::Errno(libc::ENOSYS as u32),
        "kill" => SeccompAction::KillProcess,
        "trap" => SeccompAction::Trap,
        _ => SeccompAction::Errno(libc::EPERM as u32),
    };
    let filter: BpfProgram = SeccompFilter::new(
        [(libc::SYS_close_range, rules)].into_iter().collect(),
        SeccompAction::Allow,
        action,
        std::env::consts::ARCH.try_into().unwrap(),
    )
    .unwrap()
    .try_into()
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let ran = root.path().join("owned-command-ran");
    let mut command = std::process::Command::new("/bin/sh");
    command.env_clear().arg("-c").arg("printf ran > \"$1\"").arg("sh").arg(&ran);
    inherit_only(&mut command, Vec::new());
    crate::syscall_abi::apply(&filter).unwrap();
    match case.to_str().unwrap() {
        "unsupported" => assert_eq!(command.spawn().unwrap_err().raw_os_error(), Some(libc::ENOSYS)),
        "denied" | "argument" => assert_eq!(command.spawn().unwrap_err().raw_os_error(), Some(libc::EPERM)),
        // A fatal filter takes only the forked child, before its exec: the parent is still here.
        _ => assert_eq!(command.status().unwrap().signal(), Some(libc::SIGSYS)),
    }
    assert!(!ran.exists(), "the command ran although its descriptors could not be closed");
}

#[test]
fn complete_values_round_trip_without_entering_the_loader_environment() {
    let environment = BTreeMap::from([
        ("APP_VALUE".into(), "owned-value".into()),
        ("LD_PRELOAD".into(), "owned-loader-path".into()),
        ("GCONV_PATH".into(), "owned-conversion-path".into()),
        ("HOST_BOOTSTRAP".into(), "owned-private-marker".into()),
    ]);
    let pending = Pending::new().unwrap();
    let descriptor = pending.descriptor();
    let receiver = pending.queue(&environment).unwrap();
    assert_eq!(receiver.as_raw_fd(), descriptor);
    assert_eq!(receive(&receiver).unwrap(), environment);
    assert_eq!(receive(&receiver).unwrap_err().kind(), io::ErrorKind::WouldBlock);
}

#[test]
fn helpers_cannot_open_the_queued_file_through_the_socket_descriptor() {
    let receiver = Pending::new().unwrap().queue(&BTreeMap::from([("APP_VALUE".into(), "owned".into())])).unwrap();
    let linked = format!("/proc/self/fd/{}", receiver.as_raw_fd());
    assert!(matches!(
        rustix::fs::open(&linked, rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC, Mode::empty()),
        Err(rustix::io::Errno::NXIO)
    ));
    assert_eq!(receive(&receiver).unwrap().get("APP_VALUE").map(String::as_str), Some("owned"));
}

#[test]
fn received_frames_are_sealed_and_close_on_exec() {
    let receiver = Pending::new().unwrap().queue(&BTreeMap::new()).unwrap();
    let frame = receive_file(&receiver).unwrap();
    assert!(rustix::fs::fcntl_get_seals(&frame).unwrap().contains(SEALED));
    assert!(rustix::io::fcntl_getfd(&frame).unwrap().contains(rustix::io::FdFlags::CLOEXEC));
    assert_eq!(rustix::io::write(&frame, b"altered"), Err(rustix::io::Errno::PERM));
    assert_eq!(rustix::fs::ftruncate(&frame, 0), Err(rustix::io::Errno::PERM));
}

fn packet(payload: &[u8], files: &[std::os::fd::BorrowedFd<'_>]) -> OwnedFd {
    let (sender, receiver) = UnixDatagram::pair().unwrap();
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    if !files.is_empty() {
        assert!(control.push(SendAncillaryMessage::ScmRights(files)));
    }
    assert_eq!(
        rustix::net::sendmsg(&sender, &[io::IoSlice::new(payload)], &mut control, SendFlags::NOSIGNAL).unwrap(),
        payload.len()
    );
    receiver.into()
}

fn sealed(bytes: &[u8]) -> File {
    let descriptor =
        rustix::fs::memfd_create("pryx-owned-fixture", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING).unwrap();
    let mut file = File::from(descriptor);
    file.write_all(bytes).unwrap();
    rustix::fs::fcntl_add_seals(&file, SEALED).unwrap();
    file
}

#[test]
fn invalid_rights_and_unsealed_or_malformed_frames_fail_without_values() {
    let private = "owned-marker-that-must-not-appear-in-errors";
    let mut unsealed = tempfile::tempfile().unwrap();
    unsealed.write_all(private.as_bytes()).unwrap();
    let receiver = packet(MESSAGE, &[unsealed.as_fd()]);
    assert!(receive(&receiver).is_err());
    let receiver = packet(MESSAGE, &[]);
    assert_eq!(receive(&receiver).unwrap_err().kind(), io::ErrorKind::InvalidData);
    let receiver = packet(private.as_bytes(), &[]);
    let error = receive(&receiver).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(!error.to_string().contains(private));
    let malformed = sealed(private.as_bytes());
    let receiver = packet(MESSAGE, &[malformed.as_fd()]);
    let error = receive(&receiver).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(!error.to_string().contains(private));
    let one = Pending::new().unwrap().queue(&BTreeMap::new()).unwrap();
    let two = Pending::new().unwrap().queue(&BTreeMap::new()).unwrap();
    let receiver = packet(MESSAGE, &[one.as_fd(), two.as_fd()]);
    assert_eq!(receive(&receiver).unwrap_err().kind(), io::ErrorKind::InvalidData);
}

#[test]
fn private_names_and_all_signing_prefixes_stay_out_of_workload_defaults() {
    let private = ["HOST_BOOTSTRAP", "HOST_LISTENERS"];
    for name in private {
        assert!(!allowed_for_workload(name, &private));
    }
    assert!(!allowed_for_workload(&format!("{}owned", crate::contract::SECRET_ENV_PREFIX), &private));
    for name in ["APP_VALUE", "LD_PRELOAD", "GCONV_PATH", "PATH", "HOME"] {
        assert!(allowed_for_workload(name, &private));
    }
}

#[expect(unsafe_code, reason = "Only the owned child's descriptor is made inheritable after fork")]
fn inherited(command: &mut std::process::Command, descriptor: RawFd) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the parent holds its receiver until spawn returns; the child only changes its
    // own close-on-exec bit with the async-signal-safe fcntl operation.
    unsafe {
        command.pre_exec(move || {
            rustix::io::fcntl_setfd(std::os::fd::BorrowedFd::borrow_raw(descriptor), rustix::io::FdFlags::empty())?;
            Ok(())
        });
    }
}

#[test]
fn adoption_requires_private_process_state_and_never_mutates_global_values() {
    const SOCKET: &str = "PRYX_TEST_ENVIRONMENT_SOCKET";
    if let Some(number) = std::env::var_os(SOCKET) {
        let number: RawFd = number.to_str().unwrap().parse().unwrap();
        assert!(adopt(number).is_err(), "a dumpable process cannot receive the frame");
        rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable).unwrap();
        assert!(adopt(-1).is_err());
        assert!(adopt(i32::MAX).is_err());
        adopt(number).unwrap();
        assert_eq!(var("APP_VALUE").unwrap(), "owned-value");
        assert!(std::env::var_os("APP_VALUE").is_none());
        assert_eq!(var("HOST_BOOTSTRAP").unwrap(), "owned-private-marker");
        assert_eq!(std::env::var("HOST_BOOTSTRAP").unwrap(), "owned-inherited-marker");
        let values = workload(&["HOST_BOOTSTRAP"]);
        assert_eq!(values.get("APP_VALUE").map(String::as_str), Some("owned-value"));
        assert!(!values.contains_key("HOST_BOOTSTRAP"));
        let mut arguments =
            vec!["owned-agent".into(), "process-child".into(), "1".into(), "true".into(), "true".into()];
        let socket = prepare_workload(&mut arguments, &values).unwrap().unwrap();
        assert_eq!(arguments[1], WORKLOAD_ARGUMENT);
        arguments.remove(0);
        let descriptor = parse_workload(&mut arguments).unwrap().unwrap();
        assert_eq!(descriptor, socket.as_raw_fd());
        assert_eq!(arguments[0], "process-child");
        assert_eq!(receive(&socket).unwrap(), values);
        assert!(parse_workload(&mut vec![WORKLOAD_ARGUMENT.into(), "3".into(), "bootstrap-probe".into()]).is_err());
        return;
    }
    let receiver = Pending::new()
        .unwrap()
        .queue(&BTreeMap::from([
            ("APP_VALUE".into(), "owned-value".into()),
            ("HOST_BOOTSTRAP".into(), "owned-private-marker".into()),
        ]))
        .unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "process_environment::tests::adoption_requires_private_process_state_and_never_mutates_global_values",
        ])
        .env_clear()
        .env(SOCKET, receiver.as_raw_fd().to_string())
        .env("HOST_BOOTSTRAP", "owned-inherited-marker");
    inherited(&mut command, receiver.as_raw_fd());
    let result = command.output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
}

#[test]
#[expect(unsafe_code, reason = "The owned subprocess controls and checks only its own inherited signal state")]
fn terminal_handoff_is_child_only_and_never_enters_its_loader() {
    use tokio::io::AsyncReadExt;
    let executable = std::env::current_exe().unwrap();
    let invoked = std::env::args().next().unwrap();
    if let Some(numbers) =
        std::path::Path::new(&invoked).file_name().unwrap().to_str().unwrap().strip_prefix("owned-pty-child-")
    {
        let mut numbers = numbers.split('-').map(|number| number.parse::<RawFd>().unwrap());
        let descriptor = numbers.next().unwrap();
        for number in numbers {
            // SAFETY: F_GETFD checks only this owned descriptor number and reads no contents.
            assert_eq!(unsafe { libc::fcntl(number, libc::F_GETFD) }, -1, "PTY inherited an ambient descriptor");
            assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EBADF));
        }
        // Both were deliberately inherited by the owned parent, just as with a nohup daemon.
        use nix::sys::signal::{SigSet, Signal};
        let mut inherited = std::mem::MaybeUninit::<libc::sigaction>::uninit();
        // SAFETY: a null action only queries this child's disposition; the successful call
        // initializes the entire output before assume_init, and it changes no process.
        let inherited = unsafe {
            assert_eq!(libc::sigaction(libc::SIGHUP, std::ptr::null(), inherited.as_mut_ptr()), 0);
            inherited.assume_init()
        };
        assert_eq!(inherited.sa_sigaction, libc::SIG_DFL, "PTY child retained an ignored HUP disposition");
        assert!(!SigSet::thread_get_mask().unwrap().contains(Signal::SIGALRM), "PTY child retained a blocked signal");
        assert!(std::env::var_os("APP_VALUE").is_none());
        assert!(std::env::var_os("LD_PRELOAD").is_none());
        rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable).unwrap();
        let values = take(descriptor).unwrap();
        assert_eq!(values.get("APP_VALUE").map(String::as_str), Some("owned-terminal-value"));
        assert_eq!(values.get("LD_PRELOAD").map(String::as_str), Some("owned-loader-path"));
        return;
    }
    if std::env::var_os("PRYX_TEST_TERMINAL_SIGNAL_PARENT").is_none() {
        let mut command = std::process::Command::new(&executable);
        command
            .args(["--exact", "process_environment::tests::terminal_handoff_is_child_only_and_never_enters_its_loader"])
            .env("PRYX_TEST_TERMINAL_SIGNAL_PARENT", "owned");
        use std::os::unix::process::CommandExt;
        // SAFETY: this changes signal state only in this owned subprocess after fork;
        // these libc signal and mask operations are async-signal-safe.
        unsafe {
            command.pre_exec(|| {
                if libc::signal(libc::SIGHUP, libc::SIG_IGN) == libc::SIG_ERR {
                    return Err(io::Error::last_os_error());
                }
                let mut blocked = std::mem::zeroed();
                if libc::sigemptyset(&mut blocked) != 0
                    || libc::sigaddset(&mut blocked, libc::SIGALRM) != 0
                    || libc::sigprocmask(libc::SIG_BLOCK, &blocked, std::ptr::null_mut()) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let result = command.output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let ambient = ambient_descriptors(root.path());
    let socket = Pending::new()
        .unwrap()
        .queue(&BTreeMap::from([
            ("APP_VALUE".into(), "owned-terminal-value".into()),
            ("LD_PRELOAD".into(), "owned-loader-path".into()),
        ]))
        .unwrap();
    let numbers = std::iter::once(socket.as_raw_fd())
        .chain(ambient.iter().map(AsRawFd::as_raw_fd))
        .map(|number| number.to_string())
        .collect::<Vec<_>>()
        .join("-");
    let alias = root.path().join(format!("owned-pty-child-{numbers}"));
    std::os::unix::fs::symlink(executable, &alias).unwrap();
    let argv = vec![
        alias.to_string_lossy().into_owned(),
        "--exact".into(),
        "process_environment::tests::terminal_handoff_is_child_only_and_never_enters_its_loader".into(),
    ];
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut process =
            crate::pty::spawn(&argv, root.path(), &BTreeMap::new(), "OWNED_BOOTSTRAP", Some(&socket), 80, 24).unwrap();
        drop(socket);
        drop(process.input);
        let mut output = Vec::new();
        process.output.read_to_end(&mut output).await.unwrap();
        assert!(process.child.wait().unwrap().success(), "{}", String::from_utf8_lossy(&output));
        assert!(String::from_utf8_lossy(&output).contains("1 passed"));
    });
    for descriptor in &ambient {
        assert_eq!(rustix::io::fcntl_getfd(descriptor).unwrap(), rustix::io::FdFlags::empty());
    }
}
