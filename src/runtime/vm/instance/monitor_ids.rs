// SPDX-License-Identifier: FSL-1.1-ALv2
//! Root's monitor IDs, one pair for each VM. Firecracker's production host setup
//! (docs/prod-host-setup.md at 1.18.0) recommends that each microVM run with its own uid and gid,
//! so a monitor broken out of its jail owns no other VM's files, TAP or processes. A VM takes the
//! lowest pair of the host's range that no other VM holds and no process runs as, through the
//! process tier's pool search, keeps it across boots, and lets it go when it is removed. The pairs
//! are a private ledger in the runtime's state, read again at start. Not for launching monitors
//! or recording jails (`monitor.rs`, `jail.rs`), which take the pair this hands out.
use super::*;
use std::io::Read;

/// The host's monitor IDs (`[vm]` monitor_uid, monitor_gid, monitor_ids): a VM's pair is the same
/// offset into both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitorIds {
    pub uid: u32,
    pub gid: u32,
    pub count: u32,
}

impl MonitorIds {
    /// Non-root, at least one, and within u32, so u32::MAX ("no ID" to chown(2)) is never one.
    pub(super) fn valid(&self) -> bool {
        self.uid != rustix::process::Uid::ROOT.as_raw()
            && self.gid != rustix::process::Gid::ROOT.as_raw()
            && self.count != 0
            && self.uid.checked_add(self.count).is_some()
            && self.gid.checked_add(self.count).is_some()
    }
    fn holds(&self, (uid, gid): (u32, u32)) -> bool {
        uid.checked_sub(self.uid).is_some_and(|offset| offset < self.count && gid.checked_sub(self.gid) == Some(offset))
    }
}

const LEDGER: &str = "monitor-ids.json";
const MAX_LEDGER_BYTES: u64 = 1024 * 1024;

pub(super) struct Ledger {
    path: PathBuf,
    ids: MonitorIds,
    held: BTreeMap<String, (u32, u32)>,
}

impl Ledger {
    /// The ledger under `root`, keeping a pair only for a VM that `known` names, inside today's
    /// range, held once. Any other VM takes a pair again at its next launch.
    pub(super) fn load(root: &Path, ids: MonitorIds, known: impl Fn(&str) -> bool) -> Result<Self, RuntimeError> {
        let path = root.join(LEDGER);
        let saved: BTreeMap<String, (u32, u32)> =
            match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&path) {
                Ok(file) => {
                    let info = file.metadata().map_err(error)?;
                    if !info.is_file()
                        || info.uid() != rustix::process::geteuid().as_raw()
                        || info.mode() & 0o077 != 0
                        || info.nlink() != 1
                    {
                        return Err(error("Unsafe VM monitor ID ledger"));
                    }
                    let mut bytes = Vec::new();
                    file.take(MAX_LEDGER_BYTES + 1).read_to_end(&mut bytes).map_err(error)?;
                    if bytes.len() as u64 > MAX_LEDGER_BYTES {
                        return Err(error("VM monitor ID ledger exceeds its bound"));
                    }
                    serde_json::from_slice(&bytes).map_err(|_| error("Invalid VM monitor ID ledger"))?
                }
                Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
                Err(problem) => return Err(error(problem)),
            };
        let mut held = BTreeMap::new();
        for (vm, owner) in saved {
            if known(&vm) && ids.holds(owner) && !held.values().any(|taken| *taken == owner) {
                held.insert(vm, owner);
            }
        }
        Ok(Self { path, ids, held })
    }

    /// The pair `vm` holds, or the lowest free one, written down before anything runs as it.
    pub(super) fn owner(&mut self, vm: &str) -> Result<(u32, u32), RuntimeError> {
        if let Some(owner) = self.held.get(vm) {
            return Ok(*owner);
        }
        let uid = crate::runtime::process::free_id(self.ids.uid..self.ids.uid + self.ids.count, |uid| {
            self.held.values().any(|(taken, _)| *taken == uid)
        })
        .ok_or_else(|| RuntimeError::Unavailable("Every root VM monitor ID is held; raise vm.monitor_ids".into()))?;
        let owner = (uid, self.ids.gid + (uid - self.ids.uid));
        let mut next = self.held.clone();
        next.insert(vm.into(), owner);
        self.save(&next)?;
        self.held = next;
        Ok(owner)
    }

    /// Lets `vm`'s pair go, once the VM is removed.
    pub(super) fn release(&mut self, vm: &str) -> Result<(), RuntimeError> {
        if !self.held.contains_key(vm) {
            return Ok(());
        }
        let mut next = self.held.clone();
        next.remove(vm);
        self.save(&next)?;
        self.held = next;
        Ok(())
    }

    fn save(&self, held: &BTreeMap<String, (u32, u32)>) -> Result<(), RuntimeError> {
        crate::durable::write_atomic(&self.path, &serde_json::to_vec(held).map_err(error)?, 0o600).map_err(error)
    }
}

impl VmRuntime {
    /// This VM's monitor IDs as root, `None` for an ordinary user's runtime, whose monitors run
    /// as itself.
    pub(super) fn monitor_owner(&self, vm: &str) -> Result<Option<(u32, u32)>, RuntimeError> {
        self.state.monitor_ids.as_ref().map(|ledger| ledger.lock().unwrap().owner(vm)).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Above the process tier's default pool, where nothing on a test host runs.
    const IDS: MonitorIds = MonitorIds { uid: 61000, gid: 61100, count: 3 };

    #[test]
    fn each_vm_holds_its_own_pair_across_restarts_until_removed() {
        let root = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::load(root.path(), IDS, |_| true).unwrap();
        let first = ledger.owner("vm-first").unwrap();
        let second = ledger.owner("vm-second").unwrap();
        assert_eq!((first, second), ((61000, 61100), (61001, 61101)));
        assert_eq!(ledger.owner("vm-first").unwrap(), first);
        // Recovery: the same pairs, read back from the ledger alone.
        let mut ledger = Ledger::load(root.path(), IDS, |_| true).unwrap();
        assert_eq!((ledger.owner("vm-first").unwrap(), ledger.owner("vm-second").unwrap()), (first, second));
        // Removal lets a pair go, and the next VM takes it again.
        ledger.release("vm-first").unwrap();
        let mut ledger = Ledger::load(root.path(), IDS, |_| true).unwrap();
        assert_eq!(ledger.owner("vm-third").unwrap(), first);
        assert_eq!(ledger.owner("vm-fourth").unwrap(), (61002, 61102));
        assert!(matches!(ledger.owner("vm-fifth"), Err(RuntimeError::Unavailable(_))));
        assert_eq!(fs::metadata(root.path().join(LEDGER)).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn recovery_drops_pairs_of_unknown_vms_foreign_ids_and_repeats() {
        let root = tempfile::tempdir().unwrap();
        let saved = BTreeMap::from([
            ("vm-gone".to_owned(), (61000, 61100)),
            ("vm-kept".to_owned(), (61001, 61101)),
            ("vm-moved".to_owned(), (60001, 60001)),
            ("vm-skewed".to_owned(), (61002, 61100)),
            ("vm-twin".to_owned(), (61001, 61101)),
        ]);
        crate::durable::write_atomic(&root.path().join(LEDGER), &serde_json::to_vec(&saved).unwrap(), 0o600).unwrap();
        let mut ledger = Ledger::load(root.path(), IDS, |vm| vm != "vm-gone").unwrap();
        assert_eq!(ledger.held, BTreeMap::from([("vm-kept".to_owned(), (61001, 61101))]));
        // A VM whose pair was dropped takes a free one: never its twin's.
        assert_eq!(ledger.owner("vm-twin").unwrap(), (61000, 61100));
        assert_eq!(ledger.owner("vm-moved").unwrap(), (61002, 61102));
    }

    #[test]
    fn root_and_overflowing_ranges_are_not_monitor_ids() {
        assert!(IDS.valid());
        for ids in [
            MonitorIds { uid: 0, ..IDS },
            MonitorIds { gid: 0, ..IDS },
            MonitorIds { count: 0, ..IDS },
            MonitorIds { uid: u32::MAX - 1, ..IDS },
        ] {
            assert!(!ids.valid(), "{ids:?}");
        }
    }
}
