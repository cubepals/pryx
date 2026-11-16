// SPDX-License-Identifier: FSL-1.1-ALv2
//! VM lifecycle behind the existing runtime seam, selected by verified host prerequisites: cold
//! boots, and for an ordinary user memory sleep on a host of the same class. Host-owned pins,
//! one lifetime lease per sandbox and owned children prevent a second writable guest.
//!
//! Parts (`instance/`):
//! - `configuration.rs`: generated SDK monitor configuration and drive identities.
//! - `guest.rs`: private guest arguments and bootstrap assembly.
//! - `launch.rs`: verified inputs, generated monitor configuration and guest bootstrap.
//! - `journal.rs`: private durable specifications and conservative restart adoption.
//! - `services.rs`: host-owned active and retained-disk file sessions.
//! - `maintenance.rs`: leased file-only guests for stopped disks.
//! - `memory.rs`: memory sleep and its restore on a host of the same class, and its start test.
//! - `memory_acceptance.rs`: real KVM sleep and wake with a running process, and its fallbacks.
//! - `agent.rs`: authenticated agent calls and one-shot compatibility exec.
//! - `monitor.rs`: host-selected ordinary/jailer launch paths and retired-disk recovery.
//! - `monitor_ids.rs`: root's own monitor uid and gid for each VM, from the host's range.
//! - `network.rs`: host-selected ordinary packet or leased root TAP connections.
//! - `logs.rs`: bounded guest entrypoint replay and live output.
//! - `runtime.rs`: runtime verbs, observations and bounded event polling.
//! - `diagnostics.rs`: bounded credential-redacted serial CI artifacts, in test builds only.
//! - `tests.rs`: actual KVM lifecycle acceptance through the runtime seam.
//! - `network_acceptance.rs`: real workload packet flow and immutable egress attacks.
//! - `root_lifecycle.rs`: real root runtime, file maintenance and daemon-loss recovery.
//! - `root_acceptance.rs`: real jailed boots and original-disk ownership recovery.
//! - `shutdown_acceptance.rs`: real monitor exit status after authenticated guest shutdown.
mod agent;
mod configuration;
#[cfg(test)]
mod diagnostics;
mod guest;
mod journal;
mod launch;
mod logs;
mod maintenance;
mod memory;
#[cfg(test)]
mod memory_acceptance;
mod monitor;
mod monitor_ids;
pub use monitor_ids::MonitorIds;
mod network;
#[cfg(test)]
mod network_acceptance;
#[cfg(test)]
mod root_acceptance;
#[cfg(test)]
mod root_lifecycle;
mod runtime;
mod services;
#[cfg(test)]
mod shutdown_acceptance;
#[cfg(test)]
mod tests;
use super::{
    bundle::{Artifact, Bundle},
    error,
    forward::Forwarder,
    image::Prepared,
    machine::Machine,
};
use crate::runtime::{ContainerSpec, RuntimeError};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::sync::OwnedMutexGuard;

pub struct FileGuest {
    inner: Option<Inner>,
}
struct Inner {
    entry: Entry,
    live: Live,
    bundle: Arc<Bundle>,
    _transition: OwnedMutexGuard<()>,
}

#[derive(Clone)]
pub struct Settings {
    pub artifact: Artifact,
    pub ingress_connections: u16,
    pub free_floor_bytes: u64,
    pub image_timeout: Duration,
    /// Root's monitor IDs, from which each VM takes its own pair; `None` for an ordinary user.
    pub monitor_owner: Option<MonitorIds>,
}
// The runtime owns this bounded host ingress budget; workloads cannot increase it.
pub(super) const INGRESS_CONNECTIONS: u16 = 64;
#[derive(Clone)]
pub struct VmRuntime {
    state: Arc<State>,
}
struct State {
    root: PathBuf,
    settings: Settings,
    _owner: File,
    bundle: tokio::sync::Mutex<Option<Arc<Bundle>>>,
    entries: Mutex<BTreeMap<String, Entry>>,
    transitions: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    live: Mutex<HashMap<String, Arc<tokio::sync::Mutex<Live>>>>,
    /// Images this runtime prepared, by manifest digest, with their layer files' identities.
    prepared: Mutex<HashMap<String, (Prepared, Vec<launch::Identity>)>>,
    /// The memory-sleep restore test's outcome, once it has run.
    memory_sleep: Mutex<Option<Result<(), String>>>,
    /// Root's pair of monitor IDs for each VM (`monitor_ids.rs`).
    monitor_ids: Option<Mutex<monitor_ids::Ledger>>,
    /// Manifests whose first launch is between preparation and monitor, with how many.
    launching: Mutex<HashMap<String, usize>>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Entry {
    id: String,
    spec: ContainerSpec,
    #[serde(default, with = "journal::timestamp")]
    started: Option<time::OffsetDateTime>,
    #[serde(default, with = "journal::timestamp")]
    finished: Option<time::OffsetDateTime>,
    code: i64,
    #[serde(default)]
    agent_state: Option<pryx_agent::state::Snapshot>,
    /// The manifest digest of the first start, which every later start prepares exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image_digest: Option<String>,
    /// The disk as the last signed clean shutdown left it; a start of that same file needs no check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clean_disk: Option<super::disk::Stamp>,
    /// Set while the guest sleeps with its memory; its files are in this runtime's private state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    memory: Option<memory::Image>,
}
// The runtime seam requires a numeric exit value even without a signed guest observation.
const UNKNOWN_EXIT_CODE: i64 = -1;
/// The tier this backend serves, as every VM frontend names it.
pub const ISOLATION: &str = "vm";

struct Live {
    monitor: Machine,
    jail: Option<super::jail::Jail>,
    forwarder: Forwarder,
    network: Option<network::Network>,
    _directory: tempfile::TempDir,
    serial: PathBuf,
    channel: PathBuf,
    nonce: String,
    shutdown_requested: bool,
    killing: bool,
    /// How long each stage of this boot took.
    #[cfg_attr(not(test), allow(dead_code))]
    stages: launch::Stages,
    #[cfg(test)]
    diagnostic_secrets: Vec<String>,
}
#[cfg(test)]
impl Drop for Live {
    fn drop(&mut self) {
        diagnostics::retain(&self.serial, &self.diagnostic_secrets);
    }
}

pub(super) fn directory(path: &Path) -> Result<(), RuntimeError> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(problem) if problem.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(problem) => return Err(error(problem)),
    }
    super::bundle::ancestors(path).map_err(error)?;
    let info = fs::symlink_metadata(path).map_err(error)?;
    if info.uid() != rustix::process::geteuid().as_raw() || info.mode() & 0o077 != 0 {
        return Err(error("VM runtime state must be a private owned directory"));
    }
    Ok(())
}
#[cfg(test)]
fn boot_directory(root: &Path) -> Result<tempfile::TempDir, RuntimeError> {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .prefix("boot-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(root)
        .map_err(error)
}

fn lock(path: &Path) -> Result<File, RuntimeError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(error)?;
    let info = file.metadata().map_err(error)?;
    if !info.is_file()
        || info.uid() != rustix::process::geteuid().as_raw()
        || info.mode() & 0o077 != 0
        || info.nlink() != 1
    {
        return Err(error("Unsafe VM ownership lease"));
    }
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).map_err(|problem| {
        if problem == rustix::io::Errno::WOULDBLOCK {
            RuntimeError::Unavailable("VM lifetime lease is still held".into())
        } else {
            error(problem)
        }
    })?;
    Ok(file)
}
/// A sandbox's lifetime lease, and its packet helper's beside it.
fn lease_names(sandbox: &str) -> Result<[String; 2], RuntimeError> {
    let digest = super::filesystems::sha256(sandbox.as_bytes()).map_err(error)?;
    Ok([format!("{digest}.lock"), format!("{digest}.network.lock")])
}

/// Unlinks one lease while holding it, so nothing still running keeps a lock on the name; one
/// that is held is left as it is.
fn remove_lease(path: &Path) -> Result<(), RuntimeError> {
    match fs::symlink_metadata(path) {
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(problem) => return Err(error(problem)),
        Ok(_) => (),
    }
    match lock(path) {
        Ok(_held) => fs::remove_file(path).map_err(error),
        Err(RuntimeError::Unavailable(_)) => {
            tracing::warn!(lease = %path.display(), "VM lease is still held; it is kept");
            Ok(())
        }
        Err(problem) => Err(problem),
    }
}

/// At start, after every predecessor's lease is free: the leases of sandboxes no journal names,
/// which a removal before leases were removed with their VM left behind.
fn sweep_leases(leases: &Path, entries: &BTreeMap<String, Entry>) -> Result<(), RuntimeError> {
    let mut known = std::collections::HashSet::new();
    for entry in entries.values() {
        if let Some(agent) = &entry.spec.agent {
            known.extend(lease_names(&agent.sandbox)?);
        }
    }
    for path in fs::read_dir(leases).map_err(error)? {
        let path = path.map_err(error)?.path();
        if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| !known.contains(name)) {
            remove_lease(&path)?;
        }
    }
    Ok(())
}

impl VmRuntime {
    pub async fn new(state: &Path, settings: Settings) -> Result<Self, RuntimeError> {
        monitor::owner(settings.monitor_owner)?;
        if settings.ingress_connections == 0 {
            return Err(error("VM ingress concurrency must be positive"));
        }
        Self::kvm()?;
        let root = state.join("vm-runtime");
        directory(&root)?;
        let owner = lock(&root.join("owner.lock"))?;
        for name in ["leases", "boots", "cache", "downloads", "images", "entries", "jails", "memory"] {
            directory(&root.join(name))?;
        }
        // A predecessor's owned monitor must actually exit before any disk is opened.
        // Parent-bound ordinary launches cannot survive daemon death. Private journals
        // restore stopped specifications; disk safety still needs independent verification.
        let deadline = tokio::time::Instant::now() + crate::runtime::ENGINE_PROBE_TIMEOUT;
        for entry in fs::read_dir(root.join("leases")).map_err(error)? {
            let path = entry.map_err(error)?.path();
            loop {
                match lock(&path) {
                    Ok(lease) => {
                        drop(lease);
                        break;
                    }
                    Err(RuntimeError::Unavailable(_)) if tokio::time::Instant::now() < deadline => {
                        tokio::time::sleep(Duration::from_millis(10)).await
                    }
                    Err(problem) => return Err(problem),
                }
            }
        }
        if settings.monitor_owner.is_some() {
            super::jail::Jail::recover_all(&root.join("jails"))?;
        }
        let entries = journal::load(&root)?;
        sweep_leases(&root.join("leases"), &entries)?;
        let monitor_ids = settings
            .monitor_owner
            .map(|ids| monitor_ids::Ledger::load(&root, ids, |vm| entries.contains_key(vm)).map(Mutex::new))
            .transpose()?;
        Ok(Self {
            state: Arc::new(State {
                root,
                settings,
                _owner: owner,
                bundle: tokio::sync::Mutex::new(None),
                entries: Mutex::new(entries),
                transitions: Mutex::new(HashMap::new()),
                live: Mutex::new(HashMap::new()),
                prepared: Mutex::new(HashMap::new()),
                memory_sleep: Mutex::new(None),
                monitor_ids,
                launching: Mutex::new(HashMap::new()),
            }),
        })
    }
    pub(crate) fn kvm() -> Result<(), RuntimeError> {
        #[cfg(target_arch = "x86_64")]
        {
            kvm_ioctls::Kvm::new()
                .and_then(|kvm| kvm.create_vm())
                .map(|_| ())
                .map_err(|problem| RuntimeError::Unavailable(format!("VM requires working /dev/kvm: {problem}")))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Err(RuntimeError::Unavailable("The pinned VM distribution supports x86_64 only".into()))
        }
    }
    async fn bundle(&self) -> Result<Arc<Bundle>, RuntimeError> {
        let mut available = self.state.bundle.lock().await;
        if let Some(bundle) = available.as_ref() {
            return Ok(bundle.clone());
        }
        let bundle = Bundle::acquire(
            &self.state.root.join("cache"),
            &self.state.root.join("downloads"),
            &self.state.settings.artifact,
        )
        .await?;
        let bundle = Arc::new(bundle);
        *available = Some(bundle.clone());
        Ok(bundle)
    }
    fn entry(&self, name: &str) -> Result<Entry, RuntimeError> {
        self.state
            .entries
            .lock()
            .unwrap()
            .values()
            .find(|entry| entry.id == name || entry.spec.name == name)
            .cloned()
            .ok_or_else(|| RuntimeError::NotFound("VM sandbox".into()))
    }
    fn transition(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.state
            .transitions
            .lock()
            .unwrap()
            .entry(id.into())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
    fn lease(&self, entry: &Entry) -> Result<PathBuf, RuntimeError> {
        let sandbox = &entry.spec.agent.as_ref().ok_or_else(|| error("VM requires its authenticated agent"))?.sandbox;
        let [lease, _] = lease_names(sandbox)?;
        Ok(self.state.root.join("leases").join(lease))
    }
    /// A removed VM's leases go with it.
    fn remove_leases(&self, entry: &Entry) -> Result<(), RuntimeError> {
        let sandbox = &entry.spec.agent.as_ref().ok_or_else(|| error("VM requires its authenticated agent"))?.sandbox;
        for name in lease_names(sandbox)? {
            remove_lease(&self.state.root.join("leases").join(name))?;
        }
        Ok(())
    }
    fn names(&self) -> Vec<String> {
        self.state.entries.lock().unwrap().keys().cloned().collect()
    }
}

#[cfg(test)]
mod lease_tests {
    use super::*;

    #[test]
    fn leases_go_with_their_vm_and_orphans_at_start_but_never_while_held() {
        let root = tempfile::tempdir().unwrap();
        let leases = root.path().join("leases");
        directory(&leases).unwrap();
        let spec = tests::spec(root.path(), "alpine:3.24");
        let entry: Entry =
            serde_json::from_value(serde_json::json!({"id": "vm-kept", "spec": spec, "code": 0})).unwrap();
        let kept = lease_names(&spec.agent.as_ref().unwrap().sandbox).unwrap();
        let orphans = lease_names(&uuid::Uuid::new_v4().simple().to_string()).unwrap();
        for name in kept.iter().chain(&orphans) {
            drop(lock(&leases.join(name)).unwrap());
        }
        let held = lock(&leases.join(&orphans[0])).unwrap();
        sweep_leases(&leases, &BTreeMap::from([(entry.id.clone(), entry)])).unwrap();
        let present = |name: &String| leases.join(name).exists();
        assert!(kept.iter().all(present), "A journal's leases were swept");
        assert!(present(&orphans[0]) && !present(&orphans[1]), "Only the free orphan goes");
        drop(held);
        sweep_leases(&leases, &BTreeMap::new()).unwrap();
        assert_eq!(fs::read_dir(&leases).unwrap().count(), 0);
        remove_lease(&leases.join(&kept[0])).unwrap();
    }
}
