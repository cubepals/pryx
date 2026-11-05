// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Sandbox keys remain stable within one epoch, across replacement of compute. These private
//! host files are outside records, exported data, snapshots and every sandbox bind mount.

use crate::{durable, ids::WorkloadId};
use pryx_agent::auth::SigningKey;
use std::{
    fs::{self, OpenOptions},
    io::{self, Read},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};
const DIRECTORY: &str = "agent-keys";
const MAX_KEY_BYTES: u64 = 4096;

pub(super) fn load(root: &Path, id: &WorkloadId, epoch: u64) -> io::Result<String> {
    let base = root.join(DIRECTORY);
    let dir = base.join(id.as_str());
    fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    for path in [&base, &dir] {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Unsafe agent key directory"));
        }
    }
    let path = dir.join(format!("{epoch}.key"));
    match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&path) {
        Ok(file) => {
            let metadata = file.metadata()?;
            if !metadata.is_file()
                || metadata.uid() != rustix::process::geteuid().as_raw()
                || metadata.mode() & 0o077 != 0
            {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Unsafe agent key file"));
            }
            let mut encoded = String::new();
            file.take(MAX_KEY_BYTES + 1).read_to_string(&mut encoded)?;
            if encoded.len() as u64 > MAX_KEY_BYTES || SigningKey::from_encoded(&encoded).is_err() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid agent key file"));
            }
            Ok(encoded)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let (_, encoded) =
                SigningKey::generate().map_err(|_| io::Error::other("Cannot generate sandbox signing material"))?;
            durable::write_atomic(&path, encoded.as_bytes(), 0o600)
                .map_err(|_| io::Error::other("Cannot persist sandbox signing material"))?;
            Ok(encoded)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn discard(root: &Path, id: &WorkloadId) -> io::Result<()> {
    let directory = root.join(DIRECTORY).join(id.as_str());
    match fs::remove_dir_all(&directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    #[test]
    fn keys_survive_compute_replacement_but_not_epoch_change_or_data_delete() {
        let dir = tempfile::tempdir().unwrap();
        let id = WorkloadId::parse("private").unwrap();
        let public = |key: &str| SigningKey::from_encoded(key).unwrap().public_key();
        let first = public(&load(dir.path(), &id, 1).unwrap());
        assert_eq!(first, public(&load(dir.path(), &id, 1).unwrap()));
        assert_ne!(first, public(&load(dir.path(), &id, 2).unwrap()));
        let mode =
            File::open(dir.path().join(DIRECTORY).join(id.as_str()).join("1.key")).unwrap().metadata().unwrap().mode();
        assert_eq!(mode & 0o777, 0o600);
        discard(dir.path(), &id).unwrap();
        assert_ne!(first, public(&load(dir.path(), &id, 1).unwrap()));
    }
}
