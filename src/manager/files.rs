// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Descriptor-relative data access does not require running compute. The workload lock remains
//! held for a stream's lifetime, preventing delete or restore from replacing its directory.

use pryx_agent::paths::DataDir;
use tokio::sync::OwnedMutexGuard;

use super::*;

pub(crate) struct DataAccess {
    pub(crate) directory: DataDir,
    pub(crate) guard: OwnedMutexGuard<()>,
}

impl Manager {
    pub(crate) async fn data_access(&self, id: &WorkloadId, epoch: Option<u64>) -> Result<DataAccess, NodeError> {
        let guard = self.lock_for(id).lock_owned().await;
        let record = self.record(id)?;
        check_epoch(&record, epoch, EpochRule::Exact)?;
        let directory =
            DataDir::open_owned(&self.store.data_dir(id), self.config.data_owner_ids()).map_err(file_error)?;
        Ok(DataAccess { directory, guard })
    }

    pub(crate) fn file_room(&self, bytes: u64) -> Result<(), NodeError> {
        self.admit_disk(bytes, "writing sandbox data")
    }
}

pub(crate) fn file_error(error: std::io::Error) -> NodeError {
    if error.kind() == std::io::ErrorKind::NotFound {
        NodeError::FileNotFound
    } else if matches!(
        error.kind(),
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotADirectory
    ) || error.raw_os_error() == Some(libc::ELOOP)
    {
        NodeError::Invalid(vec![crate::protocol::FieldError {
            field: "path".into(),
            problem: "file operation was refused".into(),
        }])
    } else {
        NodeError::Io { what: "sandbox file".into(), source: error }
    }
}
