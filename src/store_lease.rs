// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Opt-in object-store authority with an independent cgroup watchdog. No ownership decisions live in the HTTP client.
//!
//! Parts (`store_lease/`):
//! - `client.rs`: bounded GET/conditional PUT, store time and conservative boot-clock deadlines.
//! - `guard.rs`: protected watchdog process and cgroup freeze/kill, independent of Docker and Tokio.
use crate::protocol::StoreLeaseGrant;
use std::sync::{
    Arc, Mutex, RwLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;
pub(crate) mod client;
pub mod guard;

pub(crate) struct Held {
    pub(crate) epoch: u64,
    pub(crate) grant: RwLock<StoreLeaseGrant>,
    deadline: AtomicU64,
    terminal: AtomicBool,
    guard: Mutex<guard::Guard>,
}
impl Held {
    pub(crate) async fn start(id: &str, epoch: u64, node: &str, grant: StoreLeaseGrant) -> Result<Arc<Self>, String> {
        let deadline = client::renew(id, epoch, node, &grant, false).await?;
        let guard = guard::Guard::start(id, epoch, deadline).map_err(|_| "Cannot start independent cgroup watchdog")?;
        let held = Arc::new(Self {
            epoch,
            grant: RwLock::new(grant),
            deadline: AtomicU64::new(deadline),
            terminal: AtomicBool::new(false),
            guard: Mutex::new(guard),
        });
        let monitor = Arc::downgrade(&held);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(guard::STEP).await;
                let Some(held) = monitor.upgrade() else { return };
                if !held.live() {
                    held.expire();
                    return;
                }
            }
        });
        let weak = Arc::downgrade(&held);
        let id = id.to_owned();
        let node = node.to_owned();
        tokio::spawn(async move {
            loop {
                let Some(held) = weak.upgrade() else { return };
                let grant = held.grant.read().unwrap().clone();
                let delay = Duration::from_secs(grant.renew_seconds);
                drop(held);
                tokio::time::sleep(delay).await;
                let Some(held) = weak.upgrade() else { return };
                if !held.live() {
                    held.expire();
                    return;
                }
                match client::renew(&id, epoch, &node, &grant, true).await {
                    Ok(deadline) if held.live() => {
                        if held.guard.lock().unwrap().renew(deadline).is_err() {
                            held.expire();
                            return;
                        }
                        held.deadline.store(deadline, Ordering::Release);
                    }
                    Ok(_) => {
                        held.expire();
                        return;
                    }
                    Err(_) if !held.live() => {
                        held.expire();
                        return;
                    }
                    Err(_) => {} // Keep the old deadline; a failed request grants no time.
                }
            }
        });
        Ok(held)
    }
    pub(crate) fn live(&self) -> bool {
        !self.terminal.load(Ordering::Acquire)
            && guard::boot_ms().is_ok_and(|now| now < self.deadline.load(Ordering::Acquire))
            && self.guard.lock().unwrap().alive()
    }
    pub(crate) fn expire(&self) {
        self.terminal.store(true, Ordering::Release);
        let _ = self.guard.lock().unwrap().fence();
    }
}

/// Refuse an accidental policy downgrade while this host may retain leased execution/data.
/// Operators deactivate it only after draining/resting all copies and waiting out published terms.
pub fn check_policy(state_dir: &std::path::Path, enabled: bool) -> std::io::Result<()> {
    let marker = state_dir.join("store-lease-mode-v1");
    if !enabled && marker.exists() {
        return Err(std::io::Error::other(
            "store-lease policy is latched on this host; drain/rest copies and wait out lease terms before removing store-lease-mode-v1 to disable it",
        ));
    }
    if enabled {
        crate::durable::write_atomic(&marker, b"enabled\n", 0o600).map_err(std::io::Error::other)?;
    }
    Ok(())
}
#[cfg(test)]
mod policy_tests {
    #[test]
    fn feature_off_hosts_are_untouched_and_enabled_policy_cannot_silently_downgrade() {
        let dir = tempfile::tempdir().unwrap();
        super::check_policy(dir.path(), false).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        super::check_policy(dir.path(), true).unwrap();
        assert!(super::check_policy(dir.path(), false).is_err());
    }
}
