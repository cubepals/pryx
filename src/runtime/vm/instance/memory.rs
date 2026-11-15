// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Memory sleep for an ordinary user's VM. Sleep has the guest flush its disk and hold a
//! connection, pauses the monitor, writes its full snapshot, and retires it; wake restores that
//! snapshot on this node only, into a host of the same class (CPU model, host kernel, monitor),
//! and refuses otherwise, leaving a cold boot from the disk as the wake. An image is used once,
//! and is discarded as soon as anything else touches its disk. The memory files hold guest
//! secrets, so they stay in this runtime's private state, owned and 0600 like the disk, and are
//! never part of a data copy, an export or a transfer. The jailer path does not sleep this way.
use super::*;
use crate::runtime::vm::disk;
use pryx_agent::{vm_monitor, vm_sleep};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt},
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
    pub(super) class: Class,
    pub(super) disk: disk::Stamp,
    /// The boot the guest still is, which its init's resume must name.
    pub(super) nonce: String,
    pub(super) memory_bytes: i64,
    pub(super) nano_cpus: Option<i64>,
}

/// The CPU fields of the first processor that decide which guest CPU state a host can take.
const CPU_FIELDS: [&str; 8] =
    ["vendor_id", "cpu family", "model", "stepping", "CPU implementer", "CPU architecture", "CPU variant", "CPU part"];
const STATE: &str = "state";
const MEMORY: &str = "memory";
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
        let mut cpu =
            CPU_FIELDS.iter().filter_map(|name| field(name).map(|value| format!("{name}={value}"))).collect::<Vec<_>>();
        let flags = field("flags").or_else(|| field("Features")).unwrap_or_default();
        cpu.push(format!("flags={}", &super::super::filesystems::sha256(flags.as_bytes()).map_err(error)?[..16]));
        Ok(Self {
            cpu: cpu.join(" "),
            kernel: fs::read_to_string("/proc/sys/kernel/osrelease").map_err(error)?.trim().to_owned(),
            monitor: super::super::filesystems::sha256(File::open(monitor).map_err(error)?).map_err(error)?,
        })
    }
}

fn path_text(path: &Path) -> Result<&str, RuntimeError> {
    path.to_str().ok_or_else(|| error("VM memory paths must be UTF-8"))
}

impl VmRuntime {
    /// The private directory one sandbox's memory image lives in.
    fn memory_directory(&self, entry: &Entry) -> Result<PathBuf, RuntimeError> {
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

    /// Monitor actions go through the bundle's SDK transport, to the monitor's own API socket.
    async fn monitor_action(
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
        let image = self.freeze(&bundle, &entry, &mut live).await?;
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
            // Paused or not, the guest goes on where it was; its hold's end finds no resume.
            let _ = Self::monitor_action(bundle, &api, json!({"action": "resume"}), GUEST_TIMEOUT).await;
            drop(hold);
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
            class,
            disk: disk::Stamp::current(&entry.spec.data_dir)?.ok_or_else(|| error("VM disk is missing after sleep"))?,
            nonce: live.nonce.clone(),
            memory_bytes: entry.spec.memory_bytes,
            nano_cpus: entry.spec.nano_cpus,
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
    async fn thaw(&self, bundle: &Bundle, entry: &Entry, image: &Image) -> Result<Live, RuntimeError> {
        let started = Instant::now();
        let class = Class::current(&bundle.root().join("bin/firecracker"))?;
        if class != image.class {
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
        let channel = directory.path().join("vsock");
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
        let restoring = Restoring { bundle, entry, image, api, state, memory, channel: channel.clone(), deadline };
        let ready = self.resumed(&restoring, &mut monitor, &listener, &mut stages).await;
        let forwarder = match ready {
            Ok(forwarder) => forwarder,
            Err(problem) => {
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
            nonce: image.nonce.clone(),
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
    ) -> Result<Forwarder, RuntimeError> {
        let Restoring { bundle, entry, image, api, state, memory, channel, deadline } = restoring;
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
        self.resume_guest(entry, image, listener, monitor, *deadline).await?;
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
        Ok(forwarder)
    }

    /// Gives the restored guest's init its signed resume: the host's time and fresh entropy.
    async fn resume_guest(
        &self,
        entry: &Entry,
        image: &Image,
        listener: &UnixListener,
        monitor: &Machine,
        deadline: Instant,
    ) -> Result<(), RuntimeError> {
        let boot = entry.spec.agent.as_ref().ok_or_else(|| error("VM agent bootstrap is missing"))?;
        let key = agent::key(&entry.spec)?;
        let pid = monitor.pid().and_then(|pid| i32::try_from(pid).ok());
        tokio::time::timeout_at(deadline, async {
            let mut socket = loop {
                let (socket, _) = listener.accept().await.map_err(error)?;
                if pid.is_some() && socket.peer_cred().map_err(error)?.pid() == pid {
                    break socket;
                }
            };
            let seed = ring::rand::generate::<[u8; vm_sleep::SEED_BYTES]>(&ring::rand::SystemRandom::new())
                .map_err(|_| error("Host entropy is unavailable"))?
                .expose();
            let now = crate::clock::now().unix_timestamp_nanos();
            let resume = vm_sleep::resume(&key, &image.nonce, &boot.sandbox, boot.epoch, now, &seed);
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
            Ok(())
        })
        .await
        .map_err(|_| RuntimeError::Timeout("VM guest init did not ask for its resume".into()))?
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
        if bundle.bootstrap().metadata.vm_sleep_version != vm_sleep::VERSION {
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
        let mut live = self.thaw(bundle, &entry, &image?).await?;
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

/// What one restore reads and where its monitor answers.
struct Restoring<'a> {
    bundle: &'a Bundle,
    entry: &'a Entry,
    image: &'a Image,
    api: PathBuf,
    state: PathBuf,
    memory: PathBuf,
    channel: PathBuf,
    deadline: Instant,
}

/// Removes a host listener's socket file when the sleep it served is over.
struct Bound(PathBuf);
impl Drop for Bound {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
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
