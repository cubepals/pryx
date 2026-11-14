// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Persistent data uses a host descriptor or an authenticated backend file service. Workload
//! guards and temporary memory reservations remain owned through stream and backend cleanup,
//! including cancellation. File access never starts workload commands or chooses another tier.

use pryx_agent::paths::DataDir;
use tokio::sync::OwnedMutexGuard;

use super::*;

pub(crate) enum DataLocation {
    Host(DataDir),
    Remote(crate::runtime::files::Service),
}
pub(crate) struct DataAccess {
    location: Option<DataLocation>,
    guard: Option<OwnedMutexGuard<()>>,
    reservation: Option<Reservation>,
    manager: Arc<Manager>,
    id: WorkloadId,
}
struct Reservation {
    state: Arc<Mutex<State>>,
    id: WorkloadId,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.state.lock().unwrap().copying.remove(&self.id);
    }
}
impl DataAccess {
    pub(crate) fn location(&self) -> &DataLocation {
        self.location.as_ref().expect("Data access remains open until cleanup")
    }
    pub(crate) async fn finish(mut self) -> Result<(), NodeError> {
        if let Some(DataLocation::Remote(service)) = self.location.take() {
            let (manager, id, guard, reservation) =
                (self.manager.clone(), self.id.clone(), self.guard.take(), self.reservation.take());
            tokio::spawn(retire(service, manager, id, guard, reservation))
                .await
                .map_err(|_| NodeError::Internal("File service cleanup task failed".into()))??;
        }
        Ok(())
    }
}
async fn retire(
    service: crate::runtime::files::Service,
    manager: Arc<Manager>,
    id: WorkloadId,
    guard: Option<OwnedMutexGuard<()>>,
    reservation: Option<Reservation>,
) -> Result<(), NodeError> {
    let _held = (guard, reservation);
    let result = service.close().await;
    if let Err(problem) = &result {
        manager.set_issue(
            &id,
            Issue::new(IssueCode::RuntimeUnavailable, format!("File service cleanup failed: {problem}")),
        );
    }
    result.map_err(Into::into)
}
impl Drop for DataAccess {
    fn drop(&mut self) {
        if let Some(DataLocation::Remote(service)) = self.location.take() {
            let (manager, id, guard, reservation) =
                (self.manager.clone(), self.id.clone(), self.guard.take(), self.reservation.take());
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(retire(service, manager, id, guard, reservation));
            }
        }
    }
}

impl Manager {
    pub(super) fn data_owner(&self, id: &WorkloadId) -> Result<(u32, u32), NodeError> {
        self.record_data_owner(&self.record(id)?)
    }

    pub(crate) async fn data_access(
        self: &Arc<Self>,
        id: &WorkloadId,
        epoch: Option<u64>,
    ) -> Result<DataAccess, NodeError> {
        let guard = self.lock_for(id).lock_owned().await;
        let record = self.record(id)?;
        check_epoch(&record, epoch, EpochRule::Exact)?;
        let runtime = self.record_runtime(&record)?;
        if runtime.uses_remote_data() {
            // Publish the reservation before admission, so competing file sessions count it.
            self.state.lock().unwrap().copying.insert(id.clone());
            let reservation = Reservation { state: self.state.clone(), id: id.clone() };
            self.admit(id, u64::from(record.spec.resources.memory_mb))?;
            let request = crate::runtime::files::Request {
                name: record.container_name,
                sandbox: id.to_string(),
                epoch: record.epoch.unwrap_or_default(),
                data_dir: self.store.data_dir(id),
                mount_path: record.spec.storage.mount_path,
                user: self.config.workloads.user.to_string(),
                memory_bytes: i64::from(record.spec.resources.memory_mb) * 1024 * 1024,
                storage_bytes: u64::from(record.spec.storage.size_gb) * 1024 * 1024 * 1024,
                pids_limit: i64::from(
                    record.spec.resources.pids_limit.unwrap_or(self.config.workloads.default_pids_limit),
                ),
                tmp_size_mb: self.config.workloads.tmp_size_mb,
                log_max_size_mb: self.config.workloads.log_max_size_mb,
            };
            let (manager, id) = (self.clone(), id.clone());
            return tokio::spawn(async move {
                let service = runtime.data_service(request).await?;
                Ok(DataAccess {
                    location: Some(DataLocation::Remote(service)),
                    guard: Some(guard),
                    reservation: Some(reservation),
                    manager,
                    id,
                })
            })
            .await
            .map_err(|_| NodeError::Internal("File service preparation task failed".into()))?;
        }
        let directory = DataDir::open_owned(&runtime.data_path(&self.store.data_dir(id)), self.data_owner(id)?)
            .map_err(file_error)?;
        Ok(DataAccess {
            location: Some(DataLocation::Host(directory)),
            guard: Some(guard),
            reservation: None,
            manager: self.clone(),
            id: id.clone(),
        })
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
