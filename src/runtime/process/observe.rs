// SPDX-License-Identifier: FSL-1.1-ALv2
//! Read-only kernel process observations shared by launch, adoption and termination. No
//! journal, manager, signaling or workload state is modified here.
use std::{collections::HashSet, fs};
pub(super) fn identity(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(") ")?;
    let mut fields = fields.split_whitespace();
    if matches!(fields.next(), Some("Z" | "X")) {
        return None;
    }
    fields.nth(18)?.parse().ok()
}
pub(super) fn matching_pid(pid: Option<u32>, ticks: Option<u64>) -> Option<u32> {
    pid.filter(|pid| identity(*pid) == ticks)
}
pub(super) fn tree(root: u32) -> Vec<(u32, u64)> {
    let mut all = Vec::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
            let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else { continue };
            let Some((_, fields)) = stat.rsplit_once(") ") else { continue };
            let fields: Vec<_> = fields.split_whitespace().collect();
            if let (Some(parent), Some(ticks)) = (fields.get(1).and_then(|s| s.parse::<u32>().ok()), identity(pid)) {
                all.push((pid, parent, ticks));
            }
        }
    }
    let mut found = HashSet::from([root]);
    loop {
        let before = found.len();
        for (pid, parent, _) in &all {
            if found.contains(parent) {
                found.insert(*pid);
            }
        }
        if before == found.len() {
            break;
        }
    }
    all.into_iter().filter(|(pid, _, _)| found.contains(pid)).map(|(pid, _, ticks)| (pid, ticks)).collect()
}
pub(super) fn resident_tree(root: u32) -> Option<u64> {
    tree(root).into_iter().try_fold(0u64, |sum, (pid, _)| {
        let memory = fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
        Some(sum.saturating_add(
            memory.split_whitespace().nth(1)?.parse::<u64>().ok()?.saturating_mul(rustix::param::page_size() as u64),
        ))
    })
}
pub(super) fn uid_processes(uid: u32) -> Vec<(u32, u64)> {
    let mut found = Vec::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
            let Ok(status) = fs::read_to_string(entry.path().join("status")) else { continue };
            let owner = status
                .lines()
                .find_map(|line| line.strip_prefix("Uid:"))
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse::<u32>().ok());
            if owner == Some(uid)
                && let Some(ticks) = identity(pid)
            {
                found.push((pid, ticks));
            }
        }
    }
    found
}
pub(super) fn uid_in_use(uid: u32) -> bool {
    !uid_processes(uid).is_empty()
}
