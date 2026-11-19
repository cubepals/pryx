// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-kernel process-bound evidence for the existing gVisor fork acceptance. Sentry and its
//! platform helpers consume host tasks separately from the guest's virtual nproc limit. A lost
//! exec channel is never proof: the target's own cgroup must report an actual new refusal.
use super::*;
use std::path::{Component, PathBuf};
use std::sync::atomic::AtomicBool;

pub(super) struct HostPids {
    path: PathBuf,
    refused: u64,
    oom_kills: u64,
    initial_peak: u64,
}

impl HostPids {
    pub(super) async fn capture(backend: &bollard::Docker, container: &str) -> Self {
        let info = backend.inspect_container(container, None).await.unwrap();
        assert_eq!(info.host_config.unwrap().pids_limit, Some(128));
        let state = info.state.unwrap();
        assert_eq!(state.running, Some(true));
        let pid = i32::try_from(state.pid.unwrap()).unwrap();
        let process = procfs::process::Process::new(pid).unwrap();
        let generation = process.stat().unwrap().starttime;
        let group = process.cgroups().unwrap().0.into_iter().find(|row| row.hierarchy == 0).unwrap();
        let relative = std::path::Path::new(&group.pathname).strip_prefix("/").unwrap();
        assert!(relative.components().all(|part| matches!(part, Component::Normal(_))));
        assert!(relative.components().any(|part| part.as_os_str().to_string_lossy().contains(container)));
        let path = PathBuf::from("/sys/fs/cgroup").join(relative);
        assert_eq!(std::fs::read_to_string(path.join("pids.max")).unwrap().trim(), "128");
        assert_eq!(process.stat().unwrap().starttime, generation);
        let refused = counter(&path, "pids.events", "max").unwrap();
        let oom_kills = counter(&path, "memory.events", "oom_kill").unwrap();
        assert_eq!(oom_kills, 0, "the fresh sandbox has not hit its memory bound");
        let initial_peak = read_peak(&path).unwrap();
        assert!(initial_peak > 0 && initial_peak <= 128);
        Self { path, refused, oom_kills, initial_peak }
    }

    pub(super) async fn observe(
        &self,
        execution: impl std::future::Future<Output = Result<pryxd::protocol::ExecResponse, NodeError>>,
    ) {
        let finished = AtomicBool::new(false);
        let execute = async {
            let result = execution.await;
            finished.store(true, Ordering::Release);
            result
        };
        let sample = async {
            let mut peak = self.initial_peak;
            let mut refused = self.refused;
            let mut confirmed_host_refusal = false;
            loop {
                if let Some(count) = present(counter(&self.path, "pids.events", "max")) {
                    refused = refused.max(count);
                    // Read the kernel's retained high-water mark after its refusal counter.
                    // A sampled pids.current value cannot establish the actual task peak.
                    if let Some(kernel_peak) = present(read_peak(&self.path)) {
                        assert!(kernel_peak <= 128, "kernel pids.peak exceeded the requested bound: {kernel_peak}");
                        peak = peak.max(kernel_peak);
                        confirmed_host_refusal |= count > self.refused;
                    }
                }
                if let Some(count) = present(counter(&self.path, "memory.events", "oom_kill")) {
                    assert_eq!(count, self.oom_kills, "the process refusal must not be a memory kill");
                }
                if finished.load(Ordering::Acquire) {
                    return (peak, refused, confirmed_host_refusal);
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        let (out, (peak, refused, confirmed_host_refusal)) = tokio::join!(execute, sample);
        assert!(peak <= 128, "the sandbox exceeded its aggregate host-task ceiling: {peak}");
        let virtual_refusal = if let Ok(out) = &out {
            assert_ne!(out.exit_code, Some(0), "forking beyond the bound succeeded: {out:?}");
            assert!(!out.stdout.contains("exceeded-limit"), "{out:?}");
            // BusyBox ash reports a failed guest fork; host pids can refuse Sentry first.
            out.stdout.contains("can't fork")
        } else {
            false
        };
        assert!(
            virtual_refusal || confirmed_host_refusal,
            "neither a virtual fork refusal nor a new owned host-kernel pids refusal: {out:?}"
        );
        eprintln!(
            "GVISOR_PROCESS_BOUND max=128 kernel_peak={peak} refused_before={} refused_after={refused} virtual_refusal={virtual_refusal}",
            self.refused
        );
    }

    pub(super) async fn assert_empty(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(peak) = present(read_peak(&self.path)) {
                assert!(peak <= 128, "target retirement exceeded its aggregate task bound: {peak}");
            }
            match std::fs::read_to_string(self.path.join("cgroup.events")) {
                Ok(events) if events.lines().any(|line| line == "populated 0") => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) if error.raw_os_error() == Some(libc::ENODEV) => (),
                Err(error) => panic!("cannot prove the stopped target's cgroup is empty: {error}"),
                Ok(_) => (),
            }
            assert!(Instant::now() < deadline, "a fork-storm task outlived target retirement");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn counter(path: &std::path::Path, file: &str, field: &str) -> std::io::Result<u64> {
    std::fs::read_to_string(path.join(file))?
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(' ')?;
            (key == field).then(|| value.parse().unwrap())
        })
        .ok_or_else(|| std::io::Error::other("missing kernel cgroup event counter"))
}

fn read_peak(path: &std::path::Path) -> std::io::Result<u64> {
    Ok(std::fs::read_to_string(path.join("pids.peak"))?.trim().parse().unwrap())
}

fn present(value: std::io::Result<u64>) -> Option<u64> {
    match value {
        Ok(count) => Some(count),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        // kernfs can retire a node after the pathname lookup or after open.
        Err(error) if error.raw_os_error() == Some(libc::ENODEV) => None,
        Err(error) => panic!("lost target kernel cgroup evidence: {error}"),
    }
}

#[test]
fn a_retiring_counter_supplies_no_refusal_evidence() {
    assert_eq!(present(Ok(17)), Some(17));
    for error in [libc::ENOENT, libc::ENODEV] {
        assert_eq!(present(Err(std::io::Error::from_raw_os_error(error))), None);
    }
}

#[test]
#[should_panic(expected = "lost target kernel cgroup evidence")]
fn unreadable_counters_still_fail_the_test() {
    present(Err(std::io::Error::from_raw_os_error(libc::EACCES)));
}
