// SPDX-License-Identifier: FSL-1.1-ALv2
//! Native inputs come from the protected installation, never workload fields. Enabling them
//! requires fresh signed scope observations of managed peers and a persistent launch guard.
//! Their private directories are made per owner UID, and leave with the sandbox that held them:
//! a released UID's whole directory, or a shared UID's scratch for that sandbox alone.
use super::{
    Entry, ProcessRuntime, RuntimeError,
    journal::{error, legacy_boot, note_unscoped},
    observe,
};
use pryx_agent::process_tool_policy::{Artifact, Policy, VERSION};
use std::{
    fs,
    io::Read,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Default)]
struct Readiness {
    supported: bool,
    scoped: bool,
    artifact: Option<Artifact>,
    unavailable: Option<String>,
    browser_on: bool,
    desktop_on: Option<bool>,
    desktop_unavailable: Option<String>,
}

fn descriptor(binary: &Path) -> Result<Artifact, RuntimeError> {
    let path = binary.parent().ok_or_else(|| error("Missing agent directory"))?.join("process-tools.json");
    let file = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).map_err(
        |_| error("This installation has no protected process tool artifact; upgrade to a release containing it"),
    )?;
    let info = file.metadata().map_err(error)?;
    let own = rustix::process::geteuid().as_raw();
    if !info.is_file()
        || (info.uid() != own && info.uid() != rustix::process::Uid::ROOT.as_raw())
        || info.mode() & 0o022 != 0
    {
        return Err(error("Unsafe process tool artifact descriptor"));
    }
    const MAX_DESCRIPTOR_BYTES: u64 = 4096;
    let mut bytes = Vec::new();
    file.take(MAX_DESCRIPTOR_BYTES + 1).read_to_end(&mut bytes).map_err(error)?;
    if bytes.len() as u64 > MAX_DESCRIPTOR_BYTES {
        return Err(error("Oversized process tool artifact descriptor"));
    }
    serde_json::from_slice(&bytes).map_err(error)
}
fn directory(path: &Path, owner: (u32, u32)) -> Result<PathBuf, RuntimeError> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(problem) if problem.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(problem) => return Err(error(problem)),
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(path)
        .map_err(error)?;
    let info = file.metadata().map_err(error)?;
    let own = rustix::process::geteuid().as_raw();
    if !info.is_dir() || (info.uid() != own && info.uid() != owner.0) || info.mode() & 0o077 != 0 {
        return Err(error("Unsafe private process tool directory"));
    }
    rustix::fs::fchown(
        &file,
        Some(rustix::process::Uid::from_raw(owner.0)),
        Some(rustix::process::Gid::from_raw(owner.1)),
    )
    .map_err(error)?;
    Ok(path.to_path_buf())
}
/// A name under the tool root that no owner reaches: what is renamed to it is removed whole, at
/// once or by the runtime's next start.
const RETIRED: &str = ".retired-";

fn checked_base(base: &Path) -> Result<(), RuntimeError> {
    let info = fs::symlink_metadata(base).map_err(error)?;
    if !info.is_dir() || info.uid() != rustix::process::geteuid().as_raw() || info.mode() & 0o022 != 0 {
        return Err(error("Unsafe process tool root"));
    }
    Ok(())
}
/// One rename takes `name` out of every owner's path, so nothing it held can be seen again.
fn retire(base: &Path, name: &str) -> Result<Option<PathBuf>, RuntimeError> {
    let retired = base.join(format!("{RETIRED}{}", uuid::Uuid::new_v4()));
    let from = base.join(name);
    match rustix::fs::renameat_with(
        rustix::fs::CWD,
        &from,
        rustix::fs::CWD,
        &retired,
        rustix::fs::RenameFlags::NOREPLACE,
    ) {
        Ok(()) => Ok(Some(retired)),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(problem) => Err(error(std::io::Error::from(problem))),
    }
}
fn remove_retired(retired: Option<PathBuf>) {
    if let Some(retired) = retired
        && let Err(problem) = crate::tree::remove(&retired)
    {
        tracing::warn!(error = %problem, "a released process tool directory is left for the next start to remove");
    }
}
fn scratch_prefix(sandbox: &str) -> Result<String, RuntimeError> {
    // An id has no '.', so this prefix names one sandbox's scratch and no other's.
    Ok(format!("scratch-{}.", crate::ids::WorkloadId::parse(sandbox).map_err(error)?))
}
impl ProcessRuntime {
    fn tools_base(&self) -> Result<PathBuf, RuntimeError> {
        Ok(self.state.root.parent().ok_or_else(|| error("Missing process state parent"))?.join("process-tools"))
    }
    fn tool_paths(&self, owner: (u32, u32), sandbox: &str) -> Result<(PathBuf, PathBuf, PathBuf), RuntimeError> {
        let base = self.tools_base()?;
        if !base.exists() {
            directory(&base, (rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw()))?;
        }
        checked_base(&base)?;
        fs::set_permissions(&base, fs::Permissions::from_mode(0o711)).map_err(error)?;
        let uid = directory(&base.join(owner.0.to_string()), owner)?;
        Ok((
            directory(&uid.join("cache"), owner)?,
            directory(&uid.join("downloads"), owner)?,
            directory(&uid.join(format!("{}{}", scratch_prefix(sandbox)?, uuid::Uuid::new_v4())), owner)?,
        ))
    }
    pub(super) fn remove_retired(&self, retired: impl IntoIterator<Item = PathBuf>) {
        retired.into_iter().for_each(|retired| remove_retired(Some(retired)));
    }
    /// Takes the tool directory of a UID nobody holds out of reach: whatever an earlier holder or
    /// a crash left there, the next holder starts empty. Returns what to remove.
    pub(super) fn retire_uid_tools(&self, uid: u32) -> Result<Option<PathBuf>, RuntimeError> {
        let base = self.tools_base()?;
        if fs::symlink_metadata(&base).is_err_and(|problem| problem.kind() == std::io::ErrorKind::NotFound) {
            return Ok(None);
        }
        checked_base(&base)?;
        retire(&base, &uid.to_string())
    }
    /// A shared UID's directory serves every sandbox at once; only this one's scratch leaves.
    pub(super) fn retire_shared_scratch(&self, sandbox: &str) -> Result<Vec<PathBuf>, RuntimeError> {
        let base = self.tools_base()?;
        let shared = base.join(rustix::process::geteuid().as_raw().to_string());
        match fs::symlink_metadata(&shared) {
            Ok(info) if info.is_dir() => checked_base(&base)?,
            Ok(_) => return Err(error("Unsafe private process tool directory")),
            Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(problem) => return Err(error(problem)),
        }
        let entries = fs::read_dir(&shared).map_err(error)?;
        let prefix = scratch_prefix(sandbox)?;
        let mut retired = Vec::new();
        for entry in entries {
            let name = entry.map_err(error)?.file_name();
            if let Some(name) = name.to_str().filter(|name| name.starts_with(&prefix)) {
                retired.extend(retire(&base, &format!("{}/{name}", rustix::process::geteuid().as_raw()))?);
            }
        }
        Ok(retired)
    }
    /// What a crash left: retired trees, and in the UID pool a directory whose UID nobody holds.
    /// A shared UID's scratch stays, since supervisors adopted across a restart may still use it.
    pub(super) fn sweep_tools(&self) -> Result<(), RuntimeError> {
        let base = self.tools_base()?;
        let entries = match fs::read_dir(&base) {
            Ok(entries) => entries,
            Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(problem) => return Err(error(problem)),
        };
        checked_base(&base)?;
        let held: Vec<String> = self.state.owners.lock().unwrap().values().map(|(uid, _)| uid.to_string()).collect();
        for entry in entries {
            let name = entry.map_err(error)?.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with(RETIRED) {
                remove_retired(Some(base.join(name)));
            } else if !self.shared_uid() && name.parse::<u32>().is_ok() && !held.iter().any(|uid| uid == name) {
                remove_retired(retire(&base, name)?);
            }
        }
        Ok(())
    }
    async fn native_readiness(&self, binary: &Path) -> Result<Readiness, RuntimeError> {
        let metadata = crate::runtime::agent_metadata::query(binary).await;
        if metadata.as_ref().is_none_or(|value| value.process_tools_version < VERSION) {
            return Ok(Readiness::default());
        }
        let artifact = descriptor(binary);
        let glibc =
            crate::capability::process_tools::prerequisite(std::env::consts::ARCH, &crate::capability::c_library());
        let mut unavailable = (glibc.state != crate::capability::State::On).then_some(glibc.evidence);
        let scoped = pryx_agent::process_capabilities::private_ipc_probe().supported();
        if !scoped {
            unavailable.get_or_insert_with(|| {
                "Native process tools require enforceable Landlock ABI 6 abstract-socket isolation".into()
            });
        }
        if unavailable.is_none()
            && let Err(problem) = &artifact
        {
            unavailable = Some(problem.to_string());
        }
        if unavailable.is_none() && legacy_boot(&self.state.root)? {
            unavailable=Some("Unverified process history on this boot; native tools need a host restart to retire possible legacy orphans".into());
        }
        if unavailable.is_none() {
            for name in self.names() {
                let peer = self.entry(&name)?;
                if observe::matching_pid(peer.pid, peer.ticks).is_some() && !self.agent_scope_proven(&peer).await {
                    unavailable=Some("Native process tools are off while a live legacy or unverified supervisor shares this host; retire it or restart it with a scoped agent".into());
                    break;
                }
            }
        }
        let browser_on = unavailable.is_none()
            && metadata
                .as_ref()
                .is_some_and(|value| value.process_browser_version >= pryx_agent::process_tool_policy::BROWSER_VERSION)
            && crate::capability::child("probe-user-namespace").state == crate::capability::State::On;
        let desktop_supported = metadata
            .as_ref()
            .is_some_and(|value| value.process_desktop_version >= pryx_agent::process_tool_policy::DESKTOP_VERSION);
        let desktop_glibc = crate::capability::process_tools::desktop_prerequisite(
            std::env::consts::ARCH,
            &crate::capability::c_library(),
        );
        let mut desktop_unavailable = unavailable.clone();
        if desktop_glibc.state != crate::capability::State::On {
            desktop_unavailable.get_or_insert(desktop_glibc.evidence);
        }
        if !browser_on {
            desktop_unavailable.get_or_insert_with(|| {
                "Native desktop requires usable unprivileged user namespaces and a verified scoped agent".into()
            });
        }
        let desktop_on = desktop_supported.then_some(desktop_unavailable.is_none());
        if !desktop_supported {
            desktop_unavailable = None;
        }
        Ok(Readiness {
            supported: true,
            scoped,
            artifact: artifact.ok(),
            unavailable,
            browser_on,
            desktop_on,
            desktop_unavailable,
        })
    }
    pub(super) async fn browser_requirement(&self, binary: &Path) -> Result<(), RuntimeError> {
        let ready = self.native_readiness(binary).await?;
        if !ready.supported {
            return Err(error("Native process browser needs a compatible scoped agent; upgrade its executable"));
        }
        if let Some(reason) = ready.unavailable {
            return Err(error(reason));
        }
        if !ready.browser_on {
            return Err(error("Process browser needs a supported agent and usable unprivileged user namespaces"));
        }
        Ok(())
    }
    pub(super) async fn desktop_requirement(&self, binary: &Path) -> Result<(), RuntimeError> {
        let ready = self.native_readiness(binary).await?;
        if ready.desktop_on != Some(true) {
            return Err(error(ready.desktop_unavailable.unwrap_or_else(|| {
                "Native desktop needs an agent and installation providing the verified desktop bundle".into()
            })));
        }
        Ok(())
    }
    pub(super) async fn native_policy(&self, entry: &Entry) -> Result<(Option<Policy>, bool), RuntimeError> {
        let required = self.scope_required()?;
        let agent = entry.spec.agent.as_ref().ok_or_else(|| error("Missing process supervisor"))?;
        let ready = self.native_readiness(&agent.binary).await?;
        if entry.spec.require_browser_sandbox == Some(true) && !ready.browser_on {
            return Err(error(ready.unavailable.unwrap_or_else(|| {
                "browser-sandbox needs a compatible process agent and usable unprivileged user namespaces".into()
            })));
        }
        if !ready.supported {
            if required {
                return Err(error(
                    "This host requires scoped process agents; upgrade the legacy supervisor before launching compute",
                ));
            }
            note_unscoped(&self.state.root)?;
            return Ok((None, false));
        }
        if !ready.scoped {
            note_unscoped(&self.state.root)?;
        }
        if ready.unavailable.is_none() && !required {
            crate::durable::write_atomic(&self.state.root.join("native-scope-required"), b"1\n", 0o600)
                .map_err(error)?;
        }
        let (cache, downloads, scratch) = self.tool_paths(self.owner(&agent.sandbox)?, &agent.sandbox)?;
        Ok((
            Some(Policy {
                cache,
                downloads,
                scratch,
                artifact: ready.artifact,
                unavailable: ready.unavailable,
                browser_on: ready.browser_on,
                desktop_on: ready.desktop_on,
                desktop_unavailable: ready.desktop_unavailable,
            }),
            required || ready.scoped,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn placement_probes_do_not_mark_unscoped_history_or_acquire_tools() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ProcessRuntime::new(root.path(), crate::config::ProcessConfig::default()).unwrap();
        let binary = root.path().join("agent");
        for answer in ["{}".to_string(), serde_json::to_string(&pryx_agent::bootstrap_capabilities::probe()).unwrap()] {
            fs::write(&binary, format!("#!/bin/sh\nprintf '%s' '{answer}'\n")).unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            assert!(runtime.browser_requirement(&binary).await.is_err());
            assert!(!runtime.state.root.join("native-unscoped-boot").exists());
            assert!(!runtime.state.root.join("native-scope-required").exists());
            assert!(!root.path().join("process-tools").exists());
        }
    }

    /// A tenant's tool files: a sealed verified cache entry, a download, and scratch with a link out.
    fn tenant_files(cache: &Path, downloads: &Path, scratch: &Path, outside: &Path) {
        let sealed = cache.join("digest");
        fs::create_dir(&sealed).unwrap();
        fs::write(sealed.join("tool"), b"verified").unwrap();
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o555)).unwrap();
        fs::write(downloads.join("partial"), b"partial").unwrap();
        fs::write(scratch.join("profile"), b"first tenant").unwrap();
        std::os::unix::fs::symlink(outside, scratch.join("escape")).unwrap();
    }
    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<_> =
            fs::read_dir(directory).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }

    #[test]
    fn a_deleted_sandbox_takes_its_tool_files_and_a_reused_uid_starts_empty() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("kept"), b"kept").unwrap();
        let runtime = ProcessRuntime::new(root.path(), crate::config::ProcessConfig::default()).unwrap();
        let first = runtime.owner("first").unwrap();
        let (cache, downloads, scratch) = runtime.tool_paths(first, "first").unwrap();
        tenant_files(&cache, &downloads, &scratch, outside.path());
        let neighbour = runtime.owner("neighbour").unwrap();
        let (_, _, live) = runtime.tool_paths(neighbour, "neighbour").unwrap();
        runtime.release_owner("first").unwrap();
        let base = root.path().join("process-tools");
        assert!(!scratch.exists(), "the deleted sandbox's scratch is gone");
        assert!(live.exists(), "a neighbour's scratch stays");
        assert_eq!(fs::read(outside.path().join("kept")).unwrap(), b"kept", "a link's target is never touched");
        assert!(names(&base).iter().all(|name| !name.starts_with(RETIRED)), "{:?}", names(&base));
        if runtime.shared_uid() {
            assert!(cache.join("digest/tool").exists(), "the shared UID's verified cache still serves its others");
            return crate::tree::unseal(&base).unwrap();
        }
        assert!(!base.join(first.0.to_string()).exists(), "the released UID's whole directory is gone");
        assert_eq!(runtime.owner("second").unwrap(), first, "the pool hands the UID out again");
        let (cache, downloads, _) = runtime.tool_paths(first, "second").unwrap();
        assert!(names(&cache).is_empty() && names(&downloads).is_empty());
        let held = names(&base.join(first.0.to_string()));
        assert!(
            held.iter()
                .all(|name| ["cache", "downloads"].contains(&name.as_str()) || name.starts_with("scratch-second.")),
            "{held:?}"
        );
    }

    #[test]
    fn a_crash_between_release_and_removal_never_reaches_the_next_holder() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let config = crate::config::ProcessConfig::default();
        let runtime = ProcessRuntime::new(root.path(), config.clone()).unwrap();
        let first = runtime.owner("first").unwrap();
        let (cache, downloads, scratch) = runtime.tool_paths(first, "first").unwrap();
        tenant_files(&cache, &downloads, &scratch, outside.path());
        // Renamed out of reach, then a crash before removal.
        let retired = runtime.retire_uid_tools(first.0).unwrap().unwrap();
        let base = root.path().join("process-tools");
        let free = (first.0 + 1, first.0 + 1);
        let mut held = None;
        if !runtime.shared_uid() {
            // Released by a node from before this removal, or a crash after the release itself:
            // nobody holds the UID, yet its directory is there.
            for uid in [free.0, free.0 + 1] {
                fs::create_dir_all(base.join(format!("{uid}/cache"))).unwrap();
                fs::write(base.join(format!("{uid}/cache/leftover")), b"earlier tenant").unwrap();
            }
            assert_eq!(runtime.owner("second").unwrap(), free);
            assert!(!base.join(format!("{}/cache/leftover", free.0)).exists(), "the next holder starts empty");
            held = Some(runtime.tool_paths(free, "second").unwrap().2);
        }
        let restarted = ProcessRuntime::new(root.path(), config).unwrap();
        assert!(!retired.exists(), "the next start removes what a crash left retired");
        assert!(names(&base).iter().all(|name| !name.starts_with(RETIRED)), "{:?}", names(&base));
        if let Some(held) = held {
            assert!(!restarted.shared_uid());
            assert!(!base.join((free.0 + 1).to_string()).exists(), "and a directory whose UID nobody holds");
            assert!(held.exists(), "a held UID's directory stays");
        }
    }

    #[test]
    fn descriptors_refuse_links_writers_and_oversized_inputs() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("agent");
        let path = root.path().join("process-tools.json");
        let artifact = serde_json::json!({"url":"https://example.test/tools.tar.gz","sha256":"a".repeat(64),"manifest_sha256":"b".repeat(64),"bytes":1,"unpacked_bytes":1,"entries":1});
        fs::write(&path, artifact.to_string()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(descriptor(&binary).is_ok());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(descriptor(&binary).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, vec![b' '; 4097]).unwrap();
        assert!(descriptor(&binary).err().unwrap().to_string().contains("Oversized"));
        let outside = root.path().join("outside");
        fs::rename(&path, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(descriptor(&binary).is_err());
        assert_eq!(fs::read(outside).unwrap().len(), 4097);
    }
}
