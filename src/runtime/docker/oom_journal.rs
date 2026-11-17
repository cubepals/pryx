// SPDX-License-Identifier: FSL-1.1-ALv2
//! systemd preserves kernel OOM counters before removing a Docker scope. Bind that receipt to
//! exactly one engine start using journal synchronization and monotonic processing timestamps.
//! Docker's flags/events remain independent evidence; a signal exit alone never establishes OOM.
//!
//! Parts (`oom_journal/`):
//! - `reader.rs`: native journal authority, bounded processes and fixed record validation.
//! - `receipt.rs`: private durable pending/bound generations and cross-instance locks.
//! - `tests.rs`: causal barrier, restart, authority and late-evidence regressions.
use super::{ContainerInfo, ContainerStatus, DockerRuntime, RuntimeError, quick};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
mod reader;
mod receipt;
#[cfg(test)]
mod tests;

pub(super) fn failure(reason: &str) -> RuntimeError {
    RuntimeError::Unavailable(format!("Docker OOM evidence is unavailable: {reason}"))
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn boot() -> Result<String, RuntimeError> {
    crate::host::boot_id()
        .map(|id| id.replace('-', ""))
        .filter(|id| valid_hex(id, 32))
        .ok_or_else(|| failure("host boot identity is unavailable"))
}

fn monotonic() -> Result<u64, RuntimeError> {
    let time = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    u64::try_from(time.tv_sec)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1_000_000))
        .and_then(|micros| micros.checked_add(u64::try_from(time.tv_nsec).ok()? / 1000))
        .ok_or_else(|| failure("host monotonic clock is unavailable"))
}

fn started(info: &ContainerInfo) -> Option<String> {
    info.started_at.and_then(|time| time.format(&time::format_description::well_known::Rfc3339).ok())
}

pub(super) struct Context {
    state: PathBuf,
    source: Arc<dyn reader::Source>,
}

impl DockerRuntime {
    pub(super) fn journal_context(&self) -> Option<&Context> {
        self.oom_journal.as_ref().filter(|_| self.info.lock().unwrap().as_ref().is_none_or(|info| !info.podman))
    }

    /// The daemon's private state root, shared by reconstructed backends on this host.
    pub fn with_oom_journal(mut self, state: &Path) -> Self {
        let state = if state.is_absolute() {
            state.into()
        } else {
            std::env::current_dir().map(|directory| directory.join(state)).unwrap_or_else(|_| state.into())
        };
        self.oom_journal = Some(Context { state, source: Arc::new(reader::Native) });
        self
    }
}

impl Context {
    pub(super) async fn available(&self) -> Result<(), RuntimeError> {
        self.source.available().await
    }

    pub(super) async fn begin(&self, info: &ContainerInfo) -> Result<receipt::Held, RuntimeError> {
        let mut held = self.acquire(&info.id).await?;
        // Tombstone the old generation before every mutation, including optional capture
        // failure. Realtime can reuse StartedAt; a crash must not expose the old UUID again.
        if held.receipt.pending.is_some() {
            held.receipt.bound = None;
        }
        held.receipt.pending = Some(receipt::Pending { prior_started: started(info), boot: None, begin: None });
        held.save()?;
        if self.available().await.is_err() || reader::sync(&self.source).await.is_err() {
            return Ok(held);
        }
        let (Ok(boot), Ok(begin)) = (boot(), monotonic()) else { return Ok(held) };
        held.receipt.pending =
            Some(receipt::Pending { prior_started: started(info), boot: Some(boot), begin: Some(begin) });
        held.save()?;
        Ok(held)
    }

    pub(super) async fn finish(
        &self,
        runtime: &DockerRuntime,
        held: &mut receipt::Held,
        before: &ContainerInfo,
        info: &ContainerInfo,
        outcome: &Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        let same_state = before.started_at == info.started_at
            && before.finished_at == info.finished_at
            && before.status == info.status;
        let refused = matches!(
            outcome,
            Err(RuntimeError::NotFound(_) | RuntimeError::Conflict(_) | RuntimeError::PortTaken { .. })
        );
        if refused && same_state {
            // A definite engine refusal with unchanged owner state preserves the old proof.
            // Success, timeout and transport failure never prove that a mutation was absent.
            held.receipt.pending = None;
            return held.save();
        }
        held.receipt.bound = None;
        held.save()?;
        let Some(pending) = &held.receipt.pending else { return Ok(()) };
        let current = started(info);
        let (Some(pending_boot), Some(begin)) = (&pending.boot, pending.begin) else {
            held.receipt.pending = None;
            return held.save();
        };
        if current == pending.prior_started {
            // A successful new start whose realtime value was reused cannot inherit the old
            // invocation, and is not eligible for a new binding without a new StartedAt.
            held.receipt.pending = None;
            return held.save();
        }
        let current = current.ok_or_else(|| failure("new Docker start identity is unavailable"))?;
        if *pending_boot != boot()? {
            return Err(failure("pending start belongs to another boot"));
        }
        reader::sync(&self.source).await?;
        let starts = reader::query(&self.source, pending_boot, &info.id, reader::STARTED, None).await?;
        let mut fresh = starts.into_iter().filter(|record| record.monotonic > begin);
        let selected = fresh.next().ok_or_else(|| failure("current scope start receipt is missing"))?;
        if fresh.next().is_some() {
            return Err(failure("multiple scope starts followed the start barrier"));
        }
        let actual =
            quick("confirm scope invocation owner", runtime.docker()?.inspect_container(&info.id, None)).await?;
        let actual = super::container::info_from(actual);
        if actual.id != info.id || actual.started_at != info.started_at {
            return Err(failure("Docker start changed while binding scope invocation"));
        }
        held.receipt.bound =
            Some(receipt::Bound { started: current, boot: pending_boot.clone(), invocation: selected.invocation });
        held.receipt.pending = None;
        held.save()
    }

    async fn acquire(&self, id: &str) -> Result<receipt::Held, RuntimeError> {
        // Only lock acquisition is retried. The engine mutation is always issued once.
        for attempt in 0..20 {
            match receipt::Held::acquire(&self.state, id) {
                Err(RuntimeError::Conflict(_)) if attempt < 19 => {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                result => return result,
            }
        }
        unreachable!("bounded receipt acquisition always returns")
    }

    pub(super) async fn retire(&self, id: &str) -> Result<(), RuntimeError> {
        for attempt in 0..100 {
            match receipt::retire(&self.state, id) {
                Err(RuntimeError::Conflict(_)) if attempt < 99 => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                result => return result,
            }
        }
        unreachable!("bounded receipt retirement always returns")
    }

    pub(super) async fn inspect(&self, runtime: &DockerRuntime, info: &mut ContainerInfo) -> Result<(), RuntimeError> {
        if info.oom_killed || !matches!(info.status, ContainerStatus::Exited | ContainerStatus::Dead) {
            return Ok(());
        }
        let held = self.acquire(&info.id).await?;
        if held.receipt.pending.is_some() {
            return Ok(());
        }
        let Some(bound) = &held.receipt.bound else { return Ok(()) };
        if Some(&bound.started) != started(info).as_ref() || bound.boot != boot()? {
            return Ok(());
        }
        reader::sync(&self.source).await?;
        let mut evidence =
            reader::query(&self.source, &bound.boot, &info.id, reader::OOM, Some(&bound.invocation)).await?;
        if info.exit_code == crate::runtime::exit_by(libc::SIGKILL) {
            for delay in [100, 300, 600] {
                if !evidence.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                reader::sync(&self.source).await?;
                evidence =
                    reader::query(&self.source, &bound.boot, &info.id, reader::OOM, Some(&bound.invocation)).await?;
            }
        }
        // Do not cache negatives: PID 1 can publish the kernel receipt after FinishedAt and
        // after this bounded read. No wall-clock comparison participates in attribution.
        if !evidence.is_empty() {
            let actual =
                quick("confirm journal OOM owner", runtime.docker()?.inspect_container(&info.id, None)).await?;
            let actual = super::container::info_from(actual);
            if actual.id != info.id || actual.started_at != info.started_at {
                return Err(failure("Docker start changed while observing the OOM receipt"));
            }
            info.oom_killed = true;
        }
        Ok(())
    }
}
