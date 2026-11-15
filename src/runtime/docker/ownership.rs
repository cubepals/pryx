// SPDX-License-Identifier: FSL-1.1-ALv2
//! Read-only proof that a local engine maps the configured nonzero workload identity onto the
//! host owner of its data: the configured owner for data not yet made, and a directory's own
//! stat owner for data that exists. No ownership changes, namespace entry or compute are used.
use super::{ContainerRuntime, DATA_OWNER_REQUIREMENT, DockerRuntime, RuntimeError, RuntimeInfo};
use crate::config::{Ids, WorkloadPolicy};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

#[derive(Clone, Copy)]
pub(crate) struct Policy {
    pub(crate) user: Ids,
    pub(crate) owner: Option<Ids>,
}

impl Default for Policy {
    fn default() -> Self {
        Self { user: WorkloadPolicy::default().user, owner: None }
    }
}
impl Policy {
    fn owner_ids(self) -> (u32, u32) {
        self.owner
            .map(|ids| (ids.uid, ids.gid))
            .unwrap_or_else(|| (rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw()))
    }
}

/// Whether the engine can take new data: a new data directory is given the configured owner.
pub(super) async fn problem(runtime: &DockerRuntime, info: &RuntimeInfo) -> Option<String> {
    if !info.rootless {
        return None;
    }
    let owner = runtime.ownership.owner_ids();
    proof(runtime, info, owner, "data belongs to").await.err().map(|why| format!(
        "{DATA_OWNER_REQUIREMENT} is unavailable: {why}; use Podman keep-id or an engine with a matching nonzero UID/GID map"))
}

/// Whether compute may bind this data directory as it is now. Its owner is read from the
/// directory itself, never followed through a link: one made under an earlier configuration, or
/// given to someone else since, keeps that owner whatever the configuration says today.
pub(super) async fn data_problem(runtime: &DockerRuntime, info: &RuntimeInfo, data: &Path) -> Option<String> {
    if !info.rootless {
        return None;
    }
    let owner = match std::fs::symlink_metadata(data) {
        Ok(metadata) if metadata.file_type().is_dir() => (metadata.uid(), metadata.gid()),
        Ok(_) => {
            return Some(format!("{DATA_OWNER_REQUIREMENT} is unavailable: the data directory is not a directory"));
        }
        Err(_) => return Some(format!("{DATA_OWNER_REQUIREMENT} is unavailable: the data directory is unreadable")),
    };
    proof(runtime, info, owner, "its data directory belongs to")
        .await
        .err()
        .map(|why| format!("{DATA_OWNER_REQUIREMENT} is unavailable for this workload's data: {why}"))
}

/// Existing compute's check before it starts again, from the engine observation already held.
pub(super) async fn check_existing(runtime: &DockerRuntime, data: &Path) -> Result<(), RuntimeError> {
    let cached = runtime.info.lock().unwrap().clone();
    let info = match cached {
        Some(info) => info,
        None => runtime.info().await?,
    };
    data_problem(runtime, &info, data).await.map_or(Ok(()), |problem| Err(RuntimeError::Other(problem)))
}

/// Where the configured workload identity lands on the host, compared with `expected`.
async fn proof(runtime: &DockerRuntime, info: &RuntimeInfo, expected: (u32, u32), what: &str) -> Result<(), String> {
    let mapped = mapped(runtime, info).await?;
    if mapped != expected {
        return Err(format!(
            "workload {}:{} maps to host {}:{}, while {what} {}:{}",
            runtime.ownership.user.uid, runtime.ownership.user.gid, mapped.0, mapped.1, expected.0, expected.1
        ));
    }
    Ok(())
}

async fn mapped(runtime: &DockerRuntime, info: &RuntimeInfo) -> Result<(u32, u32), String> {
    let stream =
        tokio::time::timeout(super::super::ENGINE_PROBE_TIMEOUT, tokio::net::UnixStream::connect(&runtime.socket))
            .await
            .map_err(|_| "the selected engine's peer did not answer".to_owned())?
            .map_err(|_| "the selected engine's peer is unavailable".to_owned())?;
    let peer = stream.peer_cred().map_err(|_| "the engine's local identity is unreadable".to_owned())?;
    if info.podman {
        // Podman keep-id maps its original daemon user to the requested nonzero IDs.
        return Ok((peer.uid(), peer.gid()));
    }
    let pid = peer
        .pid()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| "the selected engine has no visible local process".to_owned())?;
    let same_namespace = std::fs::metadata("/proc/self/ns/user")
        .ok()
        .zip(std::fs::metadata(format!("/proc/{pid}/ns/user")).ok())
        .is_some_and(|(ours, theirs)| ours.dev() == theirs.dev() && ours.ino() == theirs.ino());
    if same_namespace {
        return Ok((runtime.ownership.user.uid, runtime.ownership.user.gid));
    }
    // proc_pid_uid_map(5): when read from another user namespace, outside IDs
    // are relative to this reader's namespace, which owns the data directory.
    let uids = tokio::fs::read_to_string(format!("/proc/{pid}/uid_map"))
        .await
        .map_err(|_| "the engine's UID map is unreadable".to_owned())?;
    let gids = tokio::fs::read_to_string(format!("/proc/{pid}/gid_map"))
        .await
        .map_err(|_| "the engine's GID map is unreadable".to_owned())?;
    Ok((
        mapped_id(&uids, runtime.ownership.user.uid).ok_or_else(|| "the workload UID is unmapped".to_owned())?,
        mapped_id(&gids, runtime.ownership.user.gid).ok_or_else(|| "the workload GID is unmapped".to_owned())?,
    ))
}

fn mapped_id(map: &str, inside: u32) -> Option<u32> {
    let mut found = None;
    for line in map.lines().filter(|line| !line.trim().is_empty()) {
        let fields = line.split_whitespace().map(str::parse::<u64>).collect::<Result<Vec<_>, _>>().ok()?;
        let [first, outside, length] = fields.as_slice() else { return None };
        let offset = u64::from(inside).checked_sub(*first);
        if let Some(offset) = offset.filter(|offset| offset < length) {
            if found.is_some() {
                return None;
            }
            found = Some(u32::try_from(outside.checked_add(offset)?).ok()?);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_real_local_peer_is_checked_against_the_owned_host_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.sock");
        let _listener = tokio::net::UnixListener::bind(&path).unwrap();
        let mut runtime = DockerRuntime::new(&path).unwrap();
        let user =
            Ids { uid: rustix::process::geteuid().as_raw().max(1), gid: rustix::process::getegid().as_raw().max(1) };
        runtime.ownership = Policy { user, owner: Some(user) };
        let info = RuntimeInfo { rootless: true, ..Default::default() };
        assert!(problem(&runtime, &info).await.is_none());
        runtime.ownership.owner = Some(Ids { uid: user.uid.checked_add(1).unwrap(), gid: user.gid });
        assert!(problem(&runtime, &info).await.unwrap().contains(DATA_OWNER_REQUIREMENT));
    }

    #[tokio::test]
    async fn existing_data_is_checked_against_its_own_owner_not_the_configured_one() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.sock");
        let _listener = tokio::net::UnixListener::bind(&path).unwrap();
        let data = directory.path().join("data");
        std::fs::create_dir(&data).unwrap();
        if rustix::process::geteuid().is_root() {
            // A workload identity is never zero; give the directory a nonzero owner as a create would.
            std::os::unix::fs::chown(&data, Some(1), Some(1)).unwrap();
        }
        let metadata = std::fs::metadata(&data).unwrap();
        let real = Ids { uid: metadata.uid(), gid: metadata.gid() };
        let mut runtime = DockerRuntime::new(&path).unwrap();
        let info = RuntimeInfo { rootless: true, ..Default::default() };
        *runtime.info.lock().unwrap() = Some(info.clone());
        // The configuration names another owner, and the workload maps there: new data would be served.
        let configured = Ids { uid: real.uid.checked_add(1).unwrap(), gid: real.gid };
        runtime.ownership = Policy { user: configured, owner: Some(configured) };
        assert!(problem(&runtime, &info).await.is_none());
        let refused = runtime.check_data_owner("blockly-test-w", &data).await.unwrap_err().to_string();
        assert!(refused.contains(DATA_OWNER_REQUIREMENT), "{refused}");
        assert!(
            refused.contains(&format!(
                "maps to host {}:{}, while its data directory belongs to {}:{}",
                configured.uid, configured.gid, real.uid, real.gid
            )),
            "{refused}"
        );
        runtime.ownership = Policy { user: real, owner: Some(configured) };
        runtime.check_data_owner("blockly-test-w", &data).await.unwrap();
        // A link to a directory with the right owner is still no data directory of its own.
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&data, &link).unwrap();
        let refused = runtime.check_data_owner("blockly-test-w", &link).await.unwrap_err().to_string();
        assert!(refused.contains("not a directory"), "{refused}");
        *runtime.info.lock().unwrap() = Some(RuntimeInfo::default());
        runtime.ownership = Policy { user: configured, owner: Some(configured) };
        runtime.check_data_owner("blockly-test-w", &data).await.unwrap();
    }

    #[test]
    fn actual_kernel_map_rows_have_checked_ranges() {
        let map = std::fs::read_to_string("/proc/self/uid_map").unwrap();
        let row = map.lines().next().unwrap().split_whitespace().map(|s| s.parse::<u64>().unwrap()).collect::<Vec<_>>();
        let first = u32::try_from(row[0]).unwrap();
        assert_eq!(mapped_id(&map, first), u32::try_from(row[1]).ok());
        assert_eq!(mapped_id(&format!("{map}\ninvalid"), first), None);
    }
}
