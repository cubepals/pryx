// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private runtime state is never a workload view or an exported record. Journals contain
//! launch environments and are mode 0600 outside every sandbox's Landlock data grant.
use super::*;
use std::{
    fs,
    io::Read,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};

pub(super) fn error(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Other(error.to_string())
}
fn read_private(path: &Path) -> std::io::Result<Vec<u8>> {
    let file = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    let info = file.metadata()?;
    if !info.is_file() || info.uid() != rustix::process::geteuid().as_raw() || info.mode() & 0o077 != 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Unsafe private process journal"));
    }
    const MAX_BYTES: u64 = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}
impl ProcessRuntime {
    pub fn new(root: &Path, policy: ProcessConfig) -> Result<Self, RuntimeError> {
        let root = root.join("process-runtime");
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&root).map_err(error)?;
        let metadata = fs::symlink_metadata(&root).map_err(error)?;
        if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(error("Unsafe process runtime directory"));
        }
        let mut entries = BTreeMap::new();
        for path in fs::read_dir(&root).map_err(error)? {
            let path = path.map_err(error)?.path();
            if path.extension().is_some_and(|v| v == "json") && path.file_name().is_some_and(|v| v != "owners.json") {
                let entry: Entry = serde_json::from_slice(&read_private(&path).map_err(error)?).map_err(error)?;
                entries.insert(entry.spec.name.clone(), entry);
            }
        }
        let owners = match read_private(&root.join("owners.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(error)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(error(e)),
        };
        Ok(Self {
            state: Arc::new(State {
                root,
                entries: Mutex::new(entries),
                children: Mutex::new(HashMap::new()),
                owners: Mutex::new(owners),
            }),
            policy,
        })
    }
    pub(super) fn shared_uid(&self) -> bool {
        !rustix::process::geteuid().is_root()
    }
    pub(super) fn owner(&self, id: &str) -> Result<(u32, u32), RuntimeError> {
        if self.shared_uid() {
            return Ok((rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw()));
        }
        let mut owners = self.state.owners.lock().unwrap();
        if let Some(owner) = owners.get(id) {
            return Ok(*owner);
        }
        let uid = (self.policy.uid_first..self.policy.uid_first.saturating_add(self.policy.uid_count))
            .find(|uid| !owners.values().any(|(used, _)| used == uid) && !super::observe::uid_in_use(*uid))
            .ok_or_else(|| error("Process UID pool exhausted"))?;
        let mut next = owners.clone();
        next.insert(id.into(), (uid, uid));
        crate::durable::write_atomic(
            &self.state.root.join("owners.json"),
            &serde_json::to_vec(&next).map_err(error)?,
            0o600,
        )
        .map_err(error)?;
        *owners = next;
        Ok((uid, uid))
    }
    pub(super) fn release_owner(&self, id: &str) -> Result<(), RuntimeError> {
        if self
            .state
            .entries
            .lock()
            .unwrap()
            .values()
            .any(|entry| entry.spec.agent.as_ref().is_some_and(|boot| boot.sandbox == id))
        {
            return Err(error("Process compute still owns this data"));
        }
        self.retire_orphans(id)?;
        let mut owners = self.state.owners.lock().unwrap();
        if owners.contains_key(id) {
            let mut next = owners.clone();
            next.remove(id);
            crate::durable::write_atomic(
                &self.state.root.join("owners.json"),
                &serde_json::to_vec(&next).map_err(error)?,
                0o600,
            )
            .map_err(error)?;
            *owners = next;
        }
        Ok(())
    }
    pub(super) fn entry(&self, name: &str) -> Result<Entry, RuntimeError> {
        Self::entry_from(&self.state.entries.lock().unwrap(), name)
    }
    pub(super) fn entry_from(entries: &BTreeMap<String, Entry>, name: &str) -> Result<Entry, RuntimeError> {
        entries
            .values()
            .find(|entry| entry.spec.name == name || entry.id == name)
            .cloned()
            .ok_or_else(|| RuntimeError::NotFound("Process sandbox".into()))
    }
    pub(super) fn names(&self) -> Vec<String> {
        self.state.entries.lock().unwrap().keys().cloned().collect()
    }
    pub(super) fn save(&self, entry: Entry) -> Result<(), RuntimeError> {
        self.save_in(&mut self.state.entries.lock().unwrap(), entry)
    }
    pub(super) fn save_in(&self, entries: &mut BTreeMap<String, Entry>, entry: Entry) -> Result<(), RuntimeError> {
        self.persist(&entry)?;
        entries.insert(entry.spec.name.clone(), entry);
        Ok(())
    }
    pub(super) fn persist(&self, entry: &Entry) -> Result<(), RuntimeError> {
        crate::durable::write_atomic(
            &self.state.root.join(format!("{}.json", entry.id)),
            &serde_json::to_vec(&entry).map_err(error)?,
            0o600,
        )
        .map_err(error)?;
        Ok(())
    }
    pub(super) fn create_entry(&self, spec: &ContainerSpec) -> Result<String, RuntimeError> {
        if spec.nano_cpus.is_some() {
            return Err(error(crate::protocol::CPU_LIMIT_PROBLEM));
        }
        if self.entry(&spec.name).is_ok() {
            return Err(RuntimeError::Conflict("Process sandbox exists".into()));
        }
        if spec.agent.is_none() {
            return Err(error("Process tier requires the trusted agent"));
        }
        let entry = Entry {
            id: uuid::Uuid::new_v4().to_string(),
            spec: spec.clone(),
            pid: None,
            ticks: None,
            code: 0,
            started: None,
            finished: None,
            cleaned: true,
        };
        let id = entry.id.clone();
        self.save(entry)?;
        Ok(id)
    }
    pub(super) fn remove_entry(&self, name: &str) -> Result<(), RuntimeError> {
        let mut entries = self.state.entries.lock().unwrap();
        let entry = Self::entry_from(&entries, name)?;
        if self.state.children.lock().unwrap().contains_key(&entry.spec.name) {
            return Err(RuntimeError::Conflict("Process supervisor still awaits termination and reaping".into()));
        }
        fs::remove_file(self.state.root.join(format!("{}.json", entry.id))).map_err(error)?;
        entries.remove(&entry.spec.name);
        self.state.children.lock().unwrap().remove(&entry.spec.name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cpu_ceilings_are_refused_before_creation_or_journal_relaunch() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = ProcessRuntime::new(dir.path(), ProcessConfig::default()).unwrap();
        let mut spec = fixture_spec(dir.path(), 0);
        spec.nano_cpus = Some(250_000_000);
        assert!(
            matches!(runtime.create_entry(&spec), Err(RuntimeError::Other(s)) if s == crate::protocol::CPU_LIMIT_PROBLEM)
        );
        assert!(runtime.names().is_empty());
        spec.nano_cpus = None;
        runtime.create_entry(&spec).unwrap();
        let mut entry = runtime.entry(&spec.name).unwrap();
        entry.spec.nano_cpus = Some(250_000_000);
        runtime.save(entry).unwrap();
        assert!(
            matches!(runtime.launch(&spec.name).await, Err(RuntimeError::Other(s)) if s == crate::protocol::CPU_LIMIT_PROBLEM)
        );
        assert!(runtime.state.children.lock().unwrap().is_empty());
        let mut child = std::process::Command::new("sleep").arg("10").spawn().unwrap();
        let mut entry = runtime.entry(&spec.name).unwrap();
        entry.pid = Some(child.id());
        entry.ticks = super::super::observe::identity(child.id());
        runtime.save(entry).unwrap();
        let running = runtime.launch(&spec.name).await;
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(matches!(running, Err(RuntimeError::Other(s)) if s == crate::protocol::CPU_LIMIT_PROBLEM));
    }
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn owner_reservations_survive_restart_and_release() {
        let dir = tempfile::tempdir().unwrap();
        let config = ProcessConfig::default();
        let runtime = ProcessRuntime::new(dir.path(), config.clone()).unwrap();
        let first = runtime.owner("first").unwrap();
        let second = runtime.owner("second").unwrap();
        let resumed = ProcessRuntime::new(dir.path(), config.clone()).unwrap();
        assert_eq!(resumed.owner("first").unwrap(), first);
        assert_eq!(resumed.owner("second").unwrap(), second);
        if !runtime.shared_uid() {
            assert_ne!(first, second);
            resumed.release_owner("first").unwrap();
            let reloaded = ProcessRuntime::new(dir.path(), config).unwrap();
            assert_eq!(reloaded.owner("third").unwrap(), first);
        } else {
            assert_eq!(first.0, rustix::process::geteuid().as_raw());
        }
    }

    #[test]
    fn private_journals_reject_symlinks_and_public_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = ProcessRuntime::new(dir.path(), ProcessConfig::default()).unwrap();
        let source = dir.path().join("not-a-journal");
        fs::write(&source, b"{}").unwrap();
        let path = runtime.state.root.join("owners.json");
        symlink(&source, &path).unwrap();
        assert!(ProcessRuntime::new(dir.path(), ProcessConfig::default()).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(ProcessRuntime::new(dir.path(), ProcessConfig::default()).is_err());
    }

    fn fixture_spec(path: &Path, index: usize) -> ContainerSpec {
        serde_json::from_value(serde_json::json!({
            "name": format!("observe-{index}"), "image": "", "env": [], "labels": {},
            "agent": {"binary": "", "sandbox": format!("observe-{index}"), "epoch": 1,
                "secret_env": "", "domain": "pryx.test", "agent_port": pryx_agent::contract::AGENT_PORT,
                "spare_ports": [], "keep_alive": true},
            "user": "", "memory_bytes": 0, "cpu_shares": 0, "pids_limit": 0,
            "read_only_rootfs": true, "tmp_size_mb": 0, "data_dir": path,
            "mount_path": "/data", "ports": [], "stop_signal": "SIGTERM",
            "stop_timeout_secs": 0, "network": "", "internal_network": false,
            "log_max_size_mb": 0, "log_max_files": 0, "oom_score_adj": 0
        }))
        .unwrap()
    }

    #[test]
    fn observation_cannot_restore_removed_compute() {
        use std::sync::{
            Barrier,
            atomic::{AtomicBool, Ordering},
        };
        let dir = tempfile::tempdir().unwrap();
        let runtime = ProcessRuntime::new(dir.path(), ProcessConfig::default()).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(Barrier::new(2));
        let observer = {
            let runtime = runtime.clone();
            let done = done.clone();
            let ready = ready.clone();
            std::thread::spawn(move || {
                ready.wait();
                while !done.load(Ordering::Acquire) {
                    for name in runtime.names() {
                        runtime.observe_entry(&name).unwrap();
                    }
                    std::thread::yield_now();
                }
            })
        };
        ready.wait();
        let result = (|| {
            for index in 0..250 {
                let spec = fixture_spec(dir.path(), index);
                runtime.create_entry(&spec)?;
                let mut entry = runtime.entry(&spec.name)?;
                entry.cleaned = false;
                runtime.save(entry)?;
                std::thread::yield_now();
                runtime.remove_entry(&spec.name)?;
                runtime.release_owner(&spec.name)?;
            }
            Ok::<_, RuntimeError>(())
        })();
        done.store(true, Ordering::Release);
        observer.join().unwrap();
        result.unwrap();
        assert!(runtime.names().is_empty());
        assert!(ProcessRuntime::new(dir.path(), ProcessConfig::default()).unwrap().names().is_empty());
    }

    #[tokio::test]
    async fn removal_keeps_the_child_until_termination_is_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = ProcessRuntime::new(dir.path(), ProcessConfig::default()).unwrap();
        let spec = fixture_spec(dir.path(), 0);
        runtime.create_entry(&spec).unwrap();
        let child = std::process::Command::new("sleep").arg("10").spawn().unwrap();
        let pid = rustix::process::Pid::from_raw(child.id() as i32).unwrap();
        let mut entry = runtime.entry(&spec.name).unwrap();
        entry.pid = Some(child.id());
        entry.ticks = super::super::observe::identity(child.id());
        entry.cleaned = false;
        runtime.save(entry).unwrap();
        runtime.state.children.lock().unwrap().insert(spec.name.clone(), child);
        let retained = runtime.remove_entry(&spec.name).is_err();
        if retained {
            runtime.kill(&spec.name).await.unwrap();
            runtime.remove_entry(&spec.name).unwrap();
        } else {
            rustix::process::kill_process(pid, rustix::process::Signal::KILL).unwrap();
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::empty()).unwrap();
        }
        assert!(retained, "removal discarded a child that still needed termination and reaping");
        assert_eq!(
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG).unwrap_err(),
            rustix::io::Errno::CHILD
        );
    }

    #[test]
    fn a_missing_start_time_never_identifies_a_running_process() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = ProcessRuntime::new(dir.path(), ProcessConfig::default()).unwrap();
        let spec = fixture_spec(dir.path(), 0);
        runtime.create_entry(&spec).unwrap();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        assert!(child.wait().unwrap().success());
        assert!(super::super::observe::identity(pid).is_none());
        let mut entry = runtime.entry(&spec.name).unwrap();
        entry.pid = Some(pid);
        entry.ticks = None;
        runtime.save(entry).unwrap();
        assert!(!runtime.running(&spec.name).unwrap());
        assert_eq!(runtime.observe_entry(&spec.name).unwrap().unwrap().status, ContainerStatus::Exited);
    }
}
