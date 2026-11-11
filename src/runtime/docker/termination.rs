// SPDX-License-Identifier: FSL-1.1-ALv2
//! Kernel process ancestry and cgroup-owned exec cleanup. Engine replies and status translation
//! belong to the Docker API owner; this module starts no runtime or helper container.
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
use std::{
    collections::HashMap,
    fs::File,
    os::fd::{AsRawFd, OwnedFd},
    os::unix::fs::MetadataExt,
};

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

pub(super) async fn peer(socket: &str) -> Result<u32, String> {
    let stream = tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, tokio::net::UnixStream::connect(socket))
        .await
        .map_err(|_| visibility("the engine peer timed out"))?
        .map_err(|_| visibility("the engine peer is unavailable"))?;
    let credentials = stream.peer_cred().map_err(|_| visibility("the peer identity is unavailable"))?;
    let pid = credentials.pid().filter(|pid| *pid > 0).ok_or_else(|| visibility("the engine PID is not visible"))?;
    if !single_pid_namespace(pid) || !single_pid_namespace(rustix::process::getpid().as_raw_pid()) {
        return Err(visibility("the engine and node do not share visible host PIDs"));
    }
    if !unique_uid(credentials.uid()) {
        return Err(visibility("the engine peer UID is unmapped or ambiguous"));
    }
    Ok(credentials.uid())
}

fn visibility(reason: &str) -> String {
    format!(
        "exec-pid-visibility is unavailable: {reason}; run pryx on the engine's host with directly visible process identities"
    )
}

fn unique_uid(uid: u32) -> bool {
    // user_namespaces(7): unmapped identities use overflowuid. Do not equate that alias
    // with an engine owner unless every valid UID is mapped identically for this reader.
    let overflow =
        std::fs::read_to_string("/proc/sys/kernel/overflowuid").ok().and_then(|value| value.trim().parse::<u32>().ok());
    if overflow.is_none() {
        return false;
    }
    if overflow != Some(uid) {
        return true;
    }
    let Some(map) = std::fs::read_to_string("/proc/self/uid_map").ok() else { return false };
    let rows = map
        .lines()
        .map(|line| line.split_whitespace().map(str::parse::<u64>).collect::<Result<Vec<_>, _>>())
        .collect::<Result<Vec<_>, _>>();
    rows.is_ok_and(|rows| rows == [vec![0, 0, u64::from(u32::MAX)]])
}

fn single_pid_namespace(pid: i32) -> bool {
    // proc_pid_status(5): NSpid lists identifiers from the procfs mount's PID namespace inward.
    // A single matching identifier for both processes establishes the same visible PID space.
    std::fs::read_to_string(format!("/proc/{pid}/status")).ok().is_some_and(|text| {
        text.lines().find_map(|line| line.strip_prefix("NSpid:")).is_some_and(|value| {
            let mut values = value.split_whitespace();
            values.next().and_then(|v| v.parse::<i32>().ok()) == Some(pid) && values.next().is_none()
        })
    })
}

pub(super) struct Held {
    pub(super) pid: i32,
    fd: OwnedFd,
    frozen: bool,
}

impl Held {
    pub(super) fn capture(pid: i32) -> Option<Self> {
        if pid <= 1 {
            return None;
        }
        let value = Pid::from_raw(pid)?;
        let held = Self { pid, fd: pidfd_open(value, PidfdFlags::empty()).ok()?, frozen: false };
        held.alive().then_some(held)
    }
    pub(super) fn alive(&self) -> bool {
        // proc_pid_fdinfo(5): a pidfd reports Pid -1 after its process exits. The descriptor,
        // rather than a reusable numeric PID, remains the authority for every signal.
        std::fs::read_to_string(format!("/proc/self/fdinfo/{}", self.fd.as_raw_fd())).ok().and_then(|text| {
            text.lines().find_map(|line| line.strip_prefix("Pid:")).and_then(|value| value.trim().parse::<i32>().ok())
        }) == Some(self.pid)
    }
    pub(super) fn freeze(&mut self) -> bool {
        if !self.frozen {
            self.frozen = pidfd_send_signal(&self.fd, Signal::STOP).is_ok();
        }
        self.frozen && self.alive()
    }
    pub(super) fn stopped(&self) -> bool {
        self.frozen
            && self.alive()
            && std::fs::read_to_string(format!("/proc/{}/stat", self.pid))
                .ok()
                .and_then(|text| text.rsplit_once(") ").map(|(_, fields)| fields.starts_with(['T', 't'])))
                .unwrap_or(false)
    }
    pub(super) fn kill(&mut self) -> bool {
        if !self.frozen {
            return false;
        }
        self.frozen = false;
        pidfd_send_signal(&self.fd, Signal::KILL).is_ok()
    }
}
impl Drop for Held {
    fn drop(&mut self) {
        // Cancellation cannot strand a process we already proved owned and stopped.
        self.kill();
    }
}

pub(super) fn process_tree(root: i32) -> std::collections::BTreeSet<i32> {
    descendants(root, &children_by_parent()).into_iter().collect()
}

pub(super) struct Namespace(File);

impl Namespace {
    pub(super) fn capture(init: i32) -> Option<Self> {
        // proc_pid_status(5): a private namespace's init is the final NSpid 1.
        // Its descriptor pins that namespace even after the numeric PID disappears.
        let status = std::fs::read_to_string(format!("/proc/{init}/status")).ok()?;
        let pids = status.lines().find_map(|line| line.strip_prefix("NSpid:"))?.split_whitespace().collect::<Vec<_>>();
        if pids.len() < 2 || pids.last().copied() != Some("1") {
            return None;
        }
        let namespace = Self(File::open(format!("/proc/{init}/ns/pid")).ok()?);
        (!namespace.owns(rustix::process::getpid().as_raw_pid())).then_some(namespace)
    }

    pub(super) fn owns(&self, pid: i32) -> bool {
        let ours = self.0.metadata();
        let theirs = File::open(format!("/proc/{pid}/ns/pid")).and_then(|file| file.metadata());
        ours.ok()
            .zip(theirs.ok())
            .is_some_and(|(ours, theirs)| ours.dev() == theirs.dev() && ours.ino() == theirs.ino())
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    parent: i32,
    ticks: u64,
    uid: u32,
}

fn stamp(pid: i32) -> Option<Stamp> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let uids = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let [uid, effective, saved, filesystem] = uids.as_slice() else { return None };
    if uid != effective || uid != saved || uid != filesystem {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(") ")?;
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    // proc_pid_stat(5): after field 2 (comm), field 4 is PPID and field 22 is starttime.
    Some(Stamp { parent: fields.get(1)?.parse().ok()?, ticks: fields.get(19)?.parse().ok()?, uid: *uid })
}

pub(super) fn shim_owned(pid: i32, owner: u32, container: &str) -> bool {
    let Some(first) = stamp(pid) else { return false };
    // Docker's nonzero workload UID cannot map to the engine's namespace-root owner.
    // A workload cannot forge that distinct host identity by changing its argv.
    if first.uid == owner {
        return false;
    }
    let mut current = pid;
    let mut seen = std::collections::BTreeSet::new();
    let mut chain = Vec::new();
    while current > 1 && seen.insert(current) {
        let Some(identity) = stamp(current) else { return false };
        let parent = identity.parent;
        let owned = identity.uid == owner && shim_argument(current, container);
        chain.push((current, identity));
        if owned {
            return chain.iter().all(|(pid, saved)| stamp(*pid).as_ref() == Some(saved));
        }
        current = parent;
    }
    false
}

fn shim_argument(pid: i32, container: &str) -> bool {
    // containerd v2.4.2 pkg/shim/shim.go registers the task ID as the id command-line flag.
    const TASK_ID_OPTION: &[u8] = b"-id";
    std::fs::read(format!("/proc/{pid}/cmdline")).ok().is_some_and(|bytes| {
        let args = bytes.split(|b| *b == 0).collect::<Vec<_>>();
        args.windows(2).any(|pair| pair[0] == TASK_ID_OPTION && pair[1] == container.as_bytes())
    })
}

pub(super) enum Ownership {
    Namespace(Namespace),
    Shim { uid: u32, container: String },
}
impl Ownership {
    pub(super) fn capture(init: i32, uid: u32, container: &str, docker: bool) -> Option<Self> {
        Namespace::capture(init).map(Self::Namespace).or_else(|| {
            (docker && shim_owned(init, uid, container)).then(|| Self::Shim { uid, container: container.into() })
        })
    }
    pub(super) fn owns(&self, pid: i32) -> bool {
        match self {
            Self::Namespace(namespace) => namespace.owns(pid),
            Self::Shim { uid, container } => shim_owned(pid, *uid, container),
        }
    }
}

#[cfg(test)]
mod kernel_tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Child, Command, Stdio};

    fn idle_child(container: Option<&str>) -> Child {
        let mut command = Command::new("sh");
        command.args(["-c", "read -r input"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(container) = container {
            command.args(["-id", container]);
        }
        command.spawn().unwrap()
    }

    #[test]
    fn forged_arguments_from_a_workload_identity_never_authorize_signals() {
        let container = format!("fixture-{}", rustix::process::getpid().as_raw_pid());
        let mut child = idle_child(Some(&container));
        let mut process = Held::capture(child.id() as i32).unwrap();
        let uid = rustix::process::geteuid().as_raw();
        assert!(!shim_owned(process.pid, uid, &container));
        assert!(!shim_owned(process.pid, uid.checked_add(1).unwrap(), &container));
        assert!(
            Namespace::capture(process.pid).is_none(),
            "forged argv cannot make a host process a private namespace init"
        );
        assert!(!process.kill(), "an unverified capture is never armed");
        assert!(process.alive() && child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn an_exited_process_descriptor_cannot_be_signalled() {
        let mut child = idle_child(None);
        let mut process = Held::capture(child.id() as i32).unwrap();
        drop(child.stdin.take());
        child.wait().unwrap();
        assert!(!process.alive());
        assert!(!process.freeze());
        assert!(!process.kill());
    }

    #[test]
    fn dropping_a_verified_frozen_process_retires_its_exact_descriptor() {
        let mut child = idle_child(None);
        let mut process = Held::capture(child.id() as i32).unwrap();
        assert!(process.freeze());
        drop(process);
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
    }

    #[tokio::test]
    async fn a_real_socket_peer_reports_pid_visibility_and_its_original_uid() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.sock");
        let _listener = tokio::net::UnixListener::bind(&path).unwrap();
        let answer = peer(path.to_str().unwrap()).await;
        let uid = rustix::process::geteuid().as_raw();
        if single_pid_namespace(rustix::process::getpid().as_raw_pid()) && unique_uid(uid) {
            assert_eq!(answer.unwrap(), uid);
        } else {
            assert!(answer.err().unwrap().contains("exec-pid-visibility"));
        }
    }
}
