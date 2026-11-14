// SPDX-License-Identifier: FSL-1.1-ALv2
//! Runtime verbs keep transitions owned across caller cancellation. A monitor process is
//! observed directly; only a verified boot-scoped guest proof certifies an orderly disk stop.
use super::*;
use crate::runtime::*;
use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use pryx_agent::{vm_observation, vm_shutdown};
use tokio::time::Instant;

impl VmRuntime {
    pub(super) async fn stop_entry(
        &self,
        name: &str,
        signal: i32,
        grace: Duration,
        force: bool,
    ) -> Result<(), RuntimeError> {
        let mut entry = self.entry(name)?;
        let transition = self.transition(&entry.id);
        let _transition = transition.lock().await;
        let live = self.state.live.lock().unwrap().get(&entry.id).cloned();
        let Some(live) = live else { return Ok(()) };
        let mut live = live.lock().await;
        let deadline = Instant::now() + grace;
        let already_exited = live.monitor.exited().map_err(error)?.is_some();
        let mut forced = force && !already_exited;
        if !force && !already_exited && !live.shutdown_requested {
            live.shutdown_requested = true;
            let boot = entry.spec.agent.as_ref().ok_or_else(|| error("VM agent bootstrap is missing"))?;
            let request = vm_shutdown::request_for_signal(
                &agent::key(&entry.spec)?,
                &live.nonce,
                &boot.sandbox,
                boot.epoch,
                signal,
            )
            .map_err(error)?;
            let bundle = self.bundle().await?;
            // Submit once. A lost acknowledgement cannot cancel an accepted guest shutdown.
            let _ = super::super::transport::request(
                &bundle.root().join("bin/vm-transport"),
                &serde_json::json!({"socket":live.channel,"port":vm_shutdown::PORT,"shutdown":request}),
                deadline.saturating_duration_since(Instant::now()),
                pryx_agent::vm_boot::MAX_BYTES,
            )
            .await;
        }
        let exited = if already_exited {
            true
        } else if force {
            false
        } else {
            match tokio::time::timeout_at(deadline, live.monitor.wait()).await {
                Ok(status) => {
                    status.map_err(error)?;
                    true
                }
                Err(_) => false,
            }
        };
        if !exited && live.monitor.exited().map_err(error)?.is_none() {
            forced = true;
            if !live.killing {
                live.killing = true;
                tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, live.monitor.kill())
                    .await
                    .map_err(|_| {
                        RuntimeError::Timeout(
                            "VM monitor termination is unconfirmed; its disk lease stays authoritative".into(),
                        )
                    })?
                    .map_err(error)?;
            } else {
                tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, live.monitor.wait())
                    .await
                    .map_err(|_| RuntimeError::Timeout("VM monitor termination remains unconfirmed".into()))?
                    .map_err(error)?;
            }
        }
        live.forwarder.stop().await.map_err(error)?;
        let capture_deadline = if forced { Instant::now() + crate::runtime::ENGINE_PROBE_TIMEOUT } else { deadline };
        let capture = tokio::time::timeout_at(capture_deadline, live.monitor.finish_serial())
            .await
            .map_err(|_| RuntimeError::Timeout("VM diagnostics did not finish after monitor exit".into()))
            .and_then(|result| result.map_err(error));
        let code = if forced { Ok(crate::runtime::exit_by(libc::SIGKILL)) } else { verified_exit(&entry, &live) };
        drop(live);
        self.state.live.lock().unwrap().remove(&entry.id);
        entry.code = code.as_ref().copied().unwrap_or(UNKNOWN_EXIT_CODE);
        entry.finished = Some(crate::clock::now());
        if let Some(snapshot) = &mut entry.agent_state {
            snapshot.activity.streams = 0;
        }
        self.save_entry(entry)?;
        capture.and(code.map(|_| ()))
    }
    pub(super) async fn observe(&self, name: &str) -> Result<Option<ContainerInfo>, RuntimeError> {
        let entry = match self.entry(name) {
            Ok(entry) => entry,
            Err(RuntimeError::NotFound(_)) => return Ok(None),
            Err(problem) => return Err(problem),
        };
        let transition = self.transition(&entry.id);
        let _transition = transition.lock().await;
        let mut entry = match self.entry(name) {
            Ok(entry) => entry,
            Err(RuntimeError::NotFound(_)) => return Ok(None),
            Err(problem) => return Err(problem),
        };
        let live = self.state.live.lock().unwrap().get(&entry.id).cloned();
        let status = if let Some(live) = live {
            let mut live = live.lock().await;
            if live.monitor.exited().map_err(error)?.is_none() {
                if live.forwarder.exited().map_err(error)?.is_some() {
                    return Err(RuntimeError::Unavailable("VM forwarding process exited".into()));
                }
                ContainerStatus::Running
            } else {
                live.forwarder.stop().await.map_err(error)?;
                let result = tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, live.monitor.finish_serial())
                    .await
                    .map_err(|_| RuntimeError::Timeout("VM diagnostics did not finish after monitor exit".into()))
                    .and_then(|result| result.map_err(error))
                    .and_then(|_| verified_exit(&entry, &live));
                entry.code = result.as_ref().copied().unwrap_or(UNKNOWN_EXIT_CODE);
                entry.finished = Some(crate::clock::now());
                drop(live);
                self.state.live.lock().unwrap().remove(&entry.id);
                self.save_entry(entry.clone())?;
                if result.is_ok() { ContainerStatus::Exited } else { ContainerStatus::Dead }
            }
        } else if entry.started.is_some() {
            ContainerStatus::Exited
        } else {
            ContainerStatus::Created
        };
        if status == ContainerStatus::Running {
            if let Ok(snapshot) = agent::snapshot(&entry.spec).await {
                entry.agent_state = Some(snapshot);
            }
        } else if let Some(snapshot) = &mut entry.agent_state {
            snapshot.activity.streams = 0;
        }
        self.state.entries.lock().unwrap().insert(entry.id.clone(), entry.clone());
        Ok(Some(ContainerInfo {
            agent_state: entry.agent_state,
            memory_enforced: Some(true),
            cpu_enforced: Some(true),
            cgroup_parent: None,
            id: entry.id,
            name: entry.spec.name,
            labels: entry.spec.labels,
            env: entry.spec.env.into_iter().collect(),
            status,
            exit_code: entry.code,
            oom_killed: false,
            started_at: entry.started,
            finished_at: entry.finished,
            restart_count: 0,
            health: None,
        }))
    }
}
pub(super) fn verified_exit(entry: &Entry, live: &Live) -> Result<i64, RuntimeError> {
    let boot = entry.spec.agent.as_ref().ok_or_else(|| error("VM agent bootstrap is missing"))?;
    let public_key = agent::key(&entry.spec)?.public_key();
    let proof = shutdown_proof(&live.serial, &public_key, &live.nonce, &boot.sandbox, boot.epoch)?;
    match (proof.code, proof.signal) {
        (Some(code), _) => Ok(i64::from(code)),
        (_, Some(signal)) => Ok(crate::runtime::exit_by(signal)),
        _ => Err(error("VM shutdown omitted its guest exit status")),
    }
}

fn shutdown_proof(
    path: &Path,
    key: &str,
    nonce: &str,
    sandbox: &str,
    epoch: u64,
) -> Result<vm_observation::Observation, RuntimeError> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file =
        OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).map_err(error)?;
    let bound = pryx_agent::contract::MAX_REQUEST_BYTES as u64;
    let size = file.metadata().map_err(error)?.len();
    file.seek(SeekFrom::Start(size.saturating_sub(bound))).map_err(error)?;
    let mut bytes = Vec::new();
    file.take(bound + 1).read_to_end(&mut bytes).map_err(error)?;
    for line in bytes.split(|b| *b == b'\n').rev() {
        if let Some(index) =
            line.windows(vm_observation::PREFIX.len()).position(|s| s == vm_observation::PREFIX.as_bytes())
            && let Ok(proof) =
                serde_json::from_slice::<vm_observation::Proof>(&line[index + vm_observation::PREFIX.len()..])
            && vm_observation::verify(&proof, key, nonce, sandbox, epoch)
        {
            return Ok(proof.observation);
        }
    }
    Err(error("VM exit has no valid boot-scoped clean-disk proof"))
}
#[async_trait]
impl ContainerRuntime for VmRuntime {
    fn isolation(&self) -> &'static str {
        "vm"
    }
    fn uses_host_ports(&self) -> bool {
        true
    }
    fn uses_remote_data(&self) -> bool {
        true
    }
    async fn data_service(
        &self,
        request: crate::runtime::files::Request,
    ) -> Result<crate::runtime::files::Service, RuntimeError> {
        let own = self.clone();
        tokio::spawn(async move { own.serve_files(request).await }).await.map_err(error)?
    }
    fn supports_data_exclude(&self) -> bool {
        false
    }
    fn requires_stopped_data_copy(&self) -> bool {
        true
    }
    async fn data_copy_lease(&self, sandbox: &str, data: &Path) -> Result<Box<dyn Send>, RuntimeError> {
        let lease = self
            .state
            .root
            .join("leases")
            .join(format!("{}.lock", super::super::filesystems::sha256(sandbox.as_bytes()).map_err(error)?));
        let path = data.join(super::super::disk::FILE);
        let exists = match fs::symlink_metadata(&path) {
            Ok(meta) => Some(meta.len()),
            Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => None,
            Err(problem) => return Err(error(problem)),
        };
        if let Some(bytes) = exists {
            let bundle = self.bundle().await?;
            // Recovery changes only a staged complete copy; the original is atomically replaced.
            super::super::disk::prepare(
                bundle.filesystems(),
                data,
                &lease,
                bytes,
                self.state.settings.image_timeout,
                self.state.settings.free_floor_bytes,
            )
            .await?;
            let held = lock(&lease)?;
            super::super::disk::verify(bundle.filesystems(), &path, self.state.settings.image_timeout).await?;
            Ok(Box::new(held))
        } else {
            let held = lock(&lease)?;
            if fs::read_dir(data).map_err(error)?.next().is_some() {
                return Err(error("VM data directory contains an unrecognized file"));
            }
            Ok(Box::new(held))
        }
    }
    fn supports_agent(&self) -> bool {
        true
    }
    fn supports_network_policy(&self) -> bool {
        false
    }
    fn data_owner(&self, _: &str) -> Result<Option<(u32, u32)>, RuntimeError> {
        Ok(Some((rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw())))
    }
    async fn info(&self) -> Result<RuntimeInfo, RuntimeError> {
        Self::kvm()?;
        Ok(RuntimeInfo {
            rootless: true,
            memory_limit: true,
            cpu_limit: true,
            pids_limit: true,
            version: Some(env!("CARGO_PKG_VERSION").into()),
            storage_driver: Some("vm".into()),
            security_options: vec!["own-kernel".into(), "landlock".into(), "seccomp".into()],
            ..Default::default()
        })
    }
    async fn resource_limits(&self) -> Result<Option<limits::ResourceLimits>, RuntimeError> {
        Ok(Some(limits::ResourceLimits { memory: true, cpu: true, pids: true, rootless: true }))
    }
    async fn ensure_network(&self, _: &str, _: &BTreeMap<String, String>) -> Result<NetworkReport, RuntimeError> {
        Ok(NetworkReport { isolated: true })
    }
    async fn ensure_internal_network(&self, _: &str, _: &BTreeMap<String, String>) -> Result<(), RuntimeError> {
        Ok(())
    }
    async fn network_isolated(&self, _: &str) -> Result<Option<bool>, RuntimeError> {
        Ok(Some(true))
    }
    async fn ensure_image(&self, image: &str, timeout: Duration) -> Result<bool, RuntimeError> {
        self.prepared(image, timeout).await.map(|_| true)
    }
    async fn create(&self, spec: &ContainerSpec) -> Result<String, RuntimeError> {
        launch::resources(spec)?;
        if spec.agent.is_none() {
            return Err(error("VM requires its authenticated agent"));
        }
        agent::key(spec)?;
        let id = format!("vm-{}", uuid::Uuid::new_v4().simple());
        let mut entries = self.state.entries.lock().unwrap();
        if entries.values().any(|entry| entry.spec.name == spec.name) {
            return Err(RuntimeError::Conflict("VM sandbox already exists".into()));
        }
        let entry =
            Entry { id: id.clone(), spec: spec.clone(), started: None, finished: None, code: 0, agent_state: None };
        self.persist_entry(&entry)?;
        entries.insert(id.clone(), entry);
        Ok(id)
    }
    async fn start(&self, name: &str) -> Result<(), RuntimeError> {
        let (own, name) = (self.clone(), name.to_owned());
        tokio::spawn(async move {
            own.launch_entry(&name).await?;
            match own.observe(&name).await? {
                Some(info) if matches!(info.status, ContainerStatus::Running | ContainerStatus::Exited) => Ok(()),
                _ => Err(error("VM startup has no live agent or verified guest completion")),
            }
        })
        .await
        .map_err(error)?
    }
    async fn stop(&self, name: &str, signal: &str, grace: Duration) -> Result<(), RuntimeError> {
        let signal = if signal == crate::protocol::StopSignal::SIGTERM.as_str() {
            libc::SIGTERM
        } else if signal == crate::protocol::StopSignal::SIGINT.as_str() {
            libc::SIGINT
        } else {
            return Err(error("VM orderly shutdown requires SIGTERM or SIGINT"));
        };
        let (own, name) = (self.clone(), name.to_owned());
        tokio::spawn(async move { own.stop_entry(&name, signal, grace, false).await }).await.map_err(error)?
    }
    async fn kill(&self, name: &str) -> Result<(), RuntimeError> {
        let (own, name) = (self.clone(), name.to_owned());
        tokio::spawn(async move { own.stop_entry(&name, libc::SIGKILL, Duration::ZERO, true).await })
            .await
            .map_err(error)?
    }
    async fn pause(&self, _: &str) -> Result<(), RuntimeError> {
        Err(error("VM memory-sleep is off; stop and cold-boot its retained disk"))
    }
    async fn unpause(&self, _: &str) -> Result<(), RuntimeError> {
        Err(error("VM memory-sleep is off; start its retained disk"))
    }
    async fn disable_restart(&self, _: &str) -> Result<(), RuntimeError> {
        Ok(())
    }
    async fn remove(&self, name: &str) -> Result<(), RuntimeError> {
        let entry = match self.entry(name) {
            Ok(entry) => entry,
            Err(RuntimeError::NotFound(_)) => return Ok(()),
            Err(problem) => return Err(problem),
        };
        self.stop(&entry.id, &entry.spec.stop_signal, Duration::from_secs(u64::from(entry.spec.stop_timeout_secs)))
            .await?;
        let transition = self.transition(&entry.id);
        let _transition = transition.lock().await;
        if self.state.live.lock().unwrap().contains_key(&entry.id) {
            return Err(RuntimeError::Conflict("VM started while removal was waiting; its record is retained".into()));
        }
        self.forget_entry(&entry.id)
    }
    async fn inspect(&self, name: &str) -> Result<Option<ContainerInfo>, RuntimeError> {
        self.observe(name).await
    }
    async fn list(&self, labels: &[String]) -> Result<Vec<ContainerInfo>, RuntimeError> {
        let mut found = Vec::new();
        for name in self.names() {
            if let Some(info) = self.observe(&name).await?
                && labels.iter().all(|filter| {
                    filter.split_once('=').is_some_and(|(key, value)| info.labels.get(key).is_some_and(|v| v == value))
                })
            {
                found.push(info);
            }
        }
        Ok(found)
    }
    async fn exec(
        &self,
        name: &str,
        command: &[String],
        timeout: Duration,
        limit: usize,
    ) -> Result<ExecOutput, RuntimeError> {
        self.execute(name, command, timeout, limit).await
    }
    fn logs(
        &self,
        name: &str,
        options: LogOptions,
    ) -> futures_util::stream::BoxStream<'static, Result<LogChunk, RuntimeError>> {
        self.entrypoint_logs(name, options)
    }
    async fn stats(&self, name: &str) -> Result<Option<RawStats>, RuntimeError> {
        let entry = self.entry(name)?;
        let transition = self.transition(&entry.id);
        let _transition = transition.lock().await;
        let live = self.state.live.lock().unwrap().get(&entry.id).cloned();
        let Some(live) = live else { return Ok(None) };
        let mut live = live.lock().await;
        if live.monitor.exited().map_err(error)?.is_some() {
            return Ok(None);
        }
        if live.forwarder.exited().map_err(error)?.is_some() {
            return Err(RuntimeError::Unavailable("VM stats forwarding process exited".into()));
        }
        drop(live);
        let resources = agent::snapshot(&entry.spec)
            .await?
            .vm_resources
            .ok_or_else(|| RuntimeError::Unavailable("VM agent did not supply guest resource counters".into()))?;
        Ok(Some(RawStats {
            cpu_total_ns: resources.cpu_total_ns,
            memory_usage: resources.memory_usage,
            memory_inactive_file: resources.memory_inactive_file,
            memory_limit: u64::try_from(entry.spec.memory_bytes).ok(),
            pids: resources.pids,
            pids_limit: u64::try_from(entry.spec.pids_limit).ok(),
            rx_bytes: resources.rx_bytes,
            tx_bytes: resources.tx_bytes,
            ..Default::default()
        }))
    }
    fn events(
        &self,
        labels: &[String],
    ) -> futures_util::stream::BoxStream<'static, Result<RuntimeEvent, RuntimeError>> {
        let (own, labels) = (self.clone(), labels.to_vec());
        stream::unfold((own, labels, std::collections::BTreeSet::new()), |(own, labels, mut dead)| async move {
            loop {
                match own.list(&labels).await {
                    Ok(entries) => {
                        for info in entries {
                            if info.status == ContainerStatus::Running {
                                dead.remove(&info.id);
                            } else if matches!(info.status, ContainerStatus::Exited | ContainerStatus::Dead)
                                && dead.insert(info.id.clone())
                            {
                                return Some((
                                    Ok(RuntimeEvent {
                                        container_id: info.id,
                                        action: DIED.into(),
                                        exit_code: Some(info.exit_code),
                                        at: crate::clock::now(),
                                    }),
                                    (own, labels, dead),
                                ));
                            }
                        }
                    }
                    Err(problem) => return Some((Err(problem), (own, labels, dead))),
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        })
        .boxed()
    }
}
