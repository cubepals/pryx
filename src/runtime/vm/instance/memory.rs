// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Memory sleep for an ordinary user's VM. Sleep has the guest flush its disk and hold a
//! connection, pauses the monitor, writes its full snapshot, and retires it; wake (`wake.rs`)
//! restores that snapshot on this node only, into a host of the same class (CPU model, host
//! kernel, monitor), and refuses otherwise, leaving a cold boot from the disk as the wake. An
//! image is used once, and is discarded as soon as anything else touches its disk. The memory
//! files hold guest secrets, so they stay in this runtime's private state, owned and 0600 like the
//! disk, and are never part of a data copy, an export or a transfer. The jailer path does not
//! sleep this way.
use super::*;
use crate::runtime::vm::disk;
use pryx_agent::vm_sleep;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use tokio::{
    net::{UnixListener, UnixStream},
    time::Instant,
};

/// What a restore must find again: the host's CPU, its kernel and the monitor's build.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Class {
    pub(super) cpu: String,
    pub(super) kernel: String,
    pub(super) monitor: String,
}

/// One guest's memory as its last sleep wrote it, beside the disk state it belongs to.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Image {
    /// This sleep's own name, which a snapshot of the guest asleep records so that a fork on this
    /// node can take this memory with that snapshot's disk; empty in an image older than it.
    #[serde(default)]
    pub(super) id: String,
    /// The verified bootstrap this memory actually booted: its init and network helper stay in
    /// memory across restores, so a new bundle cannot supply their capabilities to an old image.
    #[serde(default)]
    pub(super) bootstrap_sha256: String,
    pub(super) class: Class,
    pub(super) disk: disk::Stamp,
    /// The boot the guest still is, which its init's resume must name.
    pub(super) nonce: String,
    pub(super) memory_bytes: i64,
    pub(super) nano_cpus: Option<i64>,
    /// For a fork's copy: the identity the guest in this memory still holds, its source's, which
    /// signs the resume that hands it its own. Absent when that is this entry's own identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) guest: Option<Source>,
}

/// The identity a fork's source left in the memory it shares: its sandbox, epoch and agent key.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Source {
    pub(super) sandbox: String,
    pub(super) epoch: u64,
    pub(super) key: String,
}

/// The encoded agent signing key a spec carries, as its bootstrap hands it to the guest.
pub(super) fn agent_secret(spec: &ContainerSpec) -> Result<String, RuntimeError> {
    let boot = spec.agent.as_ref().ok_or_else(|| error("VM agent bootstrap is missing"))?;
    spec.env
        .iter()
        .find(|(name, _)| name == &boot.secret_env)
        .map(|(_, value)| value.clone())
        .ok_or_else(|| error("VM signing material is missing"))
}

/// The CPU fields of the first processor that decide which guest CPU state a host can take.
const CPU_FIELDS: [&str; 8] =
    ["vendor_id", "cpu family", "model", "stepping", "CPU implementer", "CPU architecture", "CPU variant", "CPU part"];
pub(super) const STATE: &str = "state";
pub(super) const MEMORY: &str = "memory";
/// The guest's own flush and the host's hold, before any pause.
const GUEST_TIMEOUT: Duration = Duration::from_secs(30);
// A bound on a stuck write, not an expected time: the slowest shared disk's sequential floor.
const SNAPSHOT_FLOOR_BYTES_PER_SECOND: u64 = 16 * 1024 * 1024;

impl Class {
    pub(super) fn current(monitor: &Path) -> Result<Self, RuntimeError> {
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").map_err(error)?;
        let first = cpuinfo.split("\n\n").next().unwrap_or_default();
        let field = |name: &str| {
            first.lines().find_map(|line| {
                line.split_once(':').filter(|(key, _)| key.trim() == name).map(|(_, value)| value.trim().to_owned())
            })
        };
        let mut cpu = CPU_FIELDS
            .iter()
            .filter_map(|&name| field(name).map(|value| format!("{name}={value}")))
            .collect::<Vec<_>>();
        let flags = field("flags").or_else(|| field("Features")).unwrap_or_default();
        cpu.push(format!("flags={}", &super::super::filesystems::sha256(flags.as_bytes()).map_err(error)?[..16]));
        Ok(Self {
            cpu: cpu.join(" "),
            kernel: fs::read_to_string("/proc/sys/kernel/osrelease").map_err(error)?.trim().to_owned(),
            monitor: super::super::filesystems::sha256(File::open(monitor).map_err(error)?).map_err(error)?,
        })
    }
}

pub(super) fn path_text(path: &Path) -> Result<&str, RuntimeError> {
    path.to_str().ok_or_else(|| error("VM memory paths must be UTF-8"))
}

impl VmRuntime {
    /// The private directory one sandbox's memory image lives in.
    pub(super) fn memory_directory(&self, entry: &Entry) -> Result<PathBuf, RuntimeError> {
        let lease = self.lease(entry)?;
        let name = lease.file_stem().and_then(|name| name.to_str()).ok_or_else(|| error("Invalid VM lease name"))?;
        Ok(self.state.root.join("memory").join(name))
    }

    /// Forgets a memory image: its files first, then the record. A cold boot or anything that
    /// touches the disk comes after this, never before.
    pub(super) fn discard_memory(&self, entry: &mut Entry) -> Result<(), RuntimeError> {
        let directory = self.memory_directory(entry)?;
        match fs::remove_dir_all(&directory) {
            Ok(()) => (),
            Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => (),
            Err(problem) => return Err(error(problem)),
        }
        if entry.memory.take().is_some() {
            self.save_entry(entry.clone())?;
        }
        Ok(())
    }

    /// Takes a sleep's image out of its sandbox's record before any monitor may load it: its files
    /// move to a name no journal knows, and the record then says it has none. Answers where the
    /// files are now; a crash after this never restores them, and the next start sweeps them.
    pub(super) fn consume(&self, entry: &mut Entry) -> Result<PathBuf, RuntimeError> {
        let directory = self.memory_directory(entry)?;
        let consumed = directory.with_extension("consumed");
        match fs::remove_dir_all(&consumed) {
            Ok(()) => (),
            Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => (),
            Err(problem) => return Err(error(problem)),
        }
        fs::rename(&directory, &consumed).map_err(error)?;
        File::open(self.state.root.join("memory")).and_then(|directory| directory.sync_all()).map_err(error)?;
        entry.memory = None;
        self.save_entry(entry.clone())?;
        Ok(consumed)
    }

    /// Monitor actions go through the bundle's SDK transport, to the monitor's own API socket.
    pub(super) async fn monitor_action(
        bundle: &Bundle,
        api: &Path,
        mut action: serde_json::Value,
        timeout: Duration,
    ) -> Result<(), RuntimeError> {
        action["timeoutMs"] = json!(u64::try_from(timeout.as_millis()).map_err(error)?);
        let (status, body) = super::super::transport::request(
            &bundle.root().join("bin/vm-transport"),
            &json!({"socket": path_text(api)?, "monitor": action}),
            timeout + crate::runtime::ENGINE_PROBE_TIMEOUT,
            pryx_agent::vm_boot::MAX_BYTES,
        )
        .await?;
        if !status.is_success() {
            // The monitor's fault message names its own state and paths, never guest secrets.
            return Err(error(format!("VM monitor refused {status}: {}", String::from_utf8_lossy(&body))));
        }
        Ok(())
    }

    pub(super) async fn sleep_entry(&self, name: &str) -> Result<(), RuntimeError> {
        let entry = self.entry(name)?;
        let transition = self.transition(&entry.id);
        let _transition = transition.lock().await;
        let mut entry = self.entry(name)?;
        if self.state.settings.monitor_owner.is_some() {
            return Err(RuntimeError::Unavailable(ROOT_REFUSAL.into()));
        }
        let live = self.state.live.lock().unwrap().get(&entry.id).cloned();
        let live =
            live.ok_or_else(|| RuntimeError::Conflict("A VM sleeps with its memory only while it runs".into()))?;
        let mut live = live.lock().await;
        let bundle = self.bundle().await?;
        self.discard_memory(&mut entry)?;
        let image = match self.freeze(&bundle, &entry, &mut live).await {
            Ok(image) => image,
            Err(problem) => {
                if live.monitor.exited().map_err(error)?.is_some() {
                    // Retired, whatever stopped it: nothing of that boot runs, and its next start
                    // boots the disk.
                    retired(&mut live).await;
                    drop(live);
                    self.state.live.lock().unwrap().remove(&entry.id);
                    entry.code = crate::runtime::exit_by(libc::SIGKILL);
                    entry.finished = Some(crate::clock::now());
                    entry.clean_disk = None;
                    self.save_entry(entry)?;
                }
                return Err(problem);
            }
        };
        drop(live);
        self.state.live.lock().unwrap().remove(&entry.id);
        entry.memory = Some(image);
        entry.clean_disk = None;
        if let Some(snapshot) = &mut entry.agent_state {
            snapshot.activity.streams = 0;
        }
        self.save_entry(entry)
    }

    /// Flush, hold, pause, snapshot, retire: the guest's memory moves into its private image
    /// directory and its monitor never runs again. Any failure before the snapshot exists
    /// leaves the guest running where it was.
    async fn freeze(&self, bundle: &Bundle, entry: &Entry, live: &mut Live) -> Result<Image, RuntimeError> {
        if live.shutdown_requested || live.killing || live.monitor.exited().map_err(error)?.is_some() {
            return Err(RuntimeError::Conflict("A VM sleeps with its memory only while it runs".into()));
        }
        let class = Class::current(&bundle.root().join("bin/firecracker"))?;
        let memory_bytes = u64::try_from(entry.spec.memory_bytes).map_err(error)?;
        let store = self.memory_directory(entry)?;
        directory(&self.state.root.join("memory"))?;
        directory(&store)?;
        match crate::host::disk(&store) {
            Some((_, available)) if available >= memory_bytes.saturating_add(self.state.settings.free_floor_bytes) => {}
            _ => return Err(error("VM memory sleep would pass the host's free-space floor")),
        }
        let boot = live._directory.path().to_owned();
        let (state, memory) = (boot.join("memory.state"), boot.join("memory.bin"));
        let started = Instant::now();
        let hold = self.hold(bundle, entry, live).await?;
        let api = boot.join("api");
        let snapshot_timeout =
            Duration::from_secs(memory_bytes.div_ceil(SNAPSHOT_FLOOR_BYTES_PER_SECOND)) + GUEST_TIMEOUT;
        let taken = async {
            Self::monitor_action(bundle, &api, json!({"action": "pause"}), GUEST_TIMEOUT).await?;
            Self::monitor_action(
                bundle,
                &api,
                json!({"action": "create", "snapshot": path_text(&state)?, "memory": path_text(&memory)?}),
                snapshot_timeout,
            )
            .await
        }
        .await;
        if let Err(problem) = taken {
            let problem = with_exit(problem, &mut live.monitor);
            // A resume the monitor confirms lets the guest go on where it was; its hold's end
            // finds no resume. Unconfirmed, the guest may stay paused or its monitor still be
            // writing the files: it is retired, and the files go only once it has exited.
            let resumed = Self::monitor_action(bundle, &api, json!({"action": "resume"}), GUEST_TIMEOUT).await;
            drop(hold);
            let recovered = async {
                resumed?;
                // Running vCPUs do not confirm that init has ended its resume wait, rebuilt
                // the reset network transport and thawed the agent's workload cgroup.
                let deadline = Instant::now() + GUEST_TIMEOUT;
                tokio::time::timeout_at(deadline, launch::ready(&mut live.monitor, &entry.spec, deadline))
                    .await
                    .map_err(|_| RuntimeError::Timeout("VM agent did not recover after a failed snapshot".into()))??;
                if live.monitor.exited().map_err(error)?.is_some() {
                    return Err(error("VM monitor exited during snapshot recovery"));
                }
                Ok(())
            }
            .await;
            if let Err(unrecovered) = recovered {
                live.killing = true;
                let killed = tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, live.monitor.kill()).await;
                let retirement = if matches!(killed, Ok(Ok(()))) {
                    let _ = fs::remove_file(&state);
                    let _ = fs::remove_file(&memory);
                    "the guest was retired"
                } else {
                    "monitor retirement remains unconfirmed and its lease is retained"
                };
                return Err(error(format!("{problem}; its recovery failed too: {unrecovered}; {retirement}")));
            }
            let _ = fs::remove_file(&state);
            let _ = fs::remove_file(&memory);
            return Err(problem);
        }
        live.killing = true;
        tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, live.monitor.kill())
            .await
            .map_err(|_| RuntimeError::Timeout("VM monitor retirement after its snapshot is unconfirmed".into()))?
            .map_err(error)?;
        drop(hold);
        live.forwarder.stop().await.map_err(error)?;
        if let Some(network) = &mut live.network {
            network.stop().await.map_err(error)?;
        }
        let _ = tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, live.monitor.finish_serial()).await;
        for (from, to) in [(&state, STATE), (&memory, MEMORY)] {
            fs::set_permissions(from, fs::Permissions::from_mode(0o600)).map_err(error)?;
            fs::rename(from, store.join(to)).map_err(error)?;
        }
        File::open(&store).and_then(|directory| directory.sync_all()).map_err(error)?;
        tracing::info!(sandbox = %entry.id, milliseconds = started.elapsed().as_millis(), "VM slept with its memory");
        Ok(Image {
            id: uuid::Uuid::new_v4().simple().to_string(),
            bootstrap_sha256: bundle.bootstrap().image_sha256.clone(),
            class,
            disk: disk::Stamp::current(&entry.spec.data_dir)?.ok_or_else(|| error("VM disk is missing after sleep"))?,
            nonce: live.nonce.clone(),
            memory_bytes: entry.spec.memory_bytes,
            nano_cpus: entry.spec.nano_cpus,
            guest: None,
        })
    }

    /// The guest's flush and its held connection, which ends when this guest is resumed or
    /// given up on. Its peer must be this boot's monitor.
    async fn hold(&self, bundle: &Bundle, entry: &Entry, live: &Live) -> Result<UnixStream, RuntimeError> {
        let deadline = Instant::now() + GUEST_TIMEOUT;
        let path = PathBuf::from(format!("{}_{}", live.channel.display(), vm_sleep::HOLD_PORT));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).map_err(error)?;
        let _bound = Bound(path.clone());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(error)?;
        let boot = entry.spec.agent.as_ref().ok_or_else(|| error("VM agent bootstrap is missing"))?;
        let request = vm_sleep::request(&agent::key(&entry.spec)?, &live.nonce, &boot.sandbox, boot.epoch);
        let (status, _) = super::super::transport::request(
            &bundle.root().join("bin/vm-transport"),
            &json!({"socket": live.channel, "port": vm_sleep::PORT, "sleep": request}),
            GUEST_TIMEOUT,
            pryx_agent::vm_boot::MAX_BYTES,
        )
        .await?;
        if !status.is_success() {
            return Err(error("VM guest init refused to sleep"));
        }
        let monitor = live.monitor.pid().and_then(|pid| i32::try_from(pid).ok());
        tokio::time::timeout_at(deadline, async {
            loop {
                let (socket, _) = listener.accept().await.map_err(error)?;
                if monitor.is_some() && socket.peer_cred().map_err(error)?.pid() == monitor {
                    return Ok(socket);
                }
            }
        })
        .await
        .map_err(|_| RuntimeError::Timeout("VM guest did not hold its sleep".into()))?
    }
}

impl VmRuntime {
    /// Runs the restore test in the background; the tier announces `memory-sleep` once it passes.
    pub fn probe_memory_sleep_in_background(&self) {
        if self.state.settings.monitor_owner.is_some() {
            return;
        }
        let own = self.clone();
        tokio::spawn(async move {
            let outcome = own.probe_memory_sleep().await.map_err(|problem| problem.to_string());
            match &outcome {
                Ok(()) => tracing::info!("VM memory sleep passed its restore test"),
                Err(problem) => tracing::warn!(%problem, "VM memory sleep is off: its restore test failed"),
            }
            *own.state.memory_sleep.lock().unwrap() = Some(outcome);
        });
    }

    /// Whether this host's restore test passed; until it has, memory sleep is off.
    pub(super) fn memory_sleep(&self) -> bool {
        matches!(*self.state.memory_sleep.lock().unwrap(), Some(Ok(())))
    }

    /// One file-only guest on a scratch disk, slept and woken through a sandbox's own path.
    pub(super) async fn probe_memory_sleep(&self) -> Result<(), RuntimeError> {
        if self.state.settings.monitor_owner.is_some() {
            return Err(RuntimeError::Unavailable(ROOT_REFUSAL.into()));
        }
        let bundle = self.bundle().await?;
        if bundle.bootstrap().metadata.vm_sleep_version < vm_sleep::SLEEP_VERSION {
            return Err(RuntimeError::Unavailable("This VM bundle's guest init has no memory sleep".into()));
        }
        let root = self.state.root.join("probe");
        match fs::remove_dir_all(&root) {
            Ok(()) => (),
            Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => (),
            Err(problem) => return Err(error(problem)),
        }
        directory(&root)?;
        let data = root.join("data");
        directory(&data)?;
        let entry = Entry {
            id: format!("vm-{}", uuid::Uuid::new_v4().simple()),
            spec: probe_specification(&data)?,
            started: None,
            finished: None,
            code: 0,
            agent_state: None,
            image_digest: None,
            clean_disk: None,
            memory: None,
        };
        let store = self.memory_directory(&entry)?;
        let transition = Arc::new(tokio::sync::Mutex::new(())).lock_owned().await;
        let outcome = match self.launch_files(entry, transition).await {
            Ok(mut guest) => match guest.inner.take() {
                Some(inner) => self.probe_guest(&bundle, inner).await,
                None => Err(error("VM file service is closed")),
            },
            Err(problem) => Err(problem),
        };
        let _ = fs::remove_dir_all(&store);
        let _ = fs::remove_dir_all(&root);
        outcome
    }

    async fn probe_guest(&self, bundle: &Bundle, mut inner: Inner) -> Result<(), RuntimeError> {
        let entry = inner.entry.clone();
        let image = self.freeze(bundle, &entry, &mut inner.live).await;
        drop(inner);
        // Never journaled, so nothing to consume: a crash leaves only files the next start sweeps.
        let mut live = self.thaw(bundle, &entry, &image?, &self.memory_directory(&entry)?).await?;
        live.killing = true;
        let _ = tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, live.monitor.kill()).await;
        live.forwarder.stop().await.map_err(error)
    }
}

/// The scratch sandbox the restore test sleeps and wakes; the file service replaces its key.
fn probe_specification(data: &Path) -> Result<ContainerSpec, RuntimeError> {
    use crate::runtime::{AgentBoot, PortBinding};
    let sandbox = format!("memory-probe-{}", uuid::Uuid::new_v4().simple());
    let secret_env = format!("{}{}", pryx_agent::contract::SECRET_ENV_PREFIX, uuid::Uuid::new_v4().simple());
    let (_, encoded) = pryx_agent::auth::SigningKey::generate().map_err(error)?;
    // Any fixed non-root guest identity: the guest runs no workload and owns only scratch data.
    const PROBE_ID: u32 = 1000;
    Ok(ContainerSpec {
        agent: Some(AgentBoot {
            public_origin: None,
            binary: PathBuf::new(),
            sandbox: sandbox.clone(),
            epoch: 1,
            secret_env: secret_env.clone(),
            domain: "vm.pryx.invalid".into(),
            agent_port: pryx_agent::contract::AGENT_PORT,
            spare_ports: Vec::new(),
            keep_alive: true,
        }),
        name: sandbox,
        cgroup_parent: None,
        image: String::new(),
        packages: Vec::new(),
        entrypoint: None,
        workdir: None,
        egress: None,
        egress_addresses: None,
        env: vec![(secret_env, encoded)],
        labels: BTreeMap::new(),
        user: format!("{PROBE_ID}:{PROBE_ID}"),
        memory_bytes: 128 * 1024 * 1024,
        storage_bytes: 64 * 1024 * 1024,
        cpu_weight_requested: false,
        allow_unenforced_memory: None,
        require_cpu_limit: true,
        require_pids_limit: true,
        require_browser_sandbox: None,
        nano_cpus: None,
        cpu_shares: 1024,
        pids_limit: 64,
        read_only_rootfs: true,
        tmp_size_mb: 16,
        data_dir: data.into(),
        mount_path: "/data".into(),
        ports: vec![PortBinding {
            container_port: pryx_agent::contract::AGENT_PORT,
            protocol: crate::protocol::Proto::Tcp,
            host_ips: vec![std::net::Ipv4Addr::LOCALHOST.into()],
            host_port: 0,
        }],
        stop_signal: crate::protocol::StopSignal::SIGTERM.as_str().into(),
        stop_timeout_secs: 5,
        network: String::new(),
        internal_network: true,
        log_max_size_mb: 1,
        log_max_files: 1,
        oom_score_adj: 0,
    })
}

pub(super) fn with_exit(problem: RuntimeError, monitor: &mut Machine) -> RuntimeError {
    match monitor.exited() {
        Ok(Some(status)) => error(format!("{problem}; the monitor ended with {status}")),
        _ => problem,
    }
}

/// What a retired monitor's boot leaves running on the host, stopped, as an observation would.
async fn retired(live: &mut Live) {
    let _ = live.forwarder.stop().await;
    if let Some(network) = &mut live.network {
        let _ = network.stop().await;
    }
    let _ = tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, live.monitor.finish_serial()).await;
}

/// Removes a host listener's socket file when the sleep it served is over.
struct Bound(PathBuf);
impl Drop for Bound {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// At start, before any guest runs: every memory file no journal names as its sleep's image, which
/// a removal, a crash mid-sleep or a wake that consumed it left behind. Guest memory holds
/// secrets, so nothing of it outlives the sandbox it was taken of.
pub(super) fn sweep(memory: &Path, entries: &BTreeMap<String, Entry>) -> Result<(), RuntimeError> {
    let mut kept = std::collections::HashSet::new();
    for entry in entries.values().filter(|entry| entry.memory.is_some()) {
        let sandbox = &entry.spec.agent.as_ref().ok_or_else(|| error("VM requires its authenticated agent"))?.sandbox;
        kept.insert(super::super::filesystems::sha256(sandbox.as_bytes()).map_err(error)?);
    }
    for path in fs::read_dir(memory).map_err(error)? {
        let path = path.map_err(error)?.path();
        if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| kept.contains(name)) {
            continue;
        }
        tracing::info!(memory = %path.display(), "VM memory no journal names is removed");
        // symlink_metadata never follows a link: a link is removed, never its target.
        let removed = if fs::symlink_metadata(&path).map_err(error)?.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        removed.map_err(error)?;
    }
    File::open(memory).and_then(|directory| directory.sync_all()).map_err(error)
}

pub(super) const ROOT_REFUSAL: &str = "VM memory sleep runs as an ordinary user; the root VM worker stops its guests";

/// A sandbox's boot directory, named for it, so a restore finds the monitor's sockets at the
/// paths its snapshot names. Whatever a previous boot of it left there is removed first.
pub(super) fn sandbox_directory(boots: &Path, lease: &Path) -> Result<tempfile::TempDir, RuntimeError> {
    let stem = lease.file_stem().and_then(|name| name.to_str()).ok_or_else(|| error("Invalid VM lease name"))?;
    let name = format!("s-{}", stem.get(..12).ok_or_else(|| error("Invalid VM lease name"))?);
    match fs::remove_dir_all(boots.join(&name)) {
        Ok(()) => (),
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => (),
        Err(problem) => return Err(error(problem)),
    }
    tempfile::Builder::new()
        .prefix(&name)
        .rand_bytes(0)
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(boots)
        .map_err(error)
}

#[cfg(test)]
mod sweep_tests {
    use super::*;

    #[test]
    fn only_a_sleeping_journals_memory_survives_a_start() {
        let root = tempfile::tempdir().unwrap();
        let memory = root.path().join("memory");
        directory(&memory).unwrap();
        let image = serde_json::json!({
            "class": {"cpu": "c", "kernel": "k", "monitor": "m"},
            "disk": {"device": 1, "inode": 2, "bytes": 3, "modified": [4, 5], "changed": [6, 7]},
            "nonce": "n", "memoryBytes": 268435456, "nanoCpus": null
        });
        let entry = |id: &str, image: Option<&serde_json::Value>| -> Entry {
            let spec = super::super::tests::spec(root.path(), "alpine:3.24");
            serde_json::from_value(serde_json::json!({"id": id, "spec": spec, "code": 0, "memory": image})).unwrap()
        };
        let name = |entry: &Entry| {
            super::super::super::filesystems::sha256(entry.spec.agent.as_ref().unwrap().sandbox.as_bytes()).unwrap()
        };
        let (asleep, awake) = (entry("vm-asleep", Some(&image)), entry("vm-awake", None));
        let entries = BTreeMap::from([(asleep.id.clone(), asleep.clone()), (awake.id.clone(), awake.clone())]);
        for directory in [name(&asleep), name(&awake), format!("{}.consumed", name(&asleep)), "removed".into()] {
            fs::create_dir(memory.join(&directory)).unwrap();
            fs::write(memory.join(&directory).join(MEMORY), b"guest secrets").unwrap();
        }
        fs::write(memory.join("stray"), b"guest secrets").unwrap();
        std::os::unix::fs::symlink(root.path(), memory.join("link")).unwrap();
        sweep(&memory, &entries).unwrap();
        let left: Vec<_> = fs::read_dir(&memory).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(left, [std::ffi::OsString::from(name(&asleep))], "Only the sleeping guest's image is kept");
        assert!(root.path().exists(), "A link is removed, never what it names");
    }
}
