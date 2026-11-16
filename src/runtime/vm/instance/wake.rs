// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! A wake with memory: restores a sleep's image on this node only, into a host of the same class
//! (CPU model, host kernel, monitor) and onto the very disk the sleep left, then gives the guest
//! its signed resume. Anything else is refused before a monitor starts, leaving a cold boot from
//! the disk as the wake. The sleep that writes the image is `memory.rs`.
use super::memory::{Class, Image, MEMORY, STATE, agent_secret, path_text, sandbox_directory, with_exit};
use super::*;
use crate::runtime::vm::disk;
use pryx_agent::{vm_monitor, vm_sleep};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    time::Instant,
};

impl VmRuntime {
    pub(super) async fn wake_entry(&self, name: &str) -> Result<(), RuntimeError> {
        let entry = self.entry(name)?;
        let transition = self.transition(&entry.id);
        let _transition = transition.lock().await;
        let mut entry = self.entry(name)?;
        let image =
            entry.memory.clone().ok_or_else(|| RuntimeError::Conflict("This VM has no memory to wake".into()))?;
        if self.running_or_retired(&entry.id).await? {
            return Err(RuntimeError::Conflict("This VM is already running".into()));
        }
        let bundle = self.bundle().await?;
        let restored = self.thaw(&bundle, &entry, &image).await;
        // Used once whatever happened: a guest that ran from it has moved on, and a refused
        // restore leaves a cold boot from the disk as the only safe wake.
        self.discard_memory(&mut entry)?;
        match restored {
            Ok(live) => {
                entry.finished = None;
                self.save_entry(entry.clone())?;
                self.state.live.lock().unwrap().insert(entry.id.clone(), Arc::new(tokio::sync::Mutex::new(live)));
                Ok(())
            }
            Err(problem) => {
                tracing::warn!(sandbox = %entry.id, %problem, "VM memory was not restored; a wake boots its disk");
                // Nothing of that boot runs any more: it reads as killed until the cold boot.
                entry.finished = Some(crate::clock::now());
                entry.code = crate::runtime::exit_by(libc::SIGKILL);
                self.save_entry(entry)?;
                Err(problem)
            }
        }
    }

    /// A new monitor from the image: refused before it starts unless this host is of the
    /// image's class and the disk is the one it was taken with. A monitor that started and
    /// failed is killed before this returns.
    pub(super) async fn thaw(&self, bundle: &Bundle, entry: &Entry, image: &Image) -> Result<Live, RuntimeError> {
        let started = Instant::now();
        restorable(bundle, entry, image)?;
        let store = self.memory_directory(entry)?;
        directory(&store)?;
        let (state, memory) = (store.join(STATE), store.join(MEMORY));
        // The same layer files the snapshot names; a file-only guest has none.
        let layers = if entry.spec.image.is_empty() {
            Vec::new()
        } else {
            self.pinned(&entry.spec.image, entry.image_digest.as_deref(), self.state.settings.image_timeout)
                .await?
                .layers
        };
        let lease = self.lease(entry)?;
        let boots = self.state.root.join("boots");
        let directory = sandbox_directory(&boots, &lease)?;
        let deadline = Instant::now() + launch::BOOT_TIMEOUT;
        let mut network = self.network(bundle, directory.path(), &lease, entry, deadline).await?;
        // This wake's gateways listen on ports of their own, which the resume hands the guest.
        let gateways =
            network.as_ref().map(network::Network::guest).transpose()?.and_then(|guest| guest.policy).map(|policy| {
                vm_sleep::Gateways { http: policy.http.target, tls: policy.tls.target, dns: policy.dns.target }
            });
        let channel = directory.path().join(configuration::CHANNEL);
        let api = directory.path().join("api");
        let serial = directory.path().join("serial");
        let mut read = vec![
            bundle.root().join("kernel/vmlinux"),
            bundle.root().join("bootstrap/bootstrap.erofs"),
            state.clone(),
            memory.clone(),
        ];
        read.extend(layers);
        let plan = vm_monitor::Plan {
            program: bundle.root().join("bin/firecracker"),
            arguments: vec!["--api-sock".into(), path_text(&api)?.into()],
            directory: directory.path().into(),
            lock: lease,
            parent: std::process::id(),
            read,
            write: vec![entry.spec.data_dir.join(disk::FILE)],
            role: vm_monitor::Role::Ordinary,
        };
        // The snapshot names the disk by its link here; for a fork it is the fork's own disk.
        monitor::link_disk(directory.path(), &entry.spec.data_dir.join(disk::FILE))?;
        let resume = PathBuf::from(format!("{}_{}", channel.display(), vm_sleep::RESUME_PORT));
        let listener = UnixListener::bind(&resume).map_err(error)?;
        fs::set_permissions(&resume, fs::Permissions::from_mode(0o600)).map_err(error)?;
        let mut monitor = Machine::restore(
            &bundle.root().join("bin/vm-launcher"),
            &plan,
            &serial,
            resume,
            deadline,
            usize::try_from(u64::from(entry.spec.log_max_size_mb) * 1024 * 1024).map_err(error)?,
        )
        .await?;
        let mut stages = launch::Stages::default();
        let restoring =
            Restoring { bundle, entry, image, api, state, memory, channel: channel.clone(), gateways, deadline };
        let ready = self.resumed(&restoring, &mut monitor, &listener, &mut stages).await;
        let (forwarder, nonce) = match ready {
            Ok(ready) => ready,
            Err(problem) => {
                let problem = with_exit(problem, &mut monitor);
                #[cfg(test)]
                diagnostics::failed(&mut monitor, &serial, &entry.spec).await;
                let _ = tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, monitor.kill()).await;
                if let Some(network) = &mut network {
                    let _ = network.stop().await;
                }
                return Err(problem);
            }
        };
        drop(listener);
        tracing::info!(
            sandbox = %entry.id,
            stages = %stages,
            milliseconds = started.elapsed().as_millis(),
            "VM woke with its memory"
        );
        Ok(Live {
            monitor,
            jail: None,
            forwarder,
            network,
            _directory: directory,
            serial,
            channel,
            nonce,
            shutdown_requested: false,
            killing: false,
            stages,
            #[cfg(test)]
            diagnostic_secrets: entry.spec.env.iter().map(|(_, value)| value.clone()).collect(),
        })
    }

    /// The restore itself, then the guest's resume and its agent's answer through a new forwarder.
    async fn resumed(
        &self,
        restoring: &Restoring<'_>,
        monitor: &mut Machine,
        listener: &UnixListener,
        stages: &mut launch::Stages,
    ) -> Result<(Forwarder, String), RuntimeError> {
        let Restoring { bundle, entry, image, api, state, memory, channel, gateways, deadline } = restoring;
        while !api.exists() {
            if monitor.exited().map_err(error)?.is_some() || Instant::now() >= *deadline {
                return Err(error("VM monitor did not open its API for a restore"));
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        Self::monitor_action(
            bundle,
            api,
            json!({"action": "load", "snapshot": path_text(state)?, "memory": path_text(memory)?}),
            launch::BOOT_TIMEOUT,
        )
        .await?;
        stages.mark("restore");
        let nonce = self.resume_guest(entry, image, *gateways, listener, monitor, *deadline).await?;
        stages.mark("resume");
        let forwarder = self.forward(bundle, channel, entry, *deadline).await?;
        // A file-only guest's agent binding takes whatever port its forwarder was given.
        let mut spec = entry.spec.clone();
        if let [binding] = spec.ports.as_mut_slice()
            && binding.host_port == 0
        {
            binding.host_port = forwarder.addresses()[0].port();
        }
        launch::ready(monitor, &spec, *deadline).await?;
        stages.mark("agent");
        Ok((forwarder, nonce))
    }

    /// Gives the restored guest's init its signed resume: the host's time and fresh entropy, this
    /// wake's gateway ports, and for a fork's copy this entry's own identity, signed by the
    /// source's the guest still holds. Answers the boot nonce the guest has from then on.
    async fn resume_guest(
        &self,
        entry: &Entry,
        image: &Image,
        gateways: Option<vm_sleep::Gateways>,
        listener: &UnixListener,
        monitor: &Machine,
        deadline: Instant,
    ) -> Result<String, RuntimeError> {
        let boot = entry.spec.agent.as_ref().ok_or_else(|| error("VM agent bootstrap is missing"))?;
        let (key, sandbox, epoch, identity) = match &image.guest {
            None => (agent::key(&entry.spec)?, boot.sandbox.clone(), boot.epoch, None),
            Some(source) => (
                pryx_agent::auth::SigningKey::from_encoded(&source.key).map_err(error)?,
                source.sandbox.clone(),
                source.epoch,
                Some(vm_sleep::Identity {
                    sandbox: boot.sandbox.clone(),
                    epoch: boot.epoch,
                    key: agent_secret(&entry.spec)?,
                    nonce: uuid::Uuid::new_v4().simple().to_string(),
                    hostname: format!("pryx-{}", entry.id),
                }),
            ),
        };
        let nonce = identity.as_ref().map_or_else(|| image.nonce.clone(), |identity| identity.nonce.clone());
        let pid = monitor.pid().and_then(|pid| i32::try_from(pid).ok());
        // What the listener saw, for a resume that never came: no connection, or others'.
        let mut others = Vec::new();
        let mut asked = false;
        let exchange = tokio::time::timeout_at(deadline, async {
            let mut socket = loop {
                let (socket, _) = listener.accept().await.map_err(error)?;
                let peer = socket.peer_cred().map_err(error)?.pid();
                if pid.is_some() && peer == pid {
                    break socket;
                }
                others.push(peer);
            };
            asked = true;
            let seed = ring::rand::generate::<[u8; vm_sleep::SEED_BYTES]>(&ring::rand::SystemRandom::new())
                .map_err(|_| error("Host entropy is unavailable"))?
                .expose();
            let now = crate::clock::now().unix_timestamp_nanos();
            let handed = vm_sleep::Handed { identity, gateways };
            let resume = vm_sleep::fork(&key, &image.nonce, &sandbox, epoch, now, &seed, handed);
            let mut line = serde_json::to_vec(&resume).map_err(error)?;
            line.push(b'\n');
            socket.write_all(&line).await.map_err(error)?;
            let mut acknowledgement = Vec::new();
            tokio::io::BufReader::new((&mut socket).take(16))
                .read_until(b'\n', &mut acknowledgement)
                .await
                .map_err(error)?;
            if acknowledgement != b"true\n" {
                return Err(error("VM guest init did not take its resume"));
            }
            Ok(nonce)
        })
        .await;
        exchange.map_err(|_| unresumed(asked, pid, &others))?
    }
}

/// What one restore reads and where its monitor answers.
struct Restoring<'a> {
    bundle: &'a Bundle,
    entry: &'a Entry,
    image: &'a Image,
    api: PathBuf,
    state: PathBuf,
    memory: PathBuf,
    channel: PathBuf,
    gateways: Option<vm_sleep::Gateways>,
    deadline: Instant,
}

/// A failed monitor action, with how the monitor ended if it has: a seccomp trap reads as SIGSYS.
/// What a restore's resume listener saw when no resume came: an unanswered ask, or no ask at all.
fn unresumed(asked: bool, monitor: Option<i32>, others: &[Option<i32>]) -> RuntimeError {
    RuntimeError::Timeout(if asked {
        "VM guest init asked for its resume and did not acknowledge it".into()
    } else {
        format!("VM guest init did not ask for its resume (monitor {monitor:?}; other peers {others:?})")
    })
}

/// Whether this host and this sandbox are still the ones the image was taken on and for: the same
/// class, the very disk the sleep left, and the same memory and CPUs; and, for a guest with a
/// network, a guest init that takes this wake's gateway ports.
fn restorable(bundle: &Bundle, entry: &Entry, image: &Image) -> Result<(), RuntimeError> {
    if Class::current(&bundle.root().join("bin/firecracker"))? != image.class {
        return Err(RuntimeError::Conflict(
            "This VM's memory was taken on a host of another class; it wakes by booting its disk".into(),
        ));
    }
    if disk::Stamp::current(&entry.spec.data_dir)? != Some(image.disk) {
        return Err(RuntimeError::Conflict("This VM's disk changed after its memory was taken".into()));
    }
    if entry.spec.memory_bytes != image.memory_bytes || entry.spec.nano_cpus != image.nano_cpus {
        return Err(RuntimeError::Conflict("This VM's memory or CPUs changed after its memory was taken".into()));
    }
    // A woken guest's redirects must take the new gateways' ports, or its DNS and egress fail.
    if !entry.spec.internal_network && bundle.bootstrap().metadata.vm_sleep_version < vm_sleep::GATEWAY_VERSION {
        return Err(RuntimeError::Conflict(
            "This VM bundle's guest init can't take a wake's gateway ports; a VM with a network wakes by booting its disk"
                .into(),
        ));
    }
    Ok(())
}
