// SPDX-License-Identifier: FSL-1.1-ALv2
//! Stop the local installation's retained compute before removing its state. The daemon
//! must have released its store lock; unrelated containers are never adopted or signalled.
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, ensure};

use crate::config::Config;
use crate::labels::{LABEL_DEPLOYMENT, LABEL_WORKLOAD};
use crate::runtime::{ContainerRuntime, configured};
use crate::store::Store;

pub fn command(path: &Path, purge: bool) -> anyhow::Result<()> {
    let config = Config::load(path)?;
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(stop(&config, purge))
}

async fn stop(config: &Config, purge: bool) -> anyhow::Result<()> {
    let store = Store::existing(&config.state_dir);
    let _lock = store.lock().context("stop the local node before removing its compute")?;
    let (records, bad) = store.load_all()?;
    ensure!(bad.is_empty(), "unreadable workload records; keep state and repair them before purging");
    let runtime = configured::create(config).await?;
    // Validate the complete set before any mutation. A reused name is never authority.
    let mut owned = Vec::new();
    for record in records {
        let Some(info) = runtime.inspect(&record.container_name).await? else { continue };
        ensure!(
            info.labels.get(LABEL_DEPLOYMENT) == Some(&config.deployment_id)
                && info.labels.get(LABEL_WORKLOAD) == Some(&record.id.to_string())
                && record.container_id.as_ref().is_none_or(|id| id == &info.id),
            "workload {} has a different runtime identity; no state was removed",
            record.id
        );
        owned.push((info.id, record.spec.stop));
    }
    stop_owned(runtime.as_ref(), owned, purge).await
}

async fn stop_owned(
    runtime: &dyn ContainerRuntime,
    owned: Vec<(String, crate::protocol::StopSpec)>,
    purge: bool,
) -> anyhow::Result<()> {
    for (id, stop) in owned {
        runtime.disable_restart(&id).await?;
        runtime.stop(&id, stop.signal.as_str(), Duration::from_secs(u64::from(stop.timeout_seconds))).await?;
        if purge {
            runtime.remove(&id).await?;
        }
    }
    Ok(())
}
