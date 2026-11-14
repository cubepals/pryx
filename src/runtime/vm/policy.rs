// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Read the bounded immutable VM descriptor installed with a signed host release. Archive
//! validation/acquisition belongs to the artifact owner; this never downloads or starts compute,
//! infers pins from a URL, or accepts a descriptor from a workload request.
use super::{bundle::Artifact, error};
use crate::{config::VmConfig, runtime::RuntimeError};
use std::{
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
};

// A host-policy descriptor is small metadata; this is its own read bound, not an archive limit.
const MAX_DESCRIPTOR_BYTES: u64 = 16 * 1024;

pub fn descriptor(config: &VmConfig) -> Result<Artifact, RuntimeError> {
    let path = config.bundle.as_ref().ok_or_else(|| {
        RuntimeError::Unavailable("The signed host installation has no pinned VM bundle descriptor".into())
    })?;
    super::bundle::ancestors(path.parent().ok_or_else(|| error("Missing VM descriptor parent"))?).map_err(error)?;
    let file =
        OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).map_err(error)?;
    let info = file.metadata().map_err(error)?;
    if !info.is_file()
        || (info.uid() != rustix::process::geteuid().as_raw() && info.uid() != rustix::process::Uid::ROOT.as_raw())
        || info.mode() & 0o222 != 0
        || info.nlink() != 1
        || info.len() > MAX_DESCRIPTOR_BYTES
    {
        return Err(error("VM descriptor must be one bounded immutable host-owned regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_DESCRIPTOR_BYTES + 1).read_to_end(&mut bytes).map_err(error)?;
    if bytes.len() as u64 > MAX_DESCRIPTOR_BYTES {
        return Err(error("VM descriptor exceeds its read bound"));
    }
    serde_json::from_slice(&bytes).map_err(|_| error("Invalid host-installed VM descriptor"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };

    #[test]
    fn actual_descriptor_requires_immutable_files_and_protected_ancestors() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("descriptor.json");
        let bytes = uuid::Uuid::new_v4();
        let digest = hex::encode(ring::digest::digest(&ring::digest::SHA256, bytes.as_bytes()));
        let artifact = Artifact {
            url: "https://artifacts.pryx.invalid/runtime.tar.gz".into(),
            sha256: digest.clone(),
            manifest_sha256: digest,
            bytes: bytes.as_bytes().len() as u64,
            unpacked_bytes: bytes.as_bytes().len() as u64,
            entries: 1,
        };
        crate::durable::write_atomic(&path, &serde_json::to_vec(&artifact).unwrap(), 0o444).unwrap();
        let config = VmConfig { bundle: Some(path.clone()) };
        assert_eq!(descriptor(&config).unwrap().sha256, artifact.sha256);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(descriptor(&config).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o777)).unwrap();
        assert!(descriptor(&config).is_err());
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let retained = root.path().join("retained");
        fs::hard_link(&path, &retained).unwrap();
        assert!(descriptor(&config).is_err());
        fs::remove_file(&path).unwrap();
        symlink(&retained, &path).unwrap();
        assert!(descriptor(&config).is_err());
    }

    #[test]
    fn absent_oversized_special_and_invalid_descriptors_do_not_enable_compute() {
        assert!(matches!(descriptor(&VmConfig::default()), Err(RuntimeError::Unavailable(_))));
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("descriptor.json");
        let config = VmConfig { bundle: Some(path.clone()) };
        crate::durable::write_atomic(&path, &vec![b'x'; MAX_DESCRIPTOR_BYTES as usize + 1], 0o444).unwrap();
        assert!(descriptor(&config).is_err());
        crate::durable::write_atomic(&path, uuid::Uuid::new_v4().simple().to_string().as_bytes(), 0o444).unwrap();
        let problem = descriptor(&config).err().unwrap().to_string();
        assert_eq!(problem, "Invalid host-installed VM descriptor");
        fs::remove_file(&path).unwrap();
        rustix::fs::mkfifoat(rustix::fs::CWD, &path, rustix::fs::Mode::RUSR).unwrap();
        assert!(descriptor(&config).is_err(), "A nonblocking FIFO must be refused before reading");
    }
}
