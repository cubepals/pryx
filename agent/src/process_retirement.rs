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
