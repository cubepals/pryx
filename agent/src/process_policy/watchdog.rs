// SPDX-License-Identifier: FSL-1.1-ALv2
//! The dedicated sampler reads procfs but never executes requests or untrusted code. It is
//! created before Landlock, so command workers need no procfs access to enforce a sampled budget.
use std::{collections::HashSet, fs, io, time::Duration};

pub(super) fn start(budget: u64) -> io::Result<()> {
    let root = std::process::id();
    std::thread::Builder::new().name("memory-watchdog".into()).spawn(move || {
        loop {
            let mut processes = Vec::new();
            if let Ok(entries) = fs::read_dir("/proc") {
                for entry in entries.flatten() {
                    let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
                    let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else { continue };
                    let Some((_, fields)) = stat.rsplit_once(") ") else { continue };
                    let fields: Vec<_> = fields.split_whitespace().collect();
                    let Some(parent) = fields.get(1).and_then(|s| s.parse::<u32>().ok()) else { continue };
                    let rss = fields.get(21).and_then(|s| s.parse::<u64>().ok()).unwrap_or_default();
                    processes.push((pid, parent, rss.saturating_mul(rustix::param::page_size() as u64)));
                }
            }
            let mut tree = HashSet::from([root]);
            loop {
                let before = tree.len();
                for (pid, parent, _) in &processes {
                    if tree.contains(parent) {
                        tree.insert(*pid);
                    }
                }
                if before == tree.len() {
                    break;
                }
            }
            let bytes: u64 = processes.iter().filter(|(pid, _, _)| tree.contains(pid)).map(|(_, _, rss)| *rss).sum();
            if bytes > budget {
                for pid in tree.iter().filter(|pid| **pid != root) {
                    if let Some(pid) = rustix::process::Pid::from_raw(*pid as i32) {
                        let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
                    }
                }
                let _ = rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::KILL);
                std::process::abort();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })?;
    Ok(())
}
