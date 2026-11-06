// SPDX-License-Identifier: FSL-1.1-ALv2
//! Freeze a stable descendant set before forced termination. Dedicated UIDs also let the node
//! retire orphaned compute after a supervisor dies; shared-UID mode cannot signal unrelated
//! processes and deliberately retains its weaker boundary.
use super::journal::error;
use super::observe::{identity, matching_pid, tree, uid_processes};
use super::*;
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
use std::fs;

fn signal(pid: u32, ticks: u64, signal: Signal) {
    let Some(value) = Pid::from_raw(pid as i32) else { return };
    if let Ok(fd) = pidfd_open(value, PidfdFlags::empty())
        && identity(pid) == Some(ticks)
    {
        let _ = pidfd_send_signal(fd, signal);
    }
}
fn stopped(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|text| text.rsplit_once(") ").map(|(_, fields)| fields.starts_with(['T', 't'])))
        .unwrap_or(false)
}
impl ProcessRuntime {
    pub(super) async fn freeze_tree(&self, name: &str) -> Result<Vec<(u32, u64)>, RuntimeError> {
        let entry = self.entry(name)?;
        let Some(root) = matching_pid(entry.pid, entry.ticks) else { return Ok(Vec::new()) };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let mut targets = tree(root);
            targets.sort_by_key(|(pid, _)| *pid != root);
            for (pid, ticks) in targets {
                signal(pid, ticks, Signal::STOP);
            }
            let current = tree(root);
            if !current.is_empty() && current.iter().all(|(pid, _)| stopped(*pid)) {
                return Ok(current);
            }
            if matching_pid(Some(root), entry.ticks).is_none() {
                return Ok(Vec::new());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(RuntimeError::Timeout("Process tree did not freeze".into()));
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    pub(super) fn retire_orphans(&self, id: &str) -> Result<(), RuntimeError> {
        if self.shared_uid() {
            return Ok(());
        }
        let owner = self.state.owners.lock().unwrap().get(id).copied();
        if let Some((uid, _)) = owner {
            for _ in 0..10 {
                let found = uid_processes(uid);
                if found.is_empty() {
                    return Ok(());
                }
                for (pid, ticks) in found {
                    signal(pid, ticks, Signal::KILL)
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if !uid_processes(uid).is_empty() {
                return Err(error("Orphaned process sandbox has not exited"));
            }
        }
        Ok(())
    }
    pub(super) async fn terminate_tree(&self, name: &str) -> Result<(), RuntimeError> {
        let entry = self.entry(name)?;
        let mut targets = self.freeze_tree(name).await?;
        targets.sort_by_key(|(pid, _)| Some(*pid) == entry.pid);
        for (pid, ticks) in targets {
            signal(pid, ticks, Signal::KILL)
        }
        if let Some(agent) = &entry.spec.agent {
            self.retire_orphans(&agent.sandbox)?
        }
        Ok(())
    }
}
