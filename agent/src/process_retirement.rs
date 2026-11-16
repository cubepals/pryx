// SPDX-License-Identifier: FSL-1.1-ALv2
//! Final descendant retirement by a trusted subreaper after it closes command admission.
//! This owns only children of the calling process, never unrelated processes or a live agent
//! that still serves other commands. Callers establish subreaper status before spawning.

/// A killed parent hands its still-live descendants to this subreaper. Kill and reap direct
/// children until the kernel reports none: session changes cannot escape that ancestry, and a
/// child PID cannot be reused while it is ours and has not been reaped.
pub fn descendants() -> Result<(), Box<dyn std::error::Error>> {
    loop {
        for pid in direct_children()? {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
        }
        match rustix::process::wait(
            rustix::process::WaitOptions::NOHANG
                | rustix::process::WaitOptions::from_bits_retain(libc::__WALL.cast_unsigned()),
        ) {
            Err(rustix::io::Errno::CHILD) => return Ok(()),
            Err(rustix::io::Errno::INTR) | Ok(Some(_)) => continue,
            Err(error) => return Err(error.into()),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    }
}

fn direct_children() -> Result<Vec<rustix::process::Pid>, Box<dyn std::error::Error>> {
    let mut children = Vec::new();
    let mut supported = false;
    if let Ok(tasks) = std::fs::read_dir("/proc/self/task") {
        for task in tasks.flatten() {
            if let Ok(owned) = std::fs::read_to_string(task.path().join("children")) {
                supported = true;
                children.extend(
                    owned
                        .split_whitespace()
                        .filter_map(|pid| pid.parse().ok())
                        .filter_map(rustix::process::Pid::from_raw),
                );
            }
        }
    }
    if supported {
        return Ok(children);
    }
    // gVisor has no task children file; ordinary supervisors retain their Landlock-scoped path.
    let own = rustix::process::getpid().as_raw_pid();
    for process in procfs::process::all_processes()?.flatten() {
        if let Ok(stat) = process.stat()
            && stat.ppid == own
            && let Some(pid) = rustix::process::Pid::from_raw(stat.pid)
        {
            children.push(pid);
        }
    }
    Ok(children)
}

/// A separate challenge from the running agent's isolation observation: it is acknowledged only
/// after command admission and the runtime have ended, and the kernel reports no child remains.
pub const NONCE_HEADER: &str = "x-pryx-retirement-nonce";
pub const CHECKPOINT: &str = ".pryx-agent/retired.json";
const CONTEXT: &str = "pryx process retirement v1";

#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Retired {
    pub nonce: String,
    pub sandbox: String,
    pub epoch: u64,
    pub pid: u32,
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct Proof {
    pub retired: Retired,
    pub signature: String,
}

fn bytes(retired: &Retired) -> Vec<u8> {
    serde_json::to_vec(&(CONTEXT, retired)).expect("fixed retirement serializes")
}

/// Called only after the executable has dropped its runtime and retired all descendants.
pub fn checkpoint(agent: &crate::context::Agent) -> std::io::Result<()> {
    let Some(nonce) = agent.retirement_nonce.lock().unwrap().clone() else { return Ok(()) };
    let proof = agent.as_identity(|sandbox, epoch, key, _| {
        let retired = Retired { nonce, sandbox: sandbox.into(), epoch, pid: std::process::id() };
        let signature = key.sign_bytes(&bytes(&retired));
        Proof { retired, signature }
    });
    crate::state::write(&agent.data, CHECKPOINT, &proof, true)
}

pub fn verify(proof: &Proof, key: &str, nonce: &str, sandbox: &str, epoch: u64, pid: u32) -> bool {
    crate::process_observation::nonce_valid(nonce)
        && proof.retired.nonce == nonce
        && proof.retired.sandbox == sandbox
        && proof.retired.epoch == epoch
        && proof.retired.pid == pid
        && crate::auth::verify_bytes(key, &bytes(&proof.retired), &proof.signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_retirement_proof_cannot_be_forged_replayed_or_reassigned() {
        let (key, _) = crate::auth::SigningKey::generate().unwrap();
        let retired = Retired { nonce: uuid::Uuid::new_v4().to_string(), sandbox: "one".into(), epoch: 2, pid: 9 };
        let mut proof = Proof { signature: key.sign_bytes(&bytes(&retired)), retired };
        let valid = |proof: &Proof, nonce: &str, sandbox: &str, epoch, pid| {
            verify(proof, &key.public_key(), nonce, sandbox, epoch, pid)
        };
        assert!(valid(&proof, &proof.retired.nonce, "one", 2, 9));
        assert!(!valid(&proof, &uuid::Uuid::new_v4().to_string(), "one", 2, 9));
        assert!(!valid(&proof, &proof.retired.nonce, "two", 2, 9));
        assert!(!valid(&proof, &proof.retired.nonce, "one", 3, 9));
        assert!(!valid(&proof, &proof.retired.nonce, "one", 2, 10));
        proof.retired.epoch = 3;
        assert!(!valid(&proof, &proof.retired.nonce, "one", 3, 9));
    }

    #[test]
    fn clone_children_with_no_sigchld_and_their_descendants_are_reaped() {
        const FIXTURE: &str = "PRYX_TEST_RETIREMENT_CLONE";
        const NAME: &str = "process_retirement::tests::clone_children_with_no_sigchld_and_their_descendants_are_reaped";
        if std::env::var_os(FIXTURE).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", NAME, "--test-threads", "1"])
                .env(FIXTURE, "1")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        rustix::process::set_child_subreaper(rustix::process::Pid::from_raw(1)).unwrap();
        let (read, write) = rustix::pipe::pipe().unwrap();
        use std::os::fd::AsRawFd;
        // SAFETY: clone creates a fork-like child on its copied stack. That child and its fork
        // call only async-signal-safe syscalls, touch no allocator/lock, and never return to Rust.
        #[expect(unsafe_code, reason = "the regression needs a Linux clone child whose exit signal is zero")]
        let clone = unsafe {
            let clone = libc::syscall(libc::SYS_clone, 0, 0, 0, 0, 0);
            if clone == 0 {
                libc::close(read.as_raw_fd());
                let grandchild = libc::fork();
                if grandchild < 0 {
                    libc::_exit(libc::EXIT_FAILURE);
                }
                if grandchild > 0 {
                    let bytes = grandchild.to_ne_bytes();
                    libc::write(write.as_raw_fd(), bytes.as_ptr().cast(), bytes.len());
                }
                loop {
                    libc::pause();
                }
            }
            clone
        };
        assert!(clone > 0, "a non-SIGCHLD clone child starts");
        drop(write);
        let mut bytes = [0u8; std::mem::size_of::<libc::pid_t>()];
        let mut count = 0;
        while count < bytes.len() {
            let read = rustix::io::read(&read, &mut bytes[count..]).unwrap();
            assert!(read > 0, "the clone fixture reports its descendant");
            count += read;
        }
        let grandchild = rustix::process::Pid::from_raw(libc::pid_t::from_ne_bytes(bytes)).unwrap();
        let child = rustix::process::Pid::from_raw(clone as libc::pid_t).unwrap();
        let child_fd = rustix::process::pidfd_open(child, rustix::process::PidfdFlags::empty()).unwrap();
        let grandchild_fd = rustix::process::pidfd_open(grandchild, rustix::process::PidfdFlags::empty()).unwrap();
        descendants().unwrap();
        let all = rustix::process::WaitOptions::from_bits_retain(libc::__WALL.cast_unsigned());
        let remaining = rustix::process::wait(all | rustix::process::WaitOptions::NOHANG);
        if !matches!(remaining, Err(rustix::io::Errno::CHILD)) {
            let _ = rustix::process::pidfd_send_signal(&child_fd, rustix::process::Signal::KILL);
            let _ = rustix::process::pidfd_send_signal(&grandchild_fd, rustix::process::Signal::KILL);
            while rustix::process::wait(all).is_ok() {}
        }
        assert!(matches!(remaining, Err(rustix::io::Errno::CHILD)), "all clone children were reaped");
    }
}
