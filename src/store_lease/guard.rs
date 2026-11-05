// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! A pipe-fed process outside the daemon's service cgroup freezes and kills the leased subtree on EOF or boot-clock expiry.
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const ROOT: &str = "/sys/fs/cgroup";
const LEASES: &str = "pryxd-leases";
const WATCHDOGS: &str = "pryxd-watchdogs";
// A short check interval also notices CLOCK_BOOTTIME advancing across host suspension.
pub(crate) const STEP: Duration = Duration::from_millis(50);
pub(crate) fn boot_ms() -> io::Result<u64> {
    let value = rustix::time::clock_gettime_dynamic(rustix::time::DynamicClockId::Boottime).map_err(io::Error::from)?;
    Ok((value.tv_sec.max(0) as u64).saturating_mul(1000) + (value.tv_nsec.max(0) as u64) / 1_000_000)
}
pub(crate) fn relative(id: &str, epoch: u64) -> String {
    format!("{LEASES}/{}", hex::encode(Sha256::digest(format!("{id}:{epoch}"))))
}
pub(crate) fn docker_parent(id: &str, epoch: u64) -> String {
    format!("/{}", relative(id, epoch))
}
fn group(id: &str, epoch: u64) -> PathBuf {
    Path::new(ROOT).join(relative(id, epoch))
}
pub(crate) fn prepare(id: &str, epoch: u64) -> io::Result<()> {
    let root = Path::new(ROOT);
    fs::create_dir_all(root.join(LEASES))?;
    for at in [root.to_path_buf(), root.join(LEASES), group(id, epoch)] {
        fs::create_dir_all(&at)?;
        // cgroup v2 requires controllers enabled in each ancestor; no limit is relaxed.
        fs::write(at.join("cgroup.subtree_control"), "+cpu +memory +pids")?;
    }
    fs::create_dir_all(root.join(WATCHDOGS))?;
    Ok(())
}
fn write_one(path: &Path) -> io::Result<()> {
    fs::write(path, "1")
}
fn fence_group(path: &Path) -> io::Result<()> {
    // Freeze remains set: a Docker start completing late cannot run a new descendant.
    let frozen = write_one(&path.join("cgroup.freeze"));
    let killed = write_one(&path.join("cgroup.kill"));
    frozen.and(killed)
}
pub(crate) struct Guard {
    child: Option<Child>,
    input: Option<ChildStdin>,
    path: PathBuf,
}
impl Guard {
    pub(crate) fn start(id: &str, epoch: u64, deadline: u64) -> io::Result<Self> {
        prepare(id, epoch)?;
        let path = group(id, epoch);
        let mut child = Command::new(std::env::current_exe()?)
            .args([
                "lease-watchdog",
                "--group",
                path.to_str().ok_or_else(|| io::Error::other("cgroup path"))?,
                "--deadline",
                &deadline.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let input = child.stdin.take();
        let output = child.stdout.take().ok_or_else(|| io::Error::other("watchdog output"))?;
        let (send, recv) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let answer = BufReader::new(output).read_line(&mut line).map(|_| line);
            let _ = send.send(answer);
        });
        if recv.recv_timeout(Duration::from_secs(2)).ok().and_then(Result::ok).as_deref() != Some("ready\n") {
            drop(input);
            let _ = fence_group(&path);
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("watchdog readiness failed"));
        }
        if boot_ms()? >= deadline || child.try_wait()?.is_some() {
            drop(input);
            let _ = fence_group(&path);
            return Err(io::Error::other("watchdog lease ended before readiness"));
        }
        fs::write(path.join("cgroup.freeze"), "0")?;
        Ok(Self { child: Some(child), input, path })
    }
    pub(crate) fn alive(&mut self) -> bool {
        self.child.as_mut().is_some_and(|child| matches!(child.try_wait(), Ok(None)))
    }
    pub(crate) fn renew(&mut self, deadline: u64) -> io::Result<()> {
        writeln!(self.input.as_mut().ok_or_else(|| io::Error::other("closed watchdog"))?, "{deadline}")
    }
    pub(crate) fn fence(&mut self) -> io::Result<()> {
        self.input.take();
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        fence_group(&self.path)
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.input.take();
        // Reap without making a Tokio worker wait; EOF makes the independent process fence.
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}
/// Hidden daemon subcommand, deliberately using no async runtime or Docker API.
pub fn run(path: &Path, deadline: u64) -> io::Result<()> {
    let allowed = Path::new(ROOT).join(LEASES);
    if path.parent() != Some(allowed.as_path())
        || path.file_name().is_none_or(|p| {
            let value = p.to_string_lossy();
            value.len() != 64 || !value.bytes().all(|c| c.is_ascii_hexdigit())
        })
    {
        return Err(io::Error::other("invalid lease cgroup"));
    }
    // Move before readiness: systemd killing pryxd's service cgroup cannot kill this watchdog.
    fs::write(Path::new(ROOT).join(WATCHDOGS).join("cgroup.procs"), std::process::id().to_string())?;
    // Open both controls before admitting any start; missing cgroup.kill is a hard refusal.
    let _kill: File = OpenOptions::new().write(true).open(path.join("cgroup.kill"))?;
    let _freeze: File = OpenOptions::new().write(true).open(path.join("cgroup.freeze"))?;
    // Serialize reuse across daemon processes. A predecessor must finish its terminal freeze
    // before this process can announce readiness and let the parent unfreeze the subtree.
    let locks = Path::new("/run/pryxd-leases");
    fs::create_dir_all(locks)?;
    fs::set_permissions(locks, fs::Permissions::from_mode(0o700))?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(locks.join(path.file_name().ok_or_else(|| io::Error::other("lease group name"))?))?;
    lock.lock()?;
    let (send, recv) = mpsc::channel();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let value = line.ok().and_then(|v| v.parse::<u64>().ok());
            if send.send(value).is_err() {
                return;
            }
        }
        let _ = send.send(None);
    });
    println!("ready");
    io::stdout().flush()?;
    let mut until = deadline;
    while let Ok(now) = boot_ms() {
        if now >= until {
            break;
        }
        match recv.recv_timeout(STEP.min(Duration::from_millis(until - now))) {
            Ok(Some(next)) if boot_ms().is_ok_and(|now| now < until) => until = until.max(next),
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    let result = fence_group(path);
    eprintln!("store-lease self-fence: cgroup={} bootMs={}", path.display(), boot_ms().unwrap_or_default());
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fence_attempts_both_controls_and_leaves_parent_frozen() {
        let dir = tempfile::tempdir().unwrap();
        fence_group(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("cgroup.freeze")).unwrap(), "1");
        assert_eq!(fs::read_to_string(dir.path().join("cgroup.kill")).unwrap(), "1");
        assert_eq!(relative("one", 1), relative("one", 1));
        assert_ne!(relative("one", 1), relative("one", 2));
    }
}
