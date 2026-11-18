// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! A fork with memory on one node. A guest asleep with its memory gives a new sandbox a copy of its
//! disk (sharing blocks where the filesystem can) and the very same memory file, linked; the fork
//! then wakes as itself: its resume, signed by the identity the guest still holds, hands it its own
//! sandbox, epoch, agent key, boot nonce and hostname, and every restore gives the guest a new VM
//! generation ID. The node's restore takes this path for a fork from a snapshot of a sleeping
//! guest (`manager/archive.rs`); across nodes a fork copies the disk, since memory never leaves
//! a node.
use super::memory::{self, Image, Source};
use super::*;
use crate::runtime::ContainerRuntime;

impl VmRuntime {
    /// Makes `spec` a fork of `source`, which must be asleep with its memory; the fork sleeps too,
    /// and a start wakes it from that memory. Everything of the spec the memory holds (image,
    /// sizes, mounts, user, network, variables, ports) must match.
    pub async fn fork_with_memory(&self, source: &str, spec: &ContainerSpec) -> Result<String, RuntimeError> {
        let (own, source, spec) = (self.clone(), source.to_owned(), spec.clone());
        tokio::spawn(async move { own.fork_entry(&source, &spec).await }).await.map_err(error)?
    }

    async fn fork_entry(&self, source: &str, spec: &ContainerSpec) -> Result<String, RuntimeError> {
        let id = self.create(spec).await?;
        if let Err(problem) = self.fork_into(source, &id, None).await {
            let _ = self.forget_entry(&id);
            return Err(problem);
        }
        Ok(id)
    }

    /// Makes `target`, a sandbox created and never started, a fork of `source` asleep with its
    /// memory: with `image`, only while that is the memory it sleeps with.
    pub(super) async fn fork_into(&self, source: &str, target: &str, image: Option<&str>) -> Result<(), RuntimeError> {
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
        let memory = source.memory.clone().ok_or_else(|| {
            RuntimeError::Conflict("A fork with memory needs its source asleep with its memory".into())
        })?;
        if image.is_some_and(|image| image.is_empty() || image != memory.id) {
            return Err(RuntimeError::Conflict(
                "The source no longer sleeps with the memory its snapshot was taken with".into(),
            ));
        }
        let fork = self.entry(target)?;
        let fork_transition = self.transition(&fork.id);
        let _fork_transition = fork_transition.lock().await;
        let fork = self.entry(&fork.id)?;
        if fork.started.is_some() || fork.memory.is_some() || fork.id == source.id {
            return Err(RuntimeError::Conflict("A fork with memory goes into a sandbox that never started".into()));
        }
        same_shape(&source.spec, &fork.spec)?;
        if fs::read_dir(&fork.spec.data_dir).is_ok_and(|mut entries| entries.next().is_some()) {
            return Err(RuntimeError::Conflict("A fork's data directory must be new".into()));
        }
        let made = self.fork_files(&source, &memory, &fork.id).await;
        if made.is_err()
            && let Ok(mut fork) = self.entry(&fork.id)
        {
            let _ = self.discard_memory(&mut fork);
        }
        made
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
            id: uuid::Uuid::new_v4().simple().to_string(),
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

/// What of a spec a guest's memory holds: all of it but what is each sandbox's own (its name,
/// labels, data directory and cgroup, its ports' host side, the agent's sandbox, epoch and key,
/// which the resume hands the fork) and the host's log bounds and OOM score. A fork that differs in
/// anything else (an image, a size, a mount, a user, a network, a variable or secret, a port inside)
/// can't take the memory: it would run with its source's and drop its own in silence.
fn memory_shape(spec: &ContainerSpec) -> Result<serde_json::Value, RuntimeError> {
    let mut spec = spec.clone();
    let agent = spec.agent.as_mut().ok_or_else(|| error("VM agent bootstrap is missing"))?;
    let secret = std::mem::take(&mut agent.secret_env);
    agent.sandbox.clear();
    agent.epoch = 0;
    spec.env.retain(|(name, _)| *name != secret);
    spec.env.sort();
    spec.name.clear();
    spec.labels.clear();
    spec.data_dir = PathBuf::new();
    spec.cgroup_parent = None;
    for port in &mut spec.ports {
        port.host_port = 0;
        port.host_ips.clear();
    }
    (spec.log_max_size_mb, spec.log_max_files, spec.oom_score_adj) = (0, 0, 0);
    // A serializer's error could quote a value, a secret's among them.
    serde_json::to_value(&spec).map_err(|_| error("A VM specification can't be compared"))
}

fn same_shape(source: &ContainerSpec, fork: &ContainerSpec) -> Result<(), RuntimeError> {
    if memory_shape(source)? != memory_shape(fork)? {
        return Err(RuntimeError::Conflict(
            "A fork with memory keeps its source's image, sizes, mounts, user, network, variables and ports".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fork_with_memory_differs_from_its_source_only_in_what_is_its_own() {
        let root = tempfile::tempdir().unwrap();
        let source = super::super::tests::spec(&root.path().join("source"), "alpine:3.24");
        let fork = super::super::tests::spec(&root.path().join("fork"), "alpine:3.24");
        same_shape(&source, &fork).unwrap();
        let mut secret = fork.clone();
        secret.env.push(("API_TOKEN".into(), "new".into()));
        let mut port = fork.clone();
        port.ports[0].container_port += 1;
        let mut command = fork.clone();
        command.entrypoint = Some(vec!["/bin/true".into()]);
        let mut network = fork.clone();
        network.internal_network = false;
        for changed in [secret, port, command, network] {
            assert!(matches!(same_shape(&source, &changed), Err(RuntimeError::Conflict(_))));
        }
    }
}
