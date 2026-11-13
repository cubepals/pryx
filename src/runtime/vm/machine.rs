// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Owned monitor launch and one-shot secret bootstrap. The node supplies verified bundle
//! inputs and a host-owned confinement plan; workload fields cannot choose the launcher,
//! listener paths or monitor policy. This module neither admits workloads nor announces a tier.
use super::error;
use crate::runtime::RuntimeError;
use pryx_agent::{vm_boot, vm_monitor};
use std::{
    fs::{self, OpenOptions},
    io,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
};
use tokio::{io::AsyncWriteExt, net::UnixListener, process::Command, task::JoinHandle, time::Instant};

pub struct Machine {
    child: tokio::process::Child,
    bootstrap: Option<JoinHandle<io::Result<()>>>,
    listener: PathBuf,
}

fn private(path: &Path) -> Result<(), RuntimeError> {
    super::bundle::ancestors(path).map_err(error)?;
    let info = fs::symlink_metadata(path).map_err(error)?;
    if info.uid() != rustix::process::geteuid().as_raw() || info.mode() & 0o077 != 0 {
        return Err(error("VM launch state must be a private owned directory"));
    }
    Ok(())
}

impl Machine {
    /// Bootstrap secrets stay on an accepted private socket, and the confinement plan stays
    /// on the launcher's stdin. There is no retry of an exec or bootstrap submission.
    pub async fn launch(
        launcher: &Path,
        plan: &vm_monitor::Plan,
        channel: &Path,
        serial: &Path,
        boot: &vm_boot::Boot,
        deadline: Instant,
    ) -> Result<Self, RuntimeError> {
        private(&plan.directory)?;
        if channel.parent() != Some(plan.directory.as_path()) || serial.parent() != Some(plan.directory.as_path()) {
            return Err(error("VM listeners and serial output must stay in its private launch directory"));
        }
        let bytes = serde_json::to_vec(boot).map_err(error)?;
        if bytes.len() > vm_boot::MAX_BYTES {
            return Err(error("VM secret bootstrap exceeds its fixed bound"));
        }
        let listener_path = PathBuf::from(format!("{}_{port}", channel.display(), port = vm_boot::PORT));
        let listener = UnixListener::bind(&listener_path).map_err(error)?;
        let setup = (|| {
            fs::set_permissions(&listener_path, fs::Permissions::from_mode(0o600))?;
            let serial = OpenOptions::new()
                .append(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(serial)?;
            let mut command = Command::new(launcher);
            command
                .arg(vm_monitor::MODE)
                .env_clear()
                .stdin(Stdio::piped())
                .stdout(serial.try_clone()?)
                .stderr(serial)
                .kill_on_drop(true);
            command.spawn()
        })();
        let child = match setup {
            Ok(child) => child,
            Err(problem) => {
                let _ = fs::remove_file(&listener_path);
                return Err(error(problem));
            }
        };
        let bootstrap = tokio::spawn(async move {
            tokio::time::timeout_at(deadline, async {
                let (mut socket, _) = listener.accept().await?;
                socket.write_all(&bytes).await?;
                socket.shutdown().await
            })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "VM bootstrap was not received"))?
        });
        let mut machine = Self { child, bootstrap: Some(bootstrap), listener: listener_path };
        let bytes = serde_json::to_vec(plan).map_err(error)?;
        let mut input = machine.child.stdin.take().ok_or_else(|| error("Missing VM launcher pipe"))?;
        tokio::time::timeout_at(deadline, async {
            input.write_all(&bytes).await?;
            input.shutdown().await
        })
        .await
        .map_err(|_| RuntimeError::Timeout("VM launcher exceeded its deadline".into()))?
        .map_err(error)?;
        drop(input);
        Ok(machine)
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Complete the one-shot delivery independently of liveness. A fast guest job may
    /// already have a signed completion; delivery alone never certifies a running guest.
    pub async fn delivered(&mut self, deadline: Instant) -> Result<(), RuntimeError> {
        if let Some(mut sender) = self.bootstrap.take() {
            match tokio::time::timeout_at(deadline, &mut sender).await {
                Ok(delivered) => delivered.map_err(error)?.map_err(error)?,
                Err(_) => {
                    sender.abort();
                    let _ = sender.await;
                    return Err(RuntimeError::Timeout("VM bootstrap delivery exceeded its deadline".into()));
                }
            }
            fs::remove_file(&self.listener).map_err(error)?;
        }
        Ok(())
    }

    /// Require completed secret delivery and a live monitor after authenticated readiness.
    pub async fn bootstrapped(&mut self, deadline: Instant) -> Result<(), RuntimeError> {
        self.delivered(deadline).await?;
        if self.child.try_wait().map_err(error)?.is_some() {
            return Err(error("VM monitor exited during bootstrap"));
        }
        Ok(())
    }

    pub fn exited(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait().await
    }

    /// Child owns the process identity; this never signals a reused numeric PID. The
    /// caller must observe exit and acquire the lifetime lease before any disk mutation.
    pub async fn kill(&mut self) -> io::Result<()> {
        self.child.kill().await
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        if let Some(sender) = self.bootstrap.take() {
            sender.abort();
        }
        let _ = fs::remove_file(&self.listener);
        // tokio Child's kill_on_drop retires the owned monitor. Its kernel lease stays
        // held until actual process exit; dropping this object never certifies disk safety.
    }
}
