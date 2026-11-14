// SPDX-License-Identifier: FSL-1.1-ALv2
//! Cold-boot VM lifecycle behind the existing runtime seam. This backend is deliberately
//! unregistered until node data APIs, root policy and acceptance are complete. Host-owned
//! pins, one lifetime lease per sandbox and owned children prevent a second writable guest.
//!
//! Parts (`instance/`):
//! - `configuration.rs`: generated SDK monitor configuration and drive identities.
//! - `guest.rs`: private guest arguments and bootstrap assembly.
//! - `launch.rs`: verified inputs, generated monitor configuration and guest bootstrap.
//! - `journal.rs`: private durable specifications and conservative restart adoption.
//! - `services.rs`: host-owned active and retained-disk file sessions.
//! - `maintenance.rs`: leased file-only guests for stopped disks.
//! - `agent.rs`: authenticated agent calls and one-shot compatibility exec.
//! - `logs.rs`: bounded guest entrypoint replay and live output.
//! - `runtime.rs`: runtime verbs, observations and bounded event polling.
//! - `diagnostics.rs`: bounded credential-redacted serial CI artifacts, in test builds only.
//! - `tests.rs`: actual KVM lifecycle acceptance through the runtime seam.
//! - `network_acceptance.rs`: real workload packet flow and immutable egress attacks.
mod agent;
mod configuration;
mod guest;
mod journal;
mod launch;
mod logs;
mod maintenance;
mod services;
pub use maintenance::FileGuest;
#[cfg(test)]
mod diagnostics;
#[cfg(test)]
mod network_acceptance;
mod runtime;
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

#[derive(Clone)]
pub struct Settings {
    pub artifact: Artifact,
    pub ingress_connections: u16,
    pub free_floor_bytes: u64,
    pub image_timeout: Duration,
}
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
}
// The runtime seam requires a numeric exit value even without a signed guest observation.
const UNKNOWN_EXIT_CODE: i64 = -1;

struct Live {
    monitor: Machine,
    forwarder: Forwarder,
    network: Option<super::network::Network>,
    _directory: tempfile::TempDir,
    serial: PathBuf,
    channel: PathBuf,
    nonce: String,
    shutdown_requested: bool,
    killing: bool,
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
impl VmRuntime {
    pub async fn new(state: &Path, settings: Settings) -> Result<Self, RuntimeError> {
        if rustix::process::geteuid().is_root() {
            return Err(RuntimeError::Unavailable(
                "Root VM jailer and firewall integration is not available in this checkpoint".into(),
            ));
        }
        if settings.ingress_connections == 0 {
            return Err(error("VM ingress concurrency must be positive"));
        }
        Self::kvm()?;
        let root = state.join("vm-runtime");
        directory(&root)?;
        let owner = lock(&root.join("owner.lock"))?;
        for name in ["leases", "boots", "cache", "downloads", "images", "entries"] {
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
        let entries = journal::load(&root)?;
        Ok(Self {
            state: Arc::new(State {
                root,
                settings,
                _owner: owner,
                bundle: tokio::sync::Mutex::new(None),
                entries: Mutex::new(entries),
                transitions: Mutex::new(HashMap::new()),
                live: Mutex::new(HashMap::new()),
            }),
        })
    }
    fn kvm() -> Result<(), RuntimeError> {
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
        Ok(self
            .state
            .root
            .join("leases")
            .join(format!("{}.lock", super::filesystems::sha256(sandbox.as_bytes()).map_err(error)?)))
    }
    fn names(&self) -> Vec<String> {
        self.state.entries.lock().unwrap().keys().cloned().collect()
    }
}
