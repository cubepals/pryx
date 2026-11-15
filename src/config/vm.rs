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
    /// Root's jailed monitors run as these fixed non-root IDs, outside the process tier's
    /// default UID pool and the Wasm worker's IDs. Ordinary users' monitors run as themselves.
    pub monitor_uid: u32,
    pub monitor_gid: u32,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self { bundle: None, monitor_uid: 60001, monitor_gid: 60001 }
    }
}

impl VmConfig {
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
