// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real exec-deadline subprocesses retain command status and retire detached descendants.
//! Engine transport and whole-sandbox retirement belong to the daemon tests and hosted CI.
use std::io::Write;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn the_deadline_retires_children_that_started_another_session() {
    detached("sleep 60", 1000, 128 + libc::SIGKILL);
}

#[test]
fn a_finished_command_does_not_leave_detached_children() {
    detached("exit 23", 5000, 23);
}

fn detached(end: &str, deadline: u64, exit: i32) {
    let (_installation, executable) = protected_executable();
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("detached.pid");
    let script = format!(
        "setsid sh -c 'echo $$ > \"$1\"; exec sleep 60' sh \"$1\" & \
         while [ ! -s \"$1\" ]; do sleep 0.01; done; {end}"
    );
    let mut wrapper = Command::new(&executable)
        .args(["exec-deadline", &deadline.to_string(), "--", "sh", "-c", &script, "sh"])
        .arg(&pid_file)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (key, input) = pryx_agent::exec_completion::Key::fresh().unwrap();
    wrapper.stdin.take().unwrap().write_all(&input).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while wrapper.try_wait().unwrap().is_none() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = wrapper.try_wait().unwrap();
    if status.is_none() {
        wrapper.kill().unwrap();
    }
    let output = wrapper.wait_with_output().unwrap();
    let pid: i32 = std::fs::read_to_string(pid_file)
        .unwrap_or_else(|error| {
            panic!(
                "Detached fixture not admitted: {error}; owner={:?}; {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )
        })
        .trim()
        .parse()
        .unwrap();
    let child = rustix::process::Pid::from_raw(pid).unwrap();
    let retired = !std::path::Path::new(&format!("/proc/{pid}")).exists();
    // Keep a failing regression from leaking its deliberately detached fixture.
    if !retired {
        let _ = rustix::process::kill_process(child, rustix::process::Signal::KILL);
    }
    assert!(status.is_some(), "deadline wrapper did not end");
    assert_eq!(output.status.code(), Some(exit), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.status.signal(), None);
    assert!(retired, "setsid descendant outlived the command's owner");
    assert!(
        key.verify(&mut output.stderr.clone(), output.status.code().map(i64::from)).is_some(),
        "cleanup completion must authenticate"
    );
}

#[test]
fn the_deadline_kills_a_direct_child_that_left_its_original_group() {
    let (_installation, executable) = protected_executable();
    use std::os::unix::process::CommandExt;
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("direct.pid");
    let mut wrapper = Command::new(&executable)
        .args([
            "exec-deadline", "1000", "--", "python3", "-c",
            "import os,sys,time; os.setpgid(0,os.getpgid(os.getppid())); open(sys.argv[1],'w').write(str(os.getpid())); time.sleep(60)",
        ])
        .arg(&pid_file)
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (key, input) = pryx_agent::exec_completion::Key::fresh().unwrap();
    wrapper.stdin.take().unwrap().write_all(&input).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while wrapper.try_wait().unwrap().is_none() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    let finished = wrapper.try_wait().unwrap().is_some();
    if !finished {
        // The changed child group is this fixture's private wrapper group, never the test's.
        let _ = rustix::process::kill_process_group(
            rustix::process::Pid::from_child(&wrapper),
            rustix::process::Signal::KILL,
        );
    }
    let output = wrapper.wait_with_output().unwrap();
    let pid: i32 = std::fs::read_to_string(pid_file)
        .unwrap_or_else(|error| {
            panic!(
                "Direct fixture not admitted: {error}; owner={:?}; {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )
        })
        .trim()
        .parse()
        .unwrap();
    assert!(finished, "a changed direct-child group hung child.wait");
    assert!(
        key.verify(&mut output.stderr.clone(), output.status.code().map(i64::from)).is_some(),
        "changed-group cleanup must authenticate"
    );
    assert_eq!(output.status.code(), Some(128 + libc::SIGKILL), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists(), "the changed-group child remained");
}

#[test]
fn a_same_uid_sibling_cannot_read_the_key_or_authenticate_an_owner_it_kills() {
    let (_installation, executable) = protected_executable();
    let dir = tempfile::tempdir().unwrap();
    let escaped = dir.path().join("escaped.pid");
    let direct = dir.path().join("direct.pid");
    let script = "echo $$ > \"$2\"; setsid sh -c 'echo $$ > \"$1\"; exec sleep 60' sh \"$1\" & while [ ! -s \"$1\" ]; do sleep 0.01; done; sleep 60";
    let mut owner = Command::new(&executable)
        .args(["exec-deadline", "10000", "--", "sh", "-c", script, "sh"])
        .arg(&escaped)
        .arg(&direct)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (key, input) = pryx_agent::exec_completion::Key::fresh().unwrap();
    owner.stdin.take().unwrap().write_all(&input).unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    while !escaped.exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    let attack = Command::new("python3").args(["-c",
        "import ctypes,os,signal,sys; p=int(sys.argv[1]);\nfor suffix in ['mem','environ','fd/2']:\n try:\n  fd=os.open('/proc/'+str(p)+'/'+suffix,os.O_RDONLY); os.close(fd); raise RuntimeError('owner file unexpectedly readable')\n except PermissionError: pass\nassert ctypes.CDLL(None).tgkill(p,p,signal.SIGKILL)==0",
        &owner.id().to_string(),
    ]).output().unwrap();
    if owner.try_wait().unwrap().is_none() {
        let _ = owner.kill();
    }
    let mut survived = false;
    for path in [&escaped, &direct] {
        if let Ok(pid) =
            std::fs::read_to_string(path).and_then(|pid| pid.trim().parse::<i32>().map_err(std::io::Error::other))
        {
            survived |= path == &escaped && std::path::Path::new(&format!("/proc/{pid}")).exists();
            if let Some(pid) = rustix::process::Pid::from_raw(pid) {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
            }
        }
    }
    let output = owner.wait_with_output().unwrap();
    assert!(attack.status.success(), "{}", String::from_utf8_lossy(&attack.stderr));
    assert!(
        survived,
        "the fixture must demonstrate why engine retirement is needed; owner={:?}; {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        key.verify(&mut output.stderr.clone(), output.status.code().map(i64::from)).is_none(),
        "an externally killed owner cannot attest cleanup"
    );
}

#[test]
fn every_trusted_exec_launcher_removes_bootstrap_material_and_keeps_user_env() {
    let (_installation, executable) = protected_executable();
    let (_, material) = pryx_agent::auth::SigningKey::generate().unwrap();
    let name = format!("{}fixture", pryx_agent::contract::SECRET_ENV_PREFIX);
    let probe = format!(
        "import os; assert not any(k.startswith({:?}) for k in os.environ); assert {:?} not in os.environ; assert {:?} not in os.environ; assert os.environ['ORDINARY_FIXTURE']=='kept'; print('bootstrap absent')",
        pryx_agent::contract::SECRET_ENV_PREFIX,
        pryx_agent::contract::MOUNTS_ENV,
        pryx_agent::process_policy::POLICY_ENV
    );
    for timed in [false, true] {
        let (key, input) = pryx_agent::exec_completion::Key::fresh().unwrap();
        let mut command = Command::new(&executable);
        if timed {
            command.args(["exec-deadline", "5000", "--"]);
        } else {
            command.arg("exec-clean-env");
        }
        let mut child = command
            .args(["python3", "-c", &probe])
            .env(&name, &material)
            .env(pryx_agent::contract::MOUNTS_ENV, "fixture")
            .env(pryx_agent::process_policy::POLICY_ENV, "fixture")
            .env("ORDINARY_FIXTURE", "kept")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if timed {
            child.stdin.take().unwrap().write_all(&input).unwrap();
        } else {
            drop(child.stdin.take());
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "trusted launcher failed: {:?}; {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"bootstrap absent\n");
        if timed {
            assert!(key.verify(&mut output.stderr.clone(), Some(0)).is_some());
        }
    }
}

fn protected_executable() -> (std::os::fd::OwnedFd, std::path::PathBuf) {
    use rustix::fs::{MemfdFlags, Mode, SealFlags};
    use std::os::fd::AsRawFd;
    let flags = MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING;
    let fd = match rustix::fs::memfd_create("pryx-exec-owner-fixture", flags | MemfdFlags::EXEC) {
        Err(rustix::io::Errno::INVAL) => rustix::fs::memfd_create("pryx-exec-owner-fixture", flags),
        other => other,
    }
    .unwrap();
    let mut copy = std::fs::File::from(fd);
    std::io::copy(&mut std::fs::File::open(env!("CARGO_BIN_EXE_pryx-agent")).unwrap(), &mut copy).unwrap();
    rustix::fs::fchmod(&copy, Mode::XUSR | Mode::XGRP | Mode::XOTH).unwrap();
    rustix::fs::fcntl_add_seals(&copy, SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE | SealFlags::SEAL)
        .unwrap();
    let path = std::path::PathBuf::from(format!("/proc/{}/fd/{}", std::process::id(), copy.as_raw_fd()));
    (copy.into(), path)
}
