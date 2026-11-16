// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! pryxd's background work. Events from the runtime are hints that make observation quick;
//! the periodic resync is what makes it correct, since an event stream can drop (a daemon
//! restart ends it) and nothing may depend on having seen every event. It's the same rule as
//! the control plane's realtime layer: an event says something may have changed; the reader asks for the
//! truth.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::manager::{Manager, Records};

/// Follows runtime events until cancelled, reconnecting with backoff; a gap in the stream is
/// closed by a full resync once it's back.
pub async fn follow_events(manager: Arc<Manager>, cancel: CancellationToken) {
    let mut backoff = Duration::from_millis(500);
    loop {
        let mut events = manager.runtime.events(&manager.owner_labels());
        let mut healthy = false;
        loop {
            let next = tokio::select! {
                next = events.next() => next,
                () = cancel.cancelled() => return,
            };
            match next {
                Some(Ok(event)) => {
                    if !healthy {
                        healthy = true;
                        backoff = Duration::from_millis(500);
                    }
                    manager.on_event(&event.container_id, &event.action, event.exit_code, event.at).await;
                }
                Some(Err(e)) => {
                    warn!(error = %e, "runtime event stream failed");
                    break;
                }
                None => {
                    warn!("runtime event stream ended");
                    break;
                }
            }
        }
        tokio::select! {
            () = tokio::time::sleep(backoff) => {}
            () = cancel.cancelled() => return,
        }
        // Short: after an outage the first look decides when pryxd is trustworthy again.
        backoff = (backoff * 2).min(Duration::from_secs(5));
        let report = manager.reconcile(Records::InMemory).await;
        if report.error.is_none() {
            info!(workloads = report.workloads, "resynced after the event stream came back");
        }
    }
}

/// Periodic work: resync (and the fences it finds unapplied), stats, disk usage, trash.
/// Resync is polled independently of optional measurements: an engine whose stats hang must
/// still lose its advertised guarantees. Cancellation drops both loops and their pending reads.
pub async fn periodic(manager: Arc<Manager>, cancel: CancellationToken) {
    let intervals = manager.config.intervals.clone();
    let mut resync = tokio::time::interval(Duration::from_secs(intervals.resync_seconds.max(5)));
    let mut stats = tokio::time::interval(Duration::from_secs(intervals.stats_seconds.max(5)));
    let mut disk = tokio::time::interval(Duration::from_secs(intervals.disk_usage_seconds.max(30)));
    let mut trash = tokio::time::interval(Duration::from_hours(1));
    for timer in [&mut resync, &mut stats, &mut disk, &mut trash] {
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    }
    // The startup pass already ran; defer resync and stats. Disk and trash retain their first tick.
    resync.tick().await;
    stats.tick().await;
    let synchronization = async {
        loop {
            resync.tick().await;
            let was_reconciled = manager.reconciled();
            let report = manager.reconcile(Records::InMemory).await;
            if let Some(error) = report.error {
                warn!(%error, "resync failed");
                continue;
            }
            if !was_reconciled {
                info!(workloads = report.workloads, adopted = report.adopted, "first successful reconciliation");
            }
            // The pass saw what runs now: a superseded copy it saw running is fenced again.
            manager.reapply_fences();
        }
    };
    let measurements = async {
        loop {
            tokio::select! {
                _ = stats.tick() => manager.sample_all().await,
                _ = disk.tick() => manager.measure_disk().await,
                _ = trash.tick() => {
                    let retention = manager.config.workloads.trash_retention();
                    let store = manager.store.clone();
                    let now = crate::clock::now().unix_timestamp();
                    if let Ok(purged) = tokio::task::spawn_blocking(move || store.purge_trash(now, retention)).await {
                        for path in purged {
                            info!(path = %path.display(), "purged from trash after retention");
                        }
                    }
                }
            }
        }
    };
    tokio::select! {
        () = cancel.cancelled() => {}
        () = async { tokio::join!(synchronization, measurements); } => {}
    }
}
