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
    let owned = owned_compute(config, runtime.as_ref(), records).await?;
    stop_owned(runtime.as_ref(), owned, purge).await?;
    if purge {
        unseal_native_tools(&config.state_dir).context("preparing the stopped native cache for purge")?;
    }
    Ok(())
}

fn unseal_native_tools(state: &Path) -> std::io::Result<()> {
    let root = state.join("process-tools");
    let directory = match pryx_agent::paths::DataDir::open(&root) {
        Ok(directory) => directory,
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(problem) => return Err(problem),
    };
    // All compute has stopped while both lifecycle/store locks remain held. Reuse the
    // descriptor-relative walk and chmod owner; links never grant access to their targets.
    crate::tree::walk(&root, &[], &mut |entry| {
        if matches!(entry.kind, crate::tree::Kind::Dir) {
            directory.chmod(entry.path.to_str().ok_or(std::io::ErrorKind::InvalidData)?, 0o700)?;
        }
        Ok(())
    })?;
    Ok(())
}

async fn owned_compute(
    config: &Config,
    runtime: &dyn ContainerRuntime,
    records: Vec<crate::store::WorkloadRecord>,
) -> anyhow::Result<Vec<(String, crate::protocol::StopSpec)>> {
    // Validate the complete set before any mutation. A reused name is never authority.
    let mut owned = Vec::new();
    for record in records {
        let tier = record
            .isolation
            .map(crate::protocol::Isolation::as_str)
            .or_else(|| (!runtime.multi_tier()).then_some(runtime.isolation()))
            .context("legacy workload tier is unresolved; restore its previous explicit runtime before cleanup")?;
        ensure!(
            runtime.tiers().contains(&tier),
            "{tier} tier is unavailable; state was kept and no compute was stopped"
        );
        runtime.bind(&record.container_name, record.container_id.as_deref(), tier)?;
        let Some(info) = runtime.inspect(&record.container_name).await? else { continue };
        ensure!(
            info.labels.get(LABEL_DEPLOYMENT) == Some(&config.deployment_id)
                && info.labels.get(LABEL_WORKLOAD) == Some(&record.id.to_string())
                && record.container_id.as_ref().is_none_or(|id| id == &info.id),
            "workload {} has a different runtime identity; no state was removed",
            record.id
        );
        runtime.bind(&record.container_name, Some(&info.id), tier)?;
        owned.push((info.id, record.spec.stop));
    }
    Ok(owned)
}

async fn stop_owned(
    runtime: &dyn ContainerRuntime,
    owned: Vec<(String, crate::protocol::StopSpec)>,
    purge: bool,
) -> anyhow::Result<()> {
    for (id, stop) in owned {
        runtime.disable_restart(&id).await.context("disabling engine restarts for owned compute")?;
        runtime
            .stop(&id, stop.signal.as_str(), Duration::from_secs(u64::from(stop.timeout_seconds)))
            .await
            .context("stopping owned compute")?;
        if purge {
            runtime.remove(&id).await.context("removing owned compute")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Isolation;
    use crate::runtime::{fake::FakeRuntime, router::Router};
    use std::sync::Arc;

    #[test]
    fn sealed_cache_purge_never_changes_link_targets() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let state = tempfile::tempdir().unwrap();
        unseal_native_tools(state.path()).unwrap();
        assert!(!state.path().join("process-tools").exists());
        let cache = state.path().join("process-tools/uid/cache/digest/nested");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("verified"), b"unchanged upstream bytes").unwrap();
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            state.path().join("process-tools/temporary-pipe"),
            rustix::fs::Mode::from_raw_mode(0o600),
        )
        .unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("kept"), b"keep these bytes").unwrap();
        symlink(outside.path(), cache.join("outside-link")).unwrap();
        for path in [cache.as_path(), cache.parent().unwrap(), outside.path()] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        unseal_native_tools(state.path()).unwrap();
        assert_eq!(std::fs::metadata(outside.path()).unwrap().permissions().mode() & 0o777, 0o555);
        std::fs::remove_dir_all(state.path().join("process-tools")).unwrap();
        assert_eq!(std::fs::read(outside.path().join("kept")).unwrap(), b"keep these bytes");
        symlink(outside.path(), state.path().join("process-tools")).unwrap();
        assert!(unseal_native_tools(state.path()).is_err());
        assert_eq!(std::fs::metadata(outside.path()).unwrap().permissions().mode() & 0o777, 0o555);
        std::fs::set_permissions(outside.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[tokio::test]
    async fn cleanup_binds_the_observed_id_when_creation_did_not_record_it() {
        use crate::{
            ids::WorkloadId,
            manager::{Manager, Precondition, Records},
            metrics::Metrics,
        };
        let directory = tempfile::tempdir().unwrap();
        let (uid, gid) = (rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw());
        let config: Config = toml::from_str(&format!(
            "deployment_id = 'test'\nnode_id = 'node'\nstate_dir = '{}'\n[api]\nlisten = '127.0.0.1:0'\n\
             [workloads]\ndata_owner = '{uid}:{gid}'\n[capacity]\nmin_free_disk_mb = 1\n",
            directory.path().join("state").display()
        ))
        .unwrap();
        let store = Store::open(&config.state_dir).unwrap();
        let process = Arc::new(FakeRuntime::new().with_isolation(Isolation::Process));
        let manager = Manager::new(
            Arc::new(config.clone()),
            process.clone(),
            store.clone(),
            Arc::new(Metrics::new()),
            Arc::new(|_, _| true),
        );
        assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
        let spec = serde_json::from_value(serde_json::json!({
            "isolation":"process", "image":"", "resources":{"memoryMb":256},
            "storage":{"mountPath":"/data","sizeGb":1}
        }))
        .unwrap();
        manager.ensure(WorkloadId::parse("created").unwrap(), spec, Precondition::None, None).await.unwrap();
        let mut record = store.load_all().unwrap().0.pop().unwrap();
        record.container_id = None;
        drop(manager);
        let runtime = Router::new(vec![process.clone()]).unwrap();
        let owned = owned_compute(&config, &runtime, vec![record.clone()]).await.unwrap();
        assert_eq!(owned.len(), 1);
        let observed = process.container(&record.container_name).unwrap();
        assert_eq!(owned[0].0, observed.id);
        assert_eq!(runtime.bound_isolation(&observed.id), Some(Isolation::Process.as_str()));
        assert_eq!(process.count_calls("disable_restart "), 0, "ownership validation is read-only");
        assert_eq!(process.count_calls("stop "), 0);
    }

    #[tokio::test]
    async fn auto_cleanup_refuses_unavailable_or_unresolved_record_tiers_and_preserves_data() {
        let directory = tempfile::tempdir().unwrap();
        let config: Config = toml::from_str(&format!(
            "runtime = 'auto'\ndeployment_id = 'test'\nnode_id = 'node'\nstate_dir = '{}'\n\
             [api]\nlisten = '127.0.0.1:0'\n",
            directory.path().join("state").display()
        ))
        .unwrap();
        let process = Arc::new(FakeRuntime::new().with_isolation(Isolation::Process));
        let runtime = Router::new(vec![process.clone()]).unwrap();
        let store = Store::open(&config.state_dir).unwrap();
        let mut record = crate::store::tests::record("kept");
        store
            .ensure_data_dir(&record.id, (rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw()))
            .unwrap();
        let marker = store.data_dir(&record.id).join("marker");
        std::fs::write(&marker, b"keep these bytes").unwrap();
        for (tier, reason) in [(Some(Isolation::Container), "container"), (None, "unresolved")] {
            record.isolation = tier;
            store.save(&record).unwrap();
            let error = owned_compute(&config, &runtime, vec![record.clone()]).await.unwrap_err();
            assert!(format!("{error:#}").contains(reason), "{error:#}");
            assert_eq!(std::fs::read(&marker).unwrap(), b"keep these bytes");
            assert_eq!(store.load_all().unwrap().0, vec![record.clone()]);
            assert!(process.calls.lock().unwrap().is_empty(), "no read or mutation may target another tier");
        }
    }
}
