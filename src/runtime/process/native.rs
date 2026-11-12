// SPDX-License-Identifier: FSL-1.1-ALv2
//! Native inputs come from the protected installation, never workload fields. Enabling them
//! requires fresh signed scope observations of managed peers and a persistent launch guard.
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
impl ProcessRuntime {
    fn tool_paths(&self, owner: (u32, u32)) -> Result<(PathBuf, PathBuf, PathBuf), RuntimeError> {
        let base = self.state.root.parent().ok_or_else(|| error("Missing process state parent"))?.join("process-tools");
        if !base.exists() {
            directory(&base, (rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw()))?;
        }
        let info = fs::symlink_metadata(&base).map_err(error)?;
        if !info.is_dir() || info.uid() != rustix::process::geteuid().as_raw() || info.mode() & 0o022 != 0 {
            return Err(error("Unsafe process tool root"));
        }
        fs::set_permissions(&base, fs::Permissions::from_mode(0o711)).map_err(error)?;
        let uid = directory(&base.join(owner.0.to_string()), owner)?;
        Ok((
            directory(&uid.join("cache"), owner)?,
            directory(&uid.join("downloads"), owner)?,
            directory(&uid.join(format!("scratch-{}", uuid::Uuid::new_v4())), owner)?,
        ))
    }
    pub(super) async fn native_policy(&self, entry: &Entry) -> Result<(Option<Policy>, bool), RuntimeError> {
        let required = self.scope_required()?;
        let agent = entry.spec.agent.as_ref().ok_or_else(|| error("Missing process supervisor"))?;
        let metadata = crate::runtime::agent_metadata::query(&agent.binary).await;
        if metadata.is_none_or(|value| value.process_tools_version < VERSION) {
            if required {
                return Err(error(
                    "This host requires scoped process agents; upgrade the legacy supervisor before launching compute",
                ));
            }
            note_unscoped(&self.state.root)?;
            return Ok((None, false));
        }
        let artifact = descriptor(&agent.binary);
        let glibc =
            crate::capability::process_tools::prerequisite(std::env::consts::ARCH, &crate::capability::c_library());
        let mut unavailable = (glibc.state != crate::capability::State::On).then_some(glibc.evidence);
        let scoped = pryx_agent::process_capabilities::private_ipc_probe().supported();
        if !scoped {
            note_unscoped(&self.state.root)?;
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
        if unavailable.is_none() && !required {
            crate::durable::write_atomic(&self.state.root.join("native-scope-required"), b"1\n", 0o600)
                .map_err(error)?;
        }
        let (cache, downloads, scratch) = self.tool_paths(self.owner(&agent.sandbox)?)?;
        let browser_on = unavailable.is_none()
            && crate::capability::child("probe-user-namespace").state == crate::capability::State::On;
        Ok((
            Some(Policy { cache, downloads, scratch, artifact: artifact.ok(), unavailable, browser_on }),
            required || scoped,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
