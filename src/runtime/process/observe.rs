// SPDX-License-Identifier: FSL-1.1-ALv2
//! Read-only kernel process observations shared by launch, adoption and termination. No
//! journal, manager, signaling or workload state is modified here.
use std::{collections::HashSet, fs};

/// Complete negative UID evidence, unlike the best-effort orphan sweep below. Filtered proc
/// mounts and unreadable task metadata mean unknown, never an empty UID.
pub(super) fn uid_processes_proven(uid: u32) -> std::io::Result<Vec<u32>> {
    uid_processes_at(uid, std::path::Path::new("/proc"))
}

fn uid_processes_at(uid: u32, root: &std::path::Path) -> std::io::Result<Vec<u32>> {
    use std::io::{Error, ErrorKind};
    let mounts = fs::read_to_string(root.join("self/mountinfo"))?;
    let mut candidates = mounts.lines().filter_map(|line| {
        let (before, after) = line.split_once(" - ")?;
        if before.split_whitespace().nth(4)? != "/proc" {
            return None;
        }
        Some(after)
    });
    let unknown = || Error::new(ErrorKind::PermissionDenied, "The process table's mount visibility is unknown");
    let mount = candidates.next().ok_or_else(unknown)?;
    // Stacked mounts at the same point leave lower records in mountinfo. Without proving
    // which mount is visible, an unfiltered lower record says nothing about the live view.
    if candidates.next().is_some() {
        return Err(unknown());
    }
    let mut mount = mount.split_whitespace();
    if mount.next() != Some("proc") {
        return Err(unknown());
    }
    mount.next().ok_or_else(unknown)?;
    let options = mount.next().ok_or_else(unknown)?;
    if options
        .split(',')
        .any(|option| option.strip_prefix("hidepid=").is_some_and(|value| value != "0" && value != "off"))
    {
        return Err(Error::new(ErrorKind::PermissionDenied, "A filtered process table cannot prove a UID empty"));
    }
    let mut found = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let status = match fs::read_to_string(entry.path().join("status")) {
            Ok(status) => status,
            Err(problem)
                if problem.kind() == ErrorKind::NotFound
                    && fs::symlink_metadata(entry.path()).is_err_and(|gone| gone.kind() == ErrorKind::NotFound) =>
            {
                continue;
            }
            Err(problem) => return Err(problem),
        };
        let owner = status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "A process UID could not be read"))?;
        if owner != uid {
            continue;
        }
        let stat = match fs::read_to_string(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(problem)
                if problem.kind() == ErrorKind::NotFound
                    && fs::symlink_metadata(entry.path()).is_err_and(|gone| gone.kind() == ErrorKind::NotFound) =>
            {
                continue;
            }
            Err(problem) => return Err(problem),
        };
        let state = stat
            .rsplit_once(") ")
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "A process state could not be read"))?;
        if !matches!(state, "Z" | "X") {
            found.push(pid);
        }
    }
    Ok(found)
}
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
    let ticks = ticks?;
    pid.filter(|pid| identity(*pid) == Some(ticks))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_or_filtered_uid_evidence_is_never_an_empty_uid() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("self")).unwrap();
        let mounts = root.path().join("self/mountinfo");
        fs::write(&mounts, "1 0 0:1 / /proc rw - proc proc rw\n").unwrap();
        assert!(uid_processes_at(123, root.path()).unwrap().is_empty());
        fs::create_dir(root.path().join("42")).unwrap();
        assert!(uid_processes_at(123, root.path()).is_err(), "an unreadable task is not an absent task");
        fs::write(root.path().join("42/status"), "Uid:\t123 123 123 123\n").unwrap();
        fs::write(root.path().join("42/stat"), "42 (owned) S 1\n").unwrap();
        assert_eq!(uid_processes_at(123, root.path()).unwrap(), [42]);
        fs::write(root.path().join("42/status"), "Uid:\t456 456 456 456\n").unwrap();
        fs::write(&mounts, "1 0 0:1 / /proc rw - proc proc rw,hidepid=2\n").unwrap();
        assert!(uid_processes_at(123, root.path()).is_err(), "a hidden UID is not an empty UID");
        for top in ["proc proc rw,hidepid=2", "tmpfs tmpfs rw"] {
            fs::write(&mounts, format!("1 0 0:1 / /proc rw - proc proc rw\n2 1 0:2 / /proc rw - {top}\n")).unwrap();
            assert!(uid_processes_at(123, root.path()).is_err(), "an unfiltered lower mount is not the visible view");
        }
    }
}
