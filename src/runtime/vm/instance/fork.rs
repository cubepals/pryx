// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! A fork with memory on one node. A guest asleep with its memory gives a new sandbox a copy of its
//! disk (sharing blocks where the filesystem can) and the very same memory file, linked; the fork
//! then wakes as itself: its resume, signed by the identity the guest still holds, hands it its own
//! sandbox, epoch, agent key, boot nonce and hostname, and every restore gives the guest a new VM
//! generation ID. Across nodes a fork copies the disk, as before, since memory never leaves a node.
use super::memory::{self, Image, Source};
use super::*;
use crate::runtime::ContainerRuntime;

impl VmRuntime {
    /// Makes `spec` a fork of `source`, which must be asleep with its memory; the fork sleeps too,
    /// and a start wakes it from that memory. Shapes the memory holds (image, size, mounts, user,
    /// network) must match.
    pub async fn fork_with_memory(&self, source: &str, spec: &ContainerSpec) -> Result<String, RuntimeError> {
        let (own, source, spec) = (self.clone(), source.to_owned(), spec.clone());
        tokio::spawn(async move { own.fork_entry(&source, &spec).await }).await.map_err(error)?
    }

    async fn fork_entry(&self, source: &str, spec: &ContainerSpec) -> Result<String, RuntimeError> {
        if self.state.settings.monitor_owner.is_some() {
            return Err(RuntimeError::Unavailable(memory::ROOT_REFUSAL.into()));
        }
        let bundle = self.bundle().await?;
        if bundle.bootstrap().metadata.vm_sleep_version < pryx_agent::vm_sleep::FORK_VERSION {
            return Err(RuntimeError::Unavailable("This VM bundle's guest init can't take a fork's identity".into()));
        }
        let entry = self.entry(source)?;
        let transition = self.transition(&entry.id);
        let _transition = transition.lock().await;
        let source = self.entry(&entry.id)?;
        let image = source.memory.clone().ok_or_else(|| {
            RuntimeError::Conflict("A fork with memory needs its source asleep with its memory".into())
        })?;
        same_shape(&source.spec, spec)?;
        if fs::read_dir(&spec.data_dir).is_ok_and(|mut entries| entries.next().is_some()) {
            return Err(RuntimeError::Conflict("A fork's data directory must be new".into()));
        }
        let id = self.create(spec).await?;
        let made = self.fork_files(&source, &image, &id).await;
        if let Err(problem) = made {
            if let Ok(mut fork) = self.entry(&id) {
                let _ = self.discard_memory(&mut fork);
            }
            let _ = self.forget_entry(&id);
            return Err(problem);
        }
        Ok(id)
    }

    /// The fork's disk copy, its links to the source's memory files and its image record.
    async fn fork_files(&self, source: &Entry, image: &Image, id: &str) -> Result<(), RuntimeError> {
        let mut fork = self.entry(id)?;
        // While this lease is held the source can't start, so its disk holds still for the copy.
        let _source = lock(&self.lease(source)?)?;
        let (from, to) = (source.spec.data_dir.clone(), fork.spec.data_dir.clone());
        if to.exists() {
            fs::remove_dir(&to).map_err(error)?;
        }
        let parent = to.parent().ok_or_else(|| error("A fork's data directory needs a parent"))?.to_owned();
        let floor = self.state.settings.free_floor_bytes;
        tokio::task::spawn_blocking(move || {
            crate::tree::copy_tree(&from, &to, None, &[], &mut |_| match crate::host::disk(&parent) {
                Some((_, available)) if available >= floor => Ok(()),
                _ => Err("A fork's disk copy would pass the host's free-space floor".into()),
            })
        })
        .await
        .map_err(error)?
        .map_err(error)?;
        let (shared, own) = (self.memory_directory(source)?, self.memory_directory(&fork)?);
        directory(&self.state.root.join("memory"))?;
        directory(&own)?;
        for name in [memory::STATE, memory::MEMORY] {
            // One memory file for both: each restore maps it privately and never writes it.
            fs::hard_link(shared.join(name), own.join(name)).map_err(error)?;
        }
        File::open(&own).and_then(|directory| directory.sync_all()).map_err(error)?;
        let boot = source.spec.agent.as_ref().ok_or_else(|| error("VM agent bootstrap is missing"))?;
        let guest = match &image.guest {
            Some(guest) => guest.clone(),
            None => {
                Source { sandbox: boot.sandbox.clone(), epoch: boot.epoch, key: memory::agent_secret(&source.spec)? }
            }
        };
        fork.memory = Some(Image {
            disk: super::super::disk::Stamp::current(&fork.spec.data_dir)?
                .ok_or_else(|| error("A fork's disk is missing after its copy"))?,
            guest: Some(guest),
            ..image.clone()
        });
        fork.image_digest = source.image_digest.clone();
        fork.started = Some(crate::clock::now());
        fork.finished = None;
        self.save_entry(fork)
    }
}

/// What a guest's memory holds of its devices, sizes, mounts, user and network, which a fork's
/// own spec can't change.
fn same_shape(source: &ContainerSpec, fork: &ContainerSpec) -> Result<(), RuntimeError> {
    let egress = |spec: &ContainerSpec| serde_json::to_value(&spec.egress).ok();
    if source.image != fork.image
        || source.memory_bytes != fork.memory_bytes
        || source.nano_cpus != fork.nano_cpus
        || source.storage_bytes != fork.storage_bytes
        || source.mount_path != fork.mount_path
        || source.user != fork.user
        || source.internal_network != fork.internal_network
        || egress(source) != egress(fork)
        || source.read_only_rootfs != fork.read_only_rootfs
        || source.tmp_size_mb != fork.tmp_size_mb
        || source.pids_limit != fork.pids_limit
    {
        return Err(RuntimeError::Conflict(
            "A fork with memory keeps its source's image, size, mounts, user and network".into(),
        ));
    }
    Ok(())
}
