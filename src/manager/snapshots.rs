// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Local snapshots of a workload's data: taken, listed, deleted, uploaded, and a fork from one that
//! takes its source's memory too. Packing and sending an upload is `pack_and_put` in `archive.rs`;
//! restoring from a snapshot is `restore` there.

use super::*;

impl Manager {
    // ─── snapshots ─────────────────────────────────────────────────────────────────────────

    /// A copy of the workload's data beside it, on this node: what a backup is first, before it
    /// is uploaded, and what a restore on this node reads. Taken from the current copy only, like
    /// an export, and from a running workload only while its saving is paused.
    pub async fn snapshot(
        self: &Arc<Self>,
        id: WorkloadId,
        request: SnapshotRequest,
        epoch: Option<u64>,
    ) -> Result<SnapshotResponse, NodeError> {
        let own = self.clone();
        tokio::spawn(async move {
            let lock = own.lock_for(&id);
            let _guard = lock.lock().await;
            let started = Instant::now();
            let result = own.snapshot_locked(&id, request, epoch).await;
            own.metrics.operation("snapshot", &result, started.elapsed());
            result
        })
        .await
        .map_err(|_| NodeError::Internal("Snapshot task did not complete".into()))?
    }

    async fn snapshot_locked(
        &self,
        id: &WorkloadId,
        request: SnapshotRequest,
        epoch: Option<u64>,
    ) -> Result<SnapshotResponse, NodeError> {
        let started = Instant::now();
        let record = self.record(id)?;
        check_epoch(&record, epoch, EpochRule::Exact)?;
        if let Some(existing) = self.store.snapshot(id, &request.id) {
            return Ok(SnapshotResponse { created: false, snapshot: existing });
        }
        if record.phase == Phase::Creating {
            return Err(NodeError::Conflict {
                code: ConflictCode::NotCreated,
                message: "the workload's creation didn't finish; PUT its spec again".into(),
            });
        }
        let running = self.observe(&record).await?.is_some_and(|i| {
            matches!(i.status, ContainerStatus::Running | ContainerStatus::Restarting | ContainerStatus::Paused)
        });
        if running && !request.quiesced {
            return Err(NodeError::Conflict {
                code: ConflictCode::NotQuiesced,
                message: "stop the workload, or pause its saving and say so (quiesced), before a snapshot".into(),
            });
        }
        // Asleep with its memory outside it (a VM's): that memory, which a fork from this snapshot
        // on this node can take while the workload still sleeps with it.
        let memory = if running && self.suspension(id).suspended() {
            self.note(self.record_runtime(&record)?.memory_image(&record.container_name).await)?
        } else {
            None
        };
        let data = self.store.data_dir(id);
        // The data's full size, even where the copy will share blocks with it: a shared copy is
        // free only until the data changes, and then takes as much as a plain one. Counted as
        // free, copies could pile up on a node near its floor and need that space later.
        let need = {
            let data = data.clone();
            tokio::task::spawn_blocking(move || crate::tarball::tree_bytes(&data)).await.unwrap_or(0)
        };
        self.admit_disk(need, "a snapshot")?;
        let into = {
            let (store, id, snapshot) = (self.store.clone(), id.clone(), request.id.clone());
            blocking(move || store.prepare_snapshot(&id, &snapshot)).await??
        };
        let copied = self
            .copy_data(&record, running, async {
                let (into, beside) = (into.clone(), self.store.snapshot_dir(id, &request.id));
                // Watched as it is written: snapshots of other workloads may be admitted beside it.
                let mut watch = self.floor_watch();
                blocking(move || {
                    // On disk before snapshot.json says the snapshot exists.
                    let disk = crate::durable::FilesystemSync::begin(&beside)?;
                    let copied = crate::tree::copy_tree(&data, &into, None, &[], &mut |copied| {
                        watch.wrote(copied).map_err(|e| e.to_string())
                    })?;
                    disk.finish().map(|()| copied)
                })
                .await?
                .map_err(|error| archive::copy_failed("copying the data", error))
            })
            .await;
        let copied = match copied {
            Ok(copied) => copied,
            Err(e) => {
                let _ = self.remove_snapshot(id, &request.id).await;
                return Err(e);
            }
        };
        let view = SnapshotView {
            id: request.id.clone(),
            workload: id.clone(),
            epoch: record.epoch,
            created_at: now_str(),
            size_bytes: copied.walked.bytes,
            files: copied.walked.files,
            method: copied.method,
            quiesced: running,
            spec_digest: record.spec_digest.clone(),
            duration_ms: started.elapsed().as_millis() as u64,
            memory,
        };
        let finished = {
            let (store, view) = (self.store.clone(), view.clone());
            blocking(move || store.finish_snapshot(&view)).await?
        };
        if let Err(e) = finished {
            let _ = self.remove_snapshot(id, &request.id).await;
            return Err(e.into());
        }
        tracing::info!(
            workload = %id,
            snapshot = %view.id,
            bytes = view.size_bytes,
            method = view.method.as_str(),
            took_ms = view.duration_ms,
            "snapshot taken"
        );
        Ok(SnapshotResponse { created: true, snapshot: view })
    }

    /// A fork into `id` (created, never started) from another workload's snapshot that takes that
    /// workload's memory too: where the snapshot was taken of it asleep with the memory it still
    /// sleeps with, on the same tier, and `id`'s spec is what that memory holds. The fork is then
    /// asleep, and its first start resumes it. None when it can't; the fork then copies the
    /// snapshot's disk alone, and the log says why.
    pub(super) async fn fork_with_memory(
        &self,
        id: &WorkloadId,
        record: &WorkloadRecord,
        fork: &crate::protocol::ForkSource,
    ) -> Result<Option<RestoreResponse>, NodeError> {
        let started = Instant::now();
        let view = self
            .store
            .snapshot(&fork.workload, &fork.snapshot)
            .ok_or_else(|| NodeError::SnapshotNotFound(fork.snapshot.to_string()))?;
        let Some(image) = view.memory.as_deref() else { return Ok(None) };
        let Ok(source) = self.record(&fork.workload) else { return Ok(None) };
        if source.id == *id
            || self.record_tier(&source) != self.record_tier(record)
            || !self.suspension(&source.id).suspended()
        {
            return Ok(None);
        }
        let runtime = self.record_runtime(record)?;
        if let Err(problem) = runtime.fork_memory(&source.container_name, &record.container_name, image).await {
            tracing::info!(workload = %id, source = %source.id, %problem, "the fork copies the snapshot's disk without its memory");
            return Ok(None);
        }
        // Asleep with its source's memory, as a suspended workload is: its first start resumes it.
        self.save_suspension(id, SuspendRecord { suspended_at: Some(now_str()), ..self.suspension(id) }).await?;
        self.state.lock().unwrap().disk.remove(id);
        // Its view reads the runtime's paused copy from here on, as after a suspend.
        self.observe(record).await?;
        tracing::info!(workload = %id, source = %source.id, "forked with the source's memory");
        Ok(Some(RestoreResponse {
            size_bytes: view.size_bytes,
            sha256: None,
            entries: view.files,
            skipped: 0,
            unpacked_bytes: view.size_bytes,
            previous_data: None,
            duration_ms: started.elapsed().as_millis() as u64,
        }))
    }

    pub fn snapshots(&self, id: &WorkloadId) -> Result<SnapshotList, NodeError> {
        self.record(id)?;
        Ok(SnapshotList { snapshots: self.store.snapshots(id) })
    }

    /// Removes a snapshot. Nothing to remove is not an error: the workload or the snapshot may
    /// already be gone, with the workload's data, and the control plane only needs it gone.
    pub async fn delete_snapshot(
        self: &Arc<Self>,
        id: WorkloadId,
        snapshot: SnapshotId,
    ) -> Result<SnapshotDeleteResponse, NodeError> {
        let lock = self.lock_for(&id);
        let _guard = lock.lock().await;
        if self.state.lock().unwrap().uploading.contains(&(id.clone(), snapshot.clone())) {
            return Err(NodeError::Conflict {
                code: ConflictCode::SnapshotBusy,
                message: "the snapshot is being uploaded; delete it once that ends".into(),
            });
        }
        let existed = self.remove_snapshot(&id, &snapshot).await?;
        if existed {
            tracing::info!(workload = %id, %snapshot, "snapshot removed");
        }
        Ok(SnapshotDeleteResponse { existed })
    }

    /// Removes a snapshot's tree, finished or not, off the async threads. Returns whether there was
    /// one.
    async fn remove_snapshot(&self, id: &WorkloadId, snapshot: &SnapshotId) -> Result<bool, NodeError> {
        let (store, id, snapshot) = (self.store.clone(), id.clone(), snapshot.clone());
        Ok(blocking(move || store.remove_snapshot(&id, &snapshot)).await??)
    }

    /// A snapshot as a gzip tarball, PUT to a presigned URL. It doesn't hold the workload's lock:
    /// a snapshot never changes, and an upload of a large one shouldn't hold up a stop.
    pub async fn upload_snapshot(
        self: &Arc<Self>,
        id: WorkloadId,
        snapshot: SnapshotId,
        request: UploadRequest,
    ) -> Result<ExportResponse, NodeError> {
        let started = Instant::now();
        let result = async {
            if let Some(parts) = &request.parts {
                parts.validate().map_err(NodeError::Invalid)?;
            }
            let uploading = {
                let lock = self.lock_for(&id);
                let _guard = lock.lock().await;
                if self.store.snapshot(&id, &snapshot).is_none() {
                    return Err(NodeError::SnapshotNotFound(snapshot.to_string()));
                }
                self.mark_uploading(&id, &snapshot).ok_or_else(|| NodeError::Conflict {
                    code: ConflictCode::SnapshotBusy,
                    message: "this snapshot is being uploaded already".into(),
                })?
            };
            let root = self.store.snapshot_dir(&id, &snapshot).join("data");
            let packed =
                self.pack_and_put(root, Vec::new(), &request.url, &request.headers, request.parts.as_ref()).await;
            drop(uploading);
            let (packed, parts) = packed?;
            tracing::info!(
                workload = %id,
                %snapshot,
                bytes = packed.size_bytes,
                parts = parts.as_ref().map_or(0, Vec::len),
                "snapshot uploaded"
            );
            Ok(ExportResponse {
                size_bytes: packed.size_bytes,
                sha256: packed.sha256,
                format: "tar.gz".into(),
                entries: packed.entries,
                duration_ms: started.elapsed().as_millis() as u64,
                parts,
            })
        }
        .await;
        self.metrics.operation("upload", &result, started.elapsed());
        result
    }
}
