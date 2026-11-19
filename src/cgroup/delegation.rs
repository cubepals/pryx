// SPDX-License-Identifier: FSL-1.1-ALv2
//! The daemon's delegated subtree with the memory, cpu and pids controllers on for its children.
//! cgroup v2 lets a cgroup hand controllers to its children only while it holds no process of
//! its own, so the daemon lives in a leaf of its subtree, `pryx-daemon`: one systemd made for it
//! (`DelegateSubgroup=pryx-daemon`), one an earlier pryxd moved into, or one it moves into here
//! when it is its cgroup's only process. It never moves another process. Found by looking, never
//! configured; what is missing is reported with what would turn it on. Not a sandbox's own limit
//! files: those are `limits.rs`.
use super::{Unavailable, members, own_cgroup, unavailable, writable};
use std::{
    fs, io,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

/// The leaf the daemon's own processes live in, below the root of its subtree.
pub(crate) const DAEMON: &str = "pryx-daemon";

/// The controllers a sandbox's limits need, in the kernel's names (cgroup-v2.rst, "Controllers").
pub(crate) const MEMORY: &str = "memory";
pub(crate) const CPU: &str = "cpu";
pub(crate) const PIDS: &str = "pids";
const WANTED: [&str; 3] = [MEMORY, CPU, PIDS];

/// What turns the controllers on, where something is missing.
pub(crate) const DELEGATE_ACTION: &str =
    "Run pryxd as root, or as a systemd service with Delegate=yes (systemd 252 or later delegates cpu to users)";
const ALONE_ACTION: &str = "Start pryxd as its cgroup's only process, or in that cgroup's pryx-daemon subgroup (systemd DelegateSubgroup=pryx-daemon, systemd 254 or later)";

/// The daemon's subtree with the controllers it could turn on for its children.
#[derive(Clone, Debug)]
pub(crate) struct Delegated {
    /// The subtree's root: each sandbox's cgroup is a child of it, beside the daemon's leaf.
    pub(crate) root: PathBuf,
    /// Of memory, cpu and pids, those its children now have, in that order.
    pub(crate) controllers: Vec<&'static str>,
}

impl Delegated {
    pub(crate) fn has(&self, controller: &str) -> bool {
        self.controllers.contains(&controller)
    }
}

/// The root of the subtree a daemon in cgroup `own` keeps sandboxes in: `own` itself, or its
/// parent where the daemon already lives in its leaf.
pub(crate) fn subtree_root(own: &Path) -> PathBuf {
    match (own.file_name(), own.parent()) {
        (Some(name), Some(parent)) if name == DAEMON => parent.to_path_buf(),
        _ => own.to_path_buf(),
    }
}

/// Turns memory, cpu and pids on for this daemon's children where its cgroup has them to give,
/// moving the daemon into its leaf first if it is not there yet and is its cgroup's only process.
pub(crate) fn delegate() -> Result<Delegated, Unavailable> {
    delegate_at(&own()?, std::process::id())
}

/// This process's own cgroup v2 directory.
fn own() -> Result<PathBuf, Unavailable> {
    let read = |path: &str| {
        fs::read_to_string(path)
            .map_err(|error| unavailable(format!("{path}: {error}"), "Run on Linux with /proc mounted"))
    };
    own_cgroup(&read("/proc/self/cgroup")?, &read("/proc/self/mountinfo")?)
}

/// Whether `root` and its subtree_control file are this caller's to write.
fn check_delegated(root: &Path) -> Result<(), Unavailable> {
    if writable(root) && rustix::fs::access(root.join("cgroup.subtree_control"), rustix::fs::Access::WRITE_OK).is_ok() {
        return Ok(());
    }
    Err(unavailable(format!("cgroup {} is not delegated to this process", root.display()), DELEGATE_ACTION))
}

/// What `delegate` would find for this caller, with nothing moved or turned on: the controllers
/// its subtree's root has to give. For `pryxd probe`, which isn't the daemon.
pub(crate) fn inspect() -> Result<Delegated, Unavailable> {
    let root = subtree_root(&own()?);
    check_delegated(&root)?;
    let controllers = fs::read_to_string(root.join("cgroup.controllers")).map_err(|error| {
        unavailable(format!("{}: {error}", root.join("cgroup.controllers").display()), DELEGATE_ACTION)
    })?;
    Ok(Delegated { controllers: listed(&controllers), root })
}

/// The controllers a cgroup's interface file lists, of those a sandbox's limits need.
fn listed(text: &str) -> Vec<&'static str> {
    WANTED.into_iter().filter(|wanted| text.split_whitespace().any(|name| name == *wanted)).collect()
}

/// `delegate` for a daemon with process ID `pid` in cgroup `own`.
fn delegate_at(own: &Path, pid: u32) -> Result<Delegated, Unavailable> {
    let root = subtree_root(own);
    check_delegated(&root)?;
    let control = root.join("cgroup.subtree_control");
    let interface = |name: &str| {
        fs::read_to_string(root.join(name))
            .map_err(|error| unavailable(format!("{}: {error}", root.join(name).display()), DELEGATE_ACTION))
    };
    let available = listed(&interface("cgroup.controllers")?);
    if !available.contains(&MEMORY) && !available.contains(&CPU) {
        return Err(unavailable(
            format!(
                "cgroup {} has neither the memory nor the cpu controller to give (cgroup.controllers: {})",
                root.display(),
                interface("cgroup.controllers")?.trim()
            ),
            DELEGATE_ACTION,
        ));
    }
    let enabled = listed(&interface("cgroup.subtree_control")?);
    let missing: Vec<&str> = available.iter().copied().filter(|name| !enabled.contains(name)).collect();
    if !missing.is_empty() {
        if own == root {
            enter_leaf(&root, pid)?;
        }
        let change: Vec<String> = missing.iter().map(|name| format!("+{name}")).collect();
        fs::write(&control, change.join(" ")).map_err(|error| {
            let busy = error.raw_os_error() == Some(libc::EBUSY);
            unavailable(
                format!("turning on {} for {}'s children: {error}", missing.join(" "), root.display()),
                if busy { ALONE_ACTION } else { DELEGATE_ACTION },
            )
        })?;
    }
    Ok(Delegated { root, controllers: available })
}

/// Moves this daemon into its leaf, only when it is the cgroup's one process: a child it started
/// a moment ago (a probe) is waited for; any other process is never moved.
fn enter_leaf(root: &Path, pid: u32) -> Result<(), Unavailable> {
    const WAIT: std::time::Duration = std::time::Duration::from_secs(2);
    const POLL: std::time::Duration = std::time::Duration::from_millis(20);
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let others: Vec<u32> = members(root)
            .map_err(|error| unavailable(format!("{}: {error}", root.join("cgroup.procs").display()), DELEGATE_ACTION))?
            .into_iter()
            .filter(|member| *member != pid)
            .collect();
        if others.is_empty() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(unavailable(
                format!("cgroup {} holds {} process(es) besides this daemon", root.display(), others.len()),
                ALONE_ACTION,
            ));
        }
        std::thread::sleep(POLL);
    }
    let leaf = root.join(DAEMON);
    match fs::DirBuilder::new().mode(0o755).create(&leaf) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(unavailable(format!("making {}: {error}", leaf.display()), DELEGATE_ACTION)),
    }
    fs::write(leaf.join("cgroup.procs"), pid.to_string())
        .map_err(|error| unavailable(format!("moving this daemon into {}: {error}", leaf.display()), DELEGATE_ACTION))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subtree_root_is_the_daemons_cgroup_or_the_parent_of_its_leaf() {
        let unit = Path::new("/sys/fs/cgroup/system.slice/pryxd.service");
        assert_eq!(subtree_root(unit), unit);
        assert_eq!(subtree_root(&unit.join(DAEMON)), unit);
        assert_eq!(subtree_root(&unit.join("pryx-0123456789abcdef")), unit.join("pryx-0123456789abcdef"));
    }

    #[test]
    fn only_the_controllers_limits_need_are_taken_from_an_interface_file() {
        assert_eq!(listed("cpuset cpu io memory hugetlb pids rdma misc\n"), [MEMORY, CPU, PIDS]);
        assert_eq!(listed("memory pids\n"), [MEMORY, PIDS]);
        assert_eq!(listed("cpuset io\n"), Vec::<&str>::new());
        assert_eq!(listed(""), Vec::<&str>::new());
    }

    /// A subtree that already gives its children every controller needs no move and no write.
    #[test]
    fn a_subtree_with_its_controllers_on_is_taken_as_it_is() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("cgroup.controllers"), "cpu memory pids\n").unwrap();
        fs::write(root.path().join("cgroup.subtree_control"), "cpu memory pids\n").unwrap();
        let delegated = delegate_at(&root.path().join(DAEMON), 1).unwrap();
        assert_eq!(delegated.root, root.path());
        assert_eq!(delegated.controllers, [MEMORY, CPU, PIDS]);
        assert!(!root.path().join(DAEMON).exists(), "nothing was moved");
    }

    #[test]
    fn a_cgroup_without_memory_or_cpu_to_give_is_refused_with_what_it_has() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("cgroup.controllers"), "pids\n").unwrap();
        fs::write(root.path().join("cgroup.subtree_control"), "").unwrap();
        let refused = delegate_at(root.path(), 1).unwrap_err();
        assert!(refused.evidence.contains("neither the memory nor the cpu") && refused.evidence.contains("pids"));
        assert_eq!(refused.action, DELEGATE_ACTION);
    }

    #[test]
    fn a_daemon_that_shares_its_cgroup_moves_nobody_and_says_so() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("cgroup.controllers"), "cpu memory pids\n").unwrap();
        fs::write(root.path().join("cgroup.subtree_control"), "").unwrap();
        fs::write(root.path().join("cgroup.procs"), "1\n4242\n").unwrap();
        let refused = delegate_at(root.path(), 4242).unwrap_err();
        assert!(refused.evidence.contains("1 process(es) besides this daemon"), "{}", refused.evidence);
        assert_eq!(refused.action, ALONE_ACTION);
        assert!(!root.path().join(DAEMON).exists(), "no leaf was made and nobody moved");
        assert_eq!(fs::read_to_string(root.path().join("cgroup.subtree_control")).unwrap(), "");
    }

    /// In a file tree standing in for cgroupfs: the daemon alone moves itself into its leaf, then
    /// turns on what was missing.
    #[test]
    fn a_daemon_alone_in_its_cgroup_enters_its_leaf_then_turns_on_what_is_missing() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("cgroup.controllers"), "cpuset cpu memory pids\n").unwrap();
        fs::write(root.path().join("cgroup.subtree_control"), "memory\n").unwrap();
        fs::write(root.path().join("cgroup.procs"), "4242\n").unwrap();
        let delegated = delegate_at(root.path(), 4242).unwrap();
        assert_eq!(delegated.controllers, [MEMORY, CPU, PIDS]);
        assert_eq!(fs::read_to_string(root.path().join(DAEMON).join("cgroup.procs")).unwrap(), "4242");
        assert_eq!(fs::read_to_string(root.path().join("cgroup.subtree_control")).unwrap(), "+cpu +pids");
    }
}
