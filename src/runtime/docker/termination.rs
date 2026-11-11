// SPDX-License-Identifier: FSL-1.1-ALv2
//! Kernel process ancestry and cgroup-owned exec cleanup. Engine replies and status translation
//! belong to the Docker API owner; this module starts no runtime or helper container.
use std::collections::HashMap;

/// SIGKILLs `pid` and every process descended from it, each only if its cgroup still names
/// `container` (a pid may have been reused since the exec began). Returns how many it killed.
pub(super) fn kill_exec_tree(pid: i32, container: &str) -> usize {
    let mut killed = 0;
    for target in descendants(pid, &children_by_parent()) {
        let owned = std::fs::read_to_string(format!("/proc/{target}/cgroup")).is_ok_and(|c| c.contains(container));
        if owned
            && let Some(p) = rustix::process::Pid::from_raw(target)
            && rustix::process::kill_process(p, rustix::process::Signal::KILL).is_ok()
        {
            killed += 1;
        }
    }
    killed
}

/// Every process's children, keyed by parent pid, as /proc shows them now.
pub(super) fn children_by_parent() -> HashMap<i32, Vec<i32>> {
    let mut parents: HashMap<i32, Vec<i32>> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.filter_map(Result::ok) {
            let Ok(child) = entry.file_name().to_string_lossy().parse::<i32>() else { continue };
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{child}/stat")) else { continue };
            // The command name is parenthesised and may hold spaces; fields resume after ')'.
            let Some(after) = stat.rfind(')').map(|i| &stat[i + 1..]) else { continue };
            let Some(ppid) = after.split_whitespace().nth(1).and_then(|p| p.parse::<i32>().ok()) else { continue };
            parents.entry(ppid).or_default().push(child);
        }
    }
    parents
}

/// `root` and everything below it, breadth first. Nothing for 0 or 1: the tree under either is
/// every process on the host, the container's own included, and the cgroup check would let
/// exactly those through. Docker reports pid 0 for an exec it hasn't started yet.
pub(super) fn descendants(root: i32, children: &HashMap<i32, Vec<i32>>) -> Vec<i32> {
    if root <= 1 {
        return Vec::new();
    }
    let mut tree = vec![root];
    let mut i = 0;
    while i < tree.len() {
        if let Some(below) = children.get(&tree[i]) {
            tree.extend(below);
        }
        i += 1;
    }
    tree
}
