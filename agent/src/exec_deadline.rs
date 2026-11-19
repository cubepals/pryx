// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! A node exec's deadline kept by the sandbox's own kernel. Where the node can't reach an exec's
//! processes from the host (gVisor's live in its own kernel), it runs the command under this
//! supervisor: the command gets a process group of its own, and this subreaper owns every
//! descendant, including children that leave the group with setsid. All are retired before exit. The exit status is the command's, or SIGKILL's at the deadline. Not for
//! the agent's process API, which owns its separate command lifecycle.
use brush_core::ExecutionExitCode;
use std::io::Write;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::time::{Duration, Instant};

/// The argument that selects this supervisor: `exec-deadline <milliseconds> -- <argv...>`.
pub const MODE: &str = "exec-deadline";
pub const CLEAN_MODE: &str = "exec-clean-env";
const POLL: Duration = Duration::from_millis(5);

pub fn run(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let millis: u64 = arguments.first().ok_or("Missing exec deadline")?.parse()?;
    let command_arguments = arguments.get(1..).ok_or("Missing deadline command")?;
    if !matches!(command_arguments.first().map(String::as_str), Some("--" | "--secret-env")) {
        return Err("Missing deadline command".into());
    }
    if crate::fuse_namespace::readable(&std::env::current_exe()?) {
        return Err("exec-timeout refuses a readable owner; protection must hold before process creation".into());
    }
    protect_owner(rustix::process::dumpable_behavior()?)?;
    std::io::Write::write_all(&mut std::io::stderr().lock(), crate::exec_completion::READY)?;
    let key = crate::exec_completion::Key::read(&mut std::io::stdin().lock())?;
    rustix::process::set_child_subreaper(rustix::process::Pid::from_raw(1))?;
    // gVisor exposes task stat ancestry, but not /proc/self/task/*/children.
    procfs::process::all_processes()?;
    let mut child = match command(command_arguments)?.stdin(std::process::Stdio::null()).process_group(0).spawn() {
        Ok(child) => child,
        Err(problem) => {
            // Owner protection and subreaper/procfs admission already succeeded. This fixed
            // spawn has no user pre-exec hook; these errors admit no workload or descendants.
            let Some(code) = launch_error_code(&problem) else { return Err(problem.into()) };
            let code = i32::from(u8::from(code));
            let mut stderr = std::io::stderr().lock();
            writeln!(stderr, "Cannot start requested command: {problem}")?;
            key.write(&mut stderr, code, false)?;
            std::process::exit(code);
        }
    };
    let group = rustix::process::Pid::from_child(&child);
    let deadline = Instant::now() + Duration::from_millis(millis);
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            // The command and everything it started in its group, by the sandbox's own kernel.
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
            // The direct child may have left that group; Child still owns its unreaped PID.
            let _ = child.kill();
            break child.wait()?;
        }
        std::thread::sleep(POLL);
    };
    crate::process_retirement::descendants()?;
    let code = status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(libc::SIGKILL));
    key.write(&mut std::io::stderr().lock(), code, timed_out)?;
    std::process::exit(code)
}

// Import the maintained shell's POSIX command-launch statuses rather than copying their numbers.
// Unknown setup/resource failures stay unconfirmed; post-start cleanup is never handled here.
fn launch_error_code(problem: &std::io::Error) -> Option<ExecutionExitCode> {
    match problem.raw_os_error() {
        Some(libc::ENOENT) => Some(ExecutionExitCode::NotFound),
        Some(libc::EACCES | libc::ENOEXEC) => Some(ExecutionExitCode::CannotExecute),
        _ => None,
    }
}

pub(crate) fn protect_owner(behavior: rustix::process::DumpableBehavior) -> Result<(), Box<dyn std::error::Error>> {
    use rustix::process::{DumpableBehavior, dumpable_behavior, set_dumpable_behavior};
    // Linux exec uses fs.suid_dumpable for an unreadable image. Both zero and root-only
    // protect it from the workload's uid from creation; gVisor currently always uses zero.
    if behavior == DumpableBehavior::Dumpable {
        return Err("exec-timeout owner was not protected by its kernel from creation".into());
    }
    // Also suppress root-only core dumps before READY or receiving the private key.
    set_dumpable_behavior(DumpableBehavior::NotDumpable)?;
    if dumpable_behavior()? != DumpableBehavior::NotDumpable {
        return Err("exec-timeout owner could not disable core dumps before receiving its key".into());
    }
    Ok(())
}

/// Ordinary Docker execs use the same trusted environment scrubber without changing their
/// host-side timeout handling. Values remain opaque; only owner-defined variable names select
/// removals, preserving the workload's ordinary environment.
pub fn clean(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    Err(command(arguments)?.exec().into())
}

fn command(arguments: &[String]) -> Result<std::process::Command, Box<dyn std::error::Error>> {
    let (secret, arguments) = if arguments.first().is_some_and(|argument| argument == "--secret-env") {
        (Some(arguments.get(1).ok_or("Missing bootstrap name")?), arguments.get(2..).ok_or("Missing exec command")?)
    } else {
        (None, arguments)
    };
    let arguments =
        if arguments.first().is_some_and(|argument| argument == "--") { &arguments[1..] } else { arguments };
    let executable = arguments.first().ok_or("Missing exec command")?;
    let mut command = std::process::Command::new(executable);
    command.args(&arguments[1..]);
    for (name, _) in std::env::vars_os() {
        if name.as_encoded_bytes().starts_with(crate::contract::SECRET_ENV_PREFIX.as_bytes()) {
            command.env_remove(name);
        }
    }
    if let Some(secret) = secret {
        command.env_remove(secret);
    }
    command.env_remove(crate::contract::MOUNTS_ENV).env_remove(crate::process_policy::POLICY_ENV);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::process::{DumpableBehavior, dumpable_behavior};

    #[test]
    fn only_ordinary_unstarted_command_errors_get_a_launch_completion() {
        for (errno, expected) in [
            (libc::ENOENT, ExecutionExitCode::NotFound),
            (libc::EACCES, ExecutionExitCode::CannotExecute),
            (libc::ENOEXEC, ExecutionExitCode::CannotExecute),
        ] {
            let actual = launch_error_code(&std::io::Error::from_raw_os_error(errno)).unwrap();
            assert_eq!(u8::from(actual), u8::from(expected));
        }
        for errno in [libc::EPERM, libc::EAGAIN, libc::ENOMEM, libc::EIO, libc::EINVAL] {
            assert!(launch_error_code(&std::io::Error::from_raw_os_error(errno)).is_none());
        }
    }

    #[test]
    fn ordinary_dumpability_is_refused_without_changing_the_callers_policy() {
        let before = dumpable_behavior().unwrap();
        assert!(protect_owner(DumpableBehavior::Dumpable).is_err());
        assert_eq!(dumpable_behavior().unwrap(), before);
    }

    #[test]
    fn both_kernel_private_exec_states_disable_dumping_before_key_admission() {
        // PR_SET_DUMPABLE no longer accepts two. Exercise the owner enum classification,
        // and the real kernel transition/readback to zero, inside test-owned processes.
        const STATE: &str = "PRYX_TEST_EXEC_OWNER_DUMPABILITY";
        if let Ok(state) = std::env::var(STATE) {
            let observed = match state.as_str() {
                "private" => DumpableBehavior::NotDumpable,
                "root-only" => DumpableBehavior::DumpableReadableOnlyByRoot,
                _ => panic!("unexpected fixture state"),
            };
            protect_owner(observed).unwrap();
            assert_eq!(dumpable_behavior().unwrap(), DumpableBehavior::NotDumpable);
            return;
        }
        for state in ["private", "root-only"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "exec_deadline::tests::both_kernel_private_exec_states_disable_dumping_before_key_admission",
                ])
                .env(STATE, state)
                .output()
                .unwrap();
            assert!(output.status.success(), "{state}: {}", String::from_utf8_lossy(&output.stderr));
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "the child must run the owned fixture"
            );
        }
    }
}
