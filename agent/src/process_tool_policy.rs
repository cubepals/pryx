// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-bound native artifact/cache inputs and descriptor-relative scratch preparation. No
//! supervisor or acquisition implementation is needed to define or safely open these paths.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SSH_AUTH_DIRECTORY: &str = "/tmp/pryx-ssh/auth";
pub const SSH_BOOTSTRAP_ENV: &str = "PRYX_SSH_BOOTSTRAP";

pub const VERSION: u16 = 1;
pub const BROWSER_VERSION: u16 = 1;
pub const BROWSER_CAPABILITY: &str = "process-browser-sandbox";
pub const DESKTOP_VERSION: u16 = 1;
pub const DESKTOP_CAPABILITY: &str = "process-desktop";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub url: String,
    pub sha256: String,
    pub manifest_sha256: String,
    pub bytes: u64,
    pub unpacked_bytes: u64,
    pub entries: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub cache: PathBuf,
    pub downloads: PathBuf,
    pub scratch: PathBuf,
    pub artifact: Option<Artifact>,
    pub unavailable: Option<String>,
    pub browser_on: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_on: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_unavailable: Option<String>,
}

impl Policy {
    fn scratch_path<'a>(&self, path: &'a std::path::Path) -> std::io::Result<&'a str> {
        path.strip_prefix(&self.scratch)
            .ok()
            .and_then(|path| path.to_str())
            .ok_or_else(|| std::io::Error::other("Native scratch path escapes its host-owned root"))
    }
    pub(crate) fn scratch_directory(&self, path: &std::path::Path) -> std::io::Result<std::fs::File> {
        crate::paths::DataDir::open(&self.scratch)?.directory_handle(self.scratch_path(path)?, true)
    }
    pub(crate) fn scratch_output(&self, path: &std::path::Path) -> std::io::Result<std::fs::File> {
        crate::paths::DataDir::open(&self.scratch)?.write(self.scratch_path(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_walk_refuses_parent_symlinks_before_creating_directories_or_outputs() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let scratch = root.path().join("scratch");
        let foreign = root.path().join("foreign");
        std::fs::create_dir(&scratch).unwrap();
        std::fs::create_dir(&foreign).unwrap();
        std::os::unix::fs::symlink(&foreign, scratch.join("session")).unwrap();
        let policy = Policy {
            cache: root.path().into(),
            downloads: root.path().into(),
            scratch: scratch.clone(),
            artifact: None,
            unavailable: None,
            browser_on: true,
            desktop_on: None,
            desktop_unavailable: None,
        };
        assert!(policy.scratch_directory(&scratch.join("session/home")).is_err());
        assert!(policy.scratch_output(&scratch.join("session/log")).is_err());
        assert_eq!(std::fs::read_dir(&foreign).unwrap().count(), 0);
        assert!(policy.scratch_output(&scratch.join("../foreign/log")).is_err());
        policy.scratch_directory(&scratch.join("own/home")).unwrap();
        policy.scratch_output(&scratch.join("own/log")).unwrap().write_all(b"own log").unwrap();
        assert_eq!(std::fs::read(scratch.join("own/log")).unwrap(), b"own log");
    }

    #[test]
    fn legacy_policy_round_trip_omits_unannounced_desktop_fields() {
        let legacy = serde_json::json!({"cache":"/cache","downloads":"/downloads","scratch":"/scratch","artifact":null,"unavailable":null,"browser_on":false});
        let decoded: Policy = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(decoded.desktop_on, None);
        assert_eq!(decoded.desktop_unavailable, None);
        assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    }
}
