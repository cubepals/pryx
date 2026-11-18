// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! pryxd's own memory never becomes executable once it could be written: the kernel's
//! memory-deny-write-execute (`prctl(PR_SET_MDWE)`), set on the daemon itself as `pryxd serve`
//! starts, and not passed on to what it starts (`PR_MDWE_NO_INHERIT`, Linux 6.6). It stands in
//! for the unit's MemoryDenyWriteExecute, a system call filter every process the unit starts
//! inherits, process sandboxes included, which refuses their just-in-time compilers. pryxd itself
//! compiles no code at run time: no JIT runs in its process (the wasm tier's runtime is a sidecar
//! of its own).
//!
//! Where the kernel can't give it without passing it on, it is off and says why; nothing else
//! changes. Not for sandboxes, which keep their own policy.

/// Holds this process to memory-deny-write-execute without passing it on to its children, or
/// says why the kernel couldn't. Once set it can't be cleared, which is the point.
pub fn deny_write_execute() -> Result<(), String> {
    let flags = libc::c_ulong::from(libc::PR_MDWE_REFUSE_EXEC_GAIN | libc::PR_MDWE_NO_INHERIT);
    let zero: libc::c_ulong = 0;
    // SAFETY: PR_SET_MDWE reads its integer arguments only, the last three of which must be zero.
    #[expect(unsafe_code, reason = "prctl(2) has no safe wrapper for PR_SET_MDWE")]
    let set = unsafe { libc::prctl(libc::PR_SET_MDWE, flags, zero, zero, zero) };
    if set == 0 {
        return Ok(());
    }
    let problem = std::io::Error::last_os_error();
    Err(match problem.raw_os_error() {
        Some(libc::EINVAL) => {
            "this kernel has no memory-deny-write-execute that children don't inherit (Linux 6.6 or later)".into()
        }
        _ => format!("prctl(PR_SET_MDWE): {problem}"),
    })
}

/// The flags memory-deny-write-execute holds this process to: none where it isn't set.
#[cfg(test)]
fn held() -> libc::c_int {
    let zero: libc::c_ulong = 0;
    // SAFETY: PR_GET_MDWE takes no pointer, and its four arguments must be zero.
    #[expect(unsafe_code, reason = "prctl(2) has no safe wrapper for PR_GET_MDWE")]
    unsafe {
        libc::prctl(libc::PR_GET_MDWE, zero, zero, zero, zero)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which side of the test a process is: the one that sets it, or the child that must not have it.
    const SIDE: &str = "PRYX_TEST_WRITE_EXECUTE";
    const NAME: &str = "write_execute::tests::this_process_refuses_writable_code_and_its_children_dont";

    /// Whether an anonymous mapping both writable and executable can be made here.
    fn writable_code() -> bool {
        // SAFETY: a fresh anonymous mapping, unmapped at once; nothing reads or runs it.
        #[expect(unsafe_code, reason = "mmap(2) is the question being asked")]
        unsafe {
            let length = rustix::param::page_size();
            let prot = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
            let mapped = libc::mmap(std::ptr::null_mut(), length, prot, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0);
            if mapped == libc::MAP_FAILED {
                return false;
            }
            libc::munmap(mapped, length);
            true
        }
    }

    fn this_test(side: &str) -> std::process::ExitStatus {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture", "--test-threads", "1"])
            .env(SIDE, side)
            .status()
            .unwrap()
    }

    /// Runs in processes of its own, since the setting can't be undone.
    #[test]
    fn this_process_refuses_writable_code_and_its_children_dont() {
        match std::env::var(SIDE).as_deref() {
            Ok("set") => {
                if let Err(reason) = deny_write_execute() {
                    // Older kernels (CI's are newer) report it off; the daemon says why.
                    eprintln!("{reason}");
                    assert!(reason.contains("Linux 6.6"), "{reason}");
                    return;
                }
                let flags = libc::PR_MDWE_REFUSE_EXEC_GAIN | libc::PR_MDWE_NO_INHERIT;
                assert_eq!(held(), flags.cast_signed());
                assert!(!writable_code(), "a writable, executable mapping was made");
                assert!(this_test("child").success(), "a child of the daemon inherited it");
            }
            Ok("child") => {
                assert_eq!(held(), 0);
                assert!(writable_code(), "a child can't make writable code");
            }
            _ => assert!(this_test("set").success()),
        }
    }
}
