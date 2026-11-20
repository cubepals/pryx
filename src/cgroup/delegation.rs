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

/// What `delegate` can turn on for this caller's children, given the current controllers and
/// membership. For `pryxd probe`, which leaves every process and controller where it found it.
pub(crate) fn inspect() -> Result<Delegated, Unavailable> {
    inspect_at(&own()?)
}

fn inspect_at(own: &Path) -> Result<Delegated, Unavailable> {
    let (delegated, missing) = delegation_plan(own)?;
    if !missing.is_empty() {
        let ignore = (own == delegated.root).then(std::process::id);
        check_empty(&delegated.root, ignore)?;
    }
    Ok(delegated)
}

/// The controllers a cgroup's interface file lists, of those a sandbox's limits need.
fn listed(text: &str) -> Vec<&'static str> {
    WANTED.into_iter().filter(|wanted| text.split_whitespace().any(|name| name == *wanted)).collect()
}

fn delegation_plan(own: &Path) -> Result<(Delegated, Vec<&'static str>), Unavailable> {
    let root = subtree_root(own);
    check_delegated(&root)?;
    let interface = |name: &str| {
        let path = root.join(name);
        fs::read_to_string(&path).map_err(|error| unavailable(format!("{}: {error}", path.display()), DELEGATE_ACTION))
    };
    let controllers = interface("cgroup.controllers")?;
    let available = listed(&controllers);
    if !available.contains(&MEMORY) && !available.contains(&CPU) {
        return Err(unavailable(
            format!(
                "cgroup {} has neither the memory nor the cpu controller to give (cgroup.controllers: {})",
                root.display(),
                controllers.trim()
            ),
            DELEGATE_ACTION,
        ));
    }
    let enabled = listed(&interface("cgroup.subtree_control")?);
    let missing: Vec<&str> = available.iter().copied().filter(|name| !enabled.contains(name)).collect();
    Ok((Delegated { root, controllers: available }, missing))
}

/// `delegate` for a daemon with process ID `pid` in cgroup `own`.
fn delegate_at(own: &Path, pid: u32) -> Result<Delegated, Unavailable> {
    let (delegated, missing) = delegation_plan(own)?;
    let root = &delegated.root;
    if !missing.is_empty() {
        if own == root {
            enter_leaf(root, pid)?;
        } else {
            check_empty(root, None)?;
        }
        let change: Vec<String> = missing.iter().map(|name| format!("+{name}")).collect();
        fs::write(root.join("cgroup.subtree_control"), change.join(" ")).map_err(|error| {
            let busy = error.raw_os_error() == Some(libc::EBUSY);
            unavailable(
                format!("turning on {} for {}'s children: {error}", missing.join(" "), root.display()),
                if busy { ALONE_ACTION } else { DELEGATE_ACTION },
            )
        })?;
    }
    Ok(delegated)
}

fn member_count(root: &Path, ignore: Option<u32>) -> Result<usize, Unavailable> {
    members(root)
        .map(|members| members.into_iter().filter(|member| Some(*member) != ignore).count())
        .map_err(|error| unavailable(format!("{}: {error}", root.join("cgroup.procs").display()), DELEGATE_ACTION))
}

fn occupied(root: &Path, count: usize, ignore: Option<u32>) -> Unavailable {
    let where_held = if ignore.is_some() { "besides this daemon" } else { "in the parent of this daemon" };
    unavailable(format!("cgroup {} holds {count} process(es) {where_held}", root.display()), ALONE_ACTION)
}

fn check_empty(root: &Path, ignore: Option<u32>) -> Result<(), Unavailable> {
    let count = member_count(root, ignore)?;
    if count == 0 { Ok(()) } else { Err(occupied(root, count, ignore)) }
}

/// Moves this daemon into its leaf, only when it is the cgroup's one process: a child it started
/// a moment ago (a probe) is waited for; any other process is never moved.
fn enter_leaf(root: &Path, pid: u32) -> Result<(), Unavailable> {
    const WAIT: std::time::Duration = std::time::Duration::from_secs(2);
    const POLL: std::time::Duration = std::time::Duration::from_millis(20);
    let deadline = std::time::Instant::now() + WAIT;
    let ignore = Some(pid);
    loop {
        let count = member_count(root, ignore)?;
        if count == 0 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(occupied(root, count, ignore));
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
        let own = root.path().join(DAEMON);
        for delegated in [inspect_at(&own).unwrap(), delegate_at(&own, 1).unwrap()] {
            assert_eq!(delegated.root, root.path());
            assert_eq!(delegated.controllers, [MEMORY, CPU, PIDS]);
        }
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
        let controllers = "cpu memory pids\n";
        let pid = std::process::id();
        let members = format!("{pid}\n{}\n", pid + 1);
        fs::write(root.path().join("cgroup.controllers"), controllers).unwrap();
        fs::write(root.path().join("cgroup.subtree_control"), "").unwrap();
        fs::write(root.path().join("cgroup.procs"), &members).unwrap();

        let inspected = inspect_at(root.path());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3, "inspection added a cgroup entry");
        assert_eq!(fs::read_to_string(root.path().join("cgroup.controllers")).unwrap(), controllers);
        assert_eq!(fs::read_to_string(root.path().join("cgroup.subtree_control")).unwrap(), "");
        assert_eq!(fs::read_to_string(root.path().join("cgroup.procs")).unwrap(), members);

        let refused = delegate_at(root.path(), pid).unwrap_err();
        assert!(refused.evidence.contains("1 process(es) besides this daemon"), "{}", refused.evidence);
        assert_eq!(refused.action, ALONE_ACTION);
        assert!(!root.path().join(DAEMON).exists(), "no leaf was made and nobody moved");
        assert_eq!(fs::read_to_string(root.path().join("cgroup.subtree_control")).unwrap(), "");

        let inspected = inspected.expect_err("inspection reported unavailable limits as usable");
        assert_eq!(inspected.evidence, refused.evidence);
        assert_eq!(inspected.action, refused.action);
    }

    /// In a file tree standing in for cgroupfs: the daemon alone moves itself into its leaf, then
    /// turns on what was missing.
    #[test]
    fn a_daemon_alone_in_its_cgroup_enters_its_leaf_then_turns_on_what_is_missing() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("cgroup.controllers"), "cpuset cpu memory pids\n").unwrap();
        fs::write(root.path().join("cgroup.subtree_control"), "memory\n").unwrap();
        let pid = std::process::id();
        let member = format!("{pid}\n");
        fs::write(root.path().join("cgroup.procs"), &member).unwrap();
        let inspected = inspect_at(root.path()).unwrap();
        assert_eq!(inspected.controllers, [MEMORY, CPU, PIDS]);
        assert!(!root.path().join(DAEMON).exists(), "inspection made a leaf");
        assert_eq!(fs::read_to_string(root.path().join("cgroup.subtree_control")).unwrap(), "memory\n");
        assert_eq!(fs::read_to_string(root.path().join("cgroup.procs")).unwrap(), member);
        let delegated = delegate_at(root.path(), pid).unwrap();
        assert_eq!(delegated.controllers, [MEMORY, CPU, PIDS]);
        assert_eq!(fs::read_to_string(root.path().join(DAEMON).join("cgroup.procs")).unwrap(), pid.to_string());
        assert_eq!(fs::read_to_string(root.path().join("cgroup.subtree_control")).unwrap(), "+cpu +pids");
    }

    #[test]
    fn a_daemon_in_its_leaf_needs_an_empty_parent_to_turn_on_missing_controllers() {
        let root = tempfile::tempdir().unwrap();
        let own = root.path().join(DAEMON);
        let procs = root.path().join("cgroup.procs");
        let subtree = root.path().join("cgroup.subtree_control");
        fs::create_dir(&own).unwrap();
        fs::write(root.path().join("cgroup.controllers"), "cpu memory pids\n").unwrap();
        fs::write(&subtree, "memory\n").unwrap();
        fs::write(&procs, "8\n").unwrap();

        let inspected = inspect_at(&own).unwrap_err();
        let refused = delegate_at(&own, 9).unwrap_err();
        assert!(inspected.evidence.contains("1 process(es) in the parent of this daemon"));
        assert_eq!(inspected.evidence, refused.evidence);
        assert_eq!(inspected.action, ALONE_ACTION);
        assert_eq!(fs::read_to_string(&procs).unwrap(), "8\n");
        assert_eq!(fs::read_to_string(&subtree).unwrap(), "memory\n");

        fs::write(&procs, "").unwrap();
        assert_eq!(inspect_at(&own).unwrap().controllers, [MEMORY, CPU, PIDS]);
        assert_eq!(fs::read_to_string(&subtree).unwrap(), "memory\n");
        assert_eq!(delegate_at(&own, 9).unwrap().controllers, [MEMORY, CPU, PIDS]);
        assert_eq!(fs::read_to_string(&subtree).unwrap(), "+cpu +pids");
        assert!(!own.join("cgroup.procs").exists(), "the daemon stayed in its leaf");
    }
}
