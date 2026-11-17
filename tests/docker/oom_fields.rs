// SPDX-License-Identifier: FSL-1.1-ALv2
//! Parsers for fixed diagnostic kernel fields, with no process command-line or arbitrary log
//! output. These controls exercise parsing and identity checks; they do not simulate kernel OOM.
use std::path::{Component, Path, PathBuf};

pub(super) fn container_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub(super) fn start_ticks(stat: &str, pid: i32) -> Option<u64> {
    let (head, fields) = stat.rsplit_once(") ")?;
    let (number, _) = head.split_once(" (")?;
    (number.parse::<i32>().ok()? == pid)
        .then_some(())
        .and_then(|()| fields.split_whitespace().nth(19)?.parse::<u64>().ok())
        .filter(|ticks| *ticks > 0)
}

pub(super) fn unified_group(text: &str, id: &str) -> Option<PathBuf> {
    let mut rows = text.lines().filter_map(|line| line.strip_prefix("0::"));
    let path = PathBuf::from(rows.next()?);
    if rows.next().is_some() || !absolute_normal(&path) {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    (name == id || name == format!("docker-{id}.scope")).then_some(path)
}

fn absolute_normal(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
        && !path.as_os_str().as_encoded_bytes().contains(&0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Mount {
    pub(super) point: PathBuf,
    pub(super) relative: PathBuf,
    pub(super) read_only: bool,
    pub(super) local_events: bool,
}

pub(super) fn group_mount(text: &str, group: &Path) -> Option<Mount> {
    text.lines().filter_map(|line| mount_row(line, group)).max_by_key(|mount| mount.point.components().count())
}

fn mount_row(line: &str, group: &Path) -> Option<Mount> {
    let (before, after) = line.split_once(" - ")?;
    let fields: Vec<_> = before.split_whitespace().collect();
    let filesystem: Vec<_> = after.split_whitespace().collect();
    if fields.len() < 6 || filesystem.len() != 3 || filesystem[0] != "cgroup2" {
        return None;
    }
    let root = PathBuf::from(unescape(fields[3])?);
    let point = PathBuf::from(unescape(fields[4])?);
    if !absolute_normal(&root) || !absolute_normal(&point) {
        return None;
    }
    let relative = group.strip_prefix(root).ok()?.to_path_buf();
    Some(Mount {
        point,
        relative,
        read_only: fields[5].split(',').any(|flag| flag == "ro"),
        local_events: filesystem[2].split(',').any(|flag| flag == "memory_localevents"),
    })
}

fn unescape(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut parts = text.split('\\');
    out.push_str(parts.next()?);
    for part in parts {
        let (escape, rest) = part.split_at_checked(3)?;
        out.push(match escape {
            "040" => ' ',
            "011" => '\t',
            "012" => '\n',
            "134" => '\\',
            _ => return None,
        });
        out.push_str(rest);
    }
    Some(out)
}

pub(super) fn oom_kills(text: &str) -> Option<u64> {
    let mut value = None;
    for row in text.lines() {
        let fields: Vec<_> = row.split_whitespace().collect();
        if fields.len() != 2 {
            return None;
        }
        if fields[0] == "oom_kill" {
            if value.is_some() {
                return None;
            }
            value = Some(fields[1].parse().ok()?);
        }
    }
    value
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", content = "bytes", rename_all = "snake_case")]
pub(super) enum Limit {
    Unlimited,
    Bytes(u64),
}

pub(super) fn memory_limit(text: &str) -> Option<Limit> {
    match text.trim() {
        "max" => Some(Limit::Unlimited),
        number => number.parse().ok().map(Limit::Bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn pid_stat_uses_the_final_comm_delimiter_and_exact_pid() {
        let suffix = std::iter::repeat_n("0", 18).collect::<Vec<_>>().join(" ");
        let stat = format!("42 (a tricky ) process) S {suffix} 9876 0");
        assert_eq!(start_ticks(&stat, 42), Some(9876));
        assert_eq!(start_ticks(&stat, 43), None);
        assert_eq!(start_ticks("42 (missing fields) S", 42), None);
    }

    #[test]
    fn a_shared_or_different_container_group_is_not_owned() {
        assert!(container_id(ID));
        assert!(!container_id(&ID.to_uppercase()));
        assert!(unified_group(&format!("0::/system.slice/docker-{ID}.scope\n"), ID).is_some());
        assert!(unified_group(&format!("0::/docker/{ID}\n"), ID).is_some());
        assert!(unified_group("0::/system.slice/docker.service\n", ID).is_none());
        assert!(unified_group(&format!("0::/docker/../{ID}\n"), ID).is_none());
        assert!(unified_group(&format!("0::/docker/{ID}\n0::/docker/{ID}\n"), ID).is_none());
    }

    #[test]
    fn mount_binding_requires_a_matching_unified_root() {
        let text = "1 0 0:1 /other /wrong rw - cgroup2 cgroup rw\n\
                    2 0 0:2 /docker /read\\040only ro - cgroup2 cgroup rw,memory_localevents\n";
        let mounted = group_mount(text, Path::new("/docker/owned")).unwrap();
        assert_eq!(mounted.point, Path::new("/read only"));
        assert_eq!(mounted.relative, Path::new("owned"));
        assert!(mounted.read_only && mounted.local_events);
        assert!(group_mount(text, Path::new("/outside/owned")).is_none());
        assert!(group_mount("1 0 0:1 / /cgroup rw - tmpfs tmpfs rw", Path::new("/owned")).is_none());
    }

    #[test]
    fn missing_malformed_or_duplicate_counts_are_unknown() {
        assert_eq!(oom_kills("oom 3\noom_kill 2\noom_group_kill 1\n"), Some(2));
        for text in ["oom 3\n", "oom_kill no\n", "oom_kill 1\noom_kill 2\n", "oom_kill -1\n", "oom_kill 1 extra\n"] {
            assert_eq!(oom_kills(text), None);
        }
        assert_eq!(memory_limit("max\n"), Some(Limit::Unlimited));
        assert_eq!(memory_limit("67108864\n"), Some(Limit::Bytes(67108864)));
        assert_eq!(memory_limit("unknown"), None);
    }
}
