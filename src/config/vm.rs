// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-installed VM descriptor location. Workloads cannot provide an archive, executable,
//! download URL or monitor identity; an absent installation leaves this tier unavailable.
use serde::Deserialize;
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VmConfig {
    pub bundle: Option<PathBuf>,
    /// Root's jailed monitors run as non-root IDs from these on, `monitor_ids` of them, each VM
    /// its own pair (Firecracker's docs/prod-host-setup.md asks a unique uid and gid for each
    /// microVM): outside the process tier's default UID pool, the Wasm worker's IDs and systemd's
    /// dynamic users (61184 on). Ordinary users' monitors run as themselves.
    pub monitor_uid: u32,
    pub monitor_gid: u32,
    pub monitor_ids: u32,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self { bundle: None, monitor_uid: 60001, monitor_gid: 60001, monitor_ids: 1024 }
    }
}

impl VmConfig {
    /// What is wrong with root's monitor IDs, if anything: they must be at least one non-root
    /// pair within u32 (an exclusive end that fits also keeps u32::MAX, "no ID" to chown(2), out),
    /// apart from the Wasm worker's `wasm` IDs and the process tier's `process` pool, where those
    /// tiers run beside them.
    pub(crate) fn monitor_ids_problem(
        &self,
        wasm: Option<(u32, u32)>,
        process: Option<std::ops::Range<u32>>,
    ) -> Option<&'static str> {
        let (Some(uid_end), Some(gid_end)) =
            (self.monitor_uid.checked_add(self.monitor_ids), self.monitor_gid.checked_add(self.monitor_ids))
        else {
            return Some("the range overflows");
        };
        let (uids, gids) = (self.monitor_uid..uid_end, self.monitor_gid..gid_end);
        if uids.is_empty() || uids.start == 0 || gids.start == 0 {
            return Some("at least one non-root ID");
        }
        if wasm.is_some_and(|(uid, gid)| uids.contains(&uid) || gids.contains(&gid))
            || process.is_some_and(|pool| uids.start < pool.end && pool.start < uids.end)
        {
            return Some("they overlap the Wasm worker's IDs or the process tier's UID pool");
        }
        None
    }

    pub(super) fn infer(&mut self, agent: &Path) -> Option<crate::infer::Inferred> {
        if self.bundle.is_some() {
            return None;
        }
        let path = agent.parent()?.join("vm-bundle.json");
        if matches!(path.symlink_metadata(), Err(problem) if problem.kind() == io::ErrorKind::NotFound) {
            return None;
        }
        // Discovery is not trust: retain unsafe or inaccessible paths so the policy reader
        // can report their refusal instead of silently treating the installation as absent.
        self.bundle = Some(path.clone());
        Some(crate::infer::Inferred {
            key: "vm.bundle",
            value: path.display().to_string(),
            from: "the VM descriptor beside the configured host agent".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn root_vm_monitor_ids_are_a_non_root_range_apart_from_other_pools() {
        let (wasm, process) = (Some((60000, 60000)), Some(20000..36384));
        assert_eq!(VmConfig::default().monitor_ids_problem(wasm, process.clone()), None);
        let with = |uid, gid, ids| VmConfig { monitor_uid: uid, monitor_gid: gid, monitor_ids: ids, bundle: None };
        assert_eq!(with(70000, 70000, 64).monitor_ids_problem(wasm, process.clone()), None);
        assert_eq!(with(30000, 61000, 8).monitor_ids_problem(wasm, None), None, "No process tier beside them");
        for (config, wasm, process) in [
            (with(60001, 60001, 0), None, None),
            (with(0, 60001, 8), None, None),
            (with(60001, 0, 8), None, None),
            (with(u32::MAX, 60001, 2), None, None),
            (with(59990, 70000, 1024), wasm, None),
            (with(70000, 59990, 1024), wasm, None),
            (with(30000, 61000, 8), None, process.clone()),
        ] {
            assert!(config.monitor_ids_problem(wasm, process).is_some(), "{config:?}");
        }
    }

    #[test]
    fn installation_discovery_keeps_explicit_host_policy_and_unsafe_candidates() {
        let root = tempfile::tempdir().unwrap();
        let agent = root.path().join("pryx-agent");
        let path = root.path().join("vm-bundle.json");
        let mut config = VmConfig::default();
        assert!(config.infer(&agent).is_none());
        assert!(config.bundle.is_none());
        fs::write(&path, uuid::Uuid::new_v4().as_bytes()).unwrap();
        let inferred = config.infer(&agent).unwrap();
        assert_eq!(config.bundle.as_deref(), Some(path.as_path()));
        assert_eq!(inferred.key, "vm.bundle");
        let explicit = root.path().join("host-selected.json");
        config.bundle = Some(explicit.clone());
        assert!(config.infer(&agent).is_none());
        assert_eq!(config.bundle, Some(explicit));
        fs::remove_file(&path).unwrap();
        symlink(root.path().join("missing"), &path).unwrap();
        let mut config = VmConfig::default();
        assert!(config.infer(&agent).is_some());
        assert_eq!(config.bundle, Some(path));
    }
}
