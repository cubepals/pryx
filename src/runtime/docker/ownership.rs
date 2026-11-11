// SPDX-License-Identifier: FSL-1.1-ALv2
//! Read-only proof that a local engine maps the host-owned data into the configured nonzero
//! workload identity. No ownership changes, namespace entry or compute are used for this probe.
use super::{DockerRuntime, RuntimeInfo};
use crate::config::{Ids, WorkloadPolicy};
use std::os::unix::fs::MetadataExt;

pub const DATA_OWNER_REQUIREMENT: &str = "data-owner-mapping";

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

pub(super) async fn problem(runtime: &DockerRuntime, info: &RuntimeInfo) -> Option<String> {
    if !info.rootless {
        return None;
    }
    proof(runtime, info).await.err().map(|why| format!(
        "{DATA_OWNER_REQUIREMENT} is unavailable: {why}; use Podman keep-id or an engine with a matching nonzero UID/GID map"))
}

async fn proof(runtime: &DockerRuntime, info: &RuntimeInfo) -> Result<(), String> {
    let stream =
        tokio::time::timeout(super::super::engine::PROBE_TIMEOUT, tokio::net::UnixStream::connect(&runtime.socket))
            .await
            .map_err(|_| "the selected engine's peer did not answer".to_owned())?
            .map_err(|_| "the selected engine's peer is unavailable".to_owned())?;
    let peer = stream.peer_cred().map_err(|_| "the engine's local identity is unreadable".to_owned())?;
    let expected = runtime.ownership.owner_ids();
    let mapped = if info.podman {
        // Podman keep-id maps its original daemon user to the requested nonzero IDs.
        (peer.uid(), peer.gid())
    } else {
        let pid = peer
            .pid()
            .filter(|pid| *pid > 0)
            .ok_or_else(|| "the selected engine has no visible local process".to_owned())?;
        let same_namespace = std::fs::metadata("/proc/self/ns/user")
            .ok()
            .zip(std::fs::metadata(format!("/proc/{pid}/ns/user")).ok())
            .is_some_and(|(ours, theirs)| ours.dev() == theirs.dev() && ours.ino() == theirs.ino());
        if same_namespace {
            (runtime.ownership.user.uid, runtime.ownership.user.gid)
        } else {
            // proc_pid_uid_map(5): when read from another user namespace, outside IDs
            // are relative to this reader's namespace, which owns the data directory.
            let uids = tokio::fs::read_to_string(format!("/proc/{pid}/uid_map"))
                .await
                .map_err(|_| "the engine's UID map is unreadable".to_owned())?;
            let gids = tokio::fs::read_to_string(format!("/proc/{pid}/gid_map"))
                .await
                .map_err(|_| "the engine's GID map is unreadable".to_owned())?;
            (
                mapped_id(&uids, runtime.ownership.user.uid)
                    .ok_or_else(|| "the workload UID is unmapped".to_owned())?,
                mapped_id(&gids, runtime.ownership.user.gid)
                    .ok_or_else(|| "the workload GID is unmapped".to_owned())?,
            )
        }
    };
    if mapped != expected {
        return Err(format!(
            "workload {}:{} maps to host {}:{}, while data belongs to {}:{}",
            runtime.ownership.user.uid, runtime.ownership.user.gid, mapped.0, mapped.1, expected.0, expected.1
        ));
    }
    Ok(())
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

    #[test]
    fn actual_kernel_map_rows_have_checked_ranges() {
        let map = std::fs::read_to_string("/proc/self/uid_map").unwrap();
        let row = map.lines().next().unwrap().split_whitespace().map(|s| s.parse::<u64>().unwrap()).collect::<Vec<_>>();
        let first = u32::try_from(row[0]).unwrap();
        assert_eq!(mapped_id(&map, first), u32::try_from(row[1]).ok());
        assert_eq!(mapped_id(&format!("{map}\ninvalid"), first), None);
    }
}
