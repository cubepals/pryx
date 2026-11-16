// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! A node exec's deadline kept by the sandbox's own kernel. Where the node can't reach an exec's
//! processes from the host (gVisor's live in its own kernel), it runs the command under this
//! supervisor: the command gets a process group of its own, and at the deadline the whole group
//! is killed from inside. The exit status is the command's, or SIGKILL's at the deadline. Not for
//! the agent's own process API, which already ends its commands.
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::time::{Duration, Instant};

/// The argument that selects this supervisor: `exec-deadline <milliseconds> -- <argv...>`.
pub const MODE: &str = "exec-deadline";
const POLL: Duration = Duration::from_millis(5);

pub fn run(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let millis: u64 = arguments.first().ok_or("Missing exec deadline")?.parse()?;
    if arguments.get(1).map(String::as_str) != Some("--") || arguments.len() < 3 {
        return Err("Missing deadline command".into());
    }
    let mut child = std::process::Command::new(&arguments[2]).args(&arguments[3..]).process_group(0).spawn()?;
    let group = rustix::process::Pid::from_child(&child);
    let deadline = Instant::now() + Duration::from_millis(millis);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            // The command and everything it started in its group, by the sandbox's own kernel.
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
            break child.wait()?;
        }
        std::thread::sleep(POLL);
    };
    std::process::exit(status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(libc::SIGKILL)))
}
