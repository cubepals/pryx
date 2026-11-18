// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Docker exec attachment, bounded output, exit inspection and timeout tree containment.

use super::*;
use crate::runtime::parse_time;

// Docker exposes exec inspection, but no exec wait endpoint. Avoid a 20 ms quantization after EOF.
const EXIT_POLL: Duration = Duration::from_millis(4);
const KILLED_EXIT_WAIT: Duration = Duration::from_secs(1);

impl DockerRuntime {
    pub(super) async fn exec_command(
        &self,
        container: &str,
        command: &[String],
        timeout: Duration,
        max_output: usize,
        live: Option<tokio::sync::mpsc::Sender<LogChunk>>,
    ) -> Result<ExecOutput, RuntimeError> {
        let policy = self.rootless_exec_policy(container).await?;
        let container = policy.as_ref().map_or(container, |policy| policy.container.as_str());
        let identity = quick("inspect exec identity", self.docker()?.inspect_container(container, None))
            .await?
            .id
            .ok_or_else(|| RuntimeError::Other("exec target has no immutable identity".into()))?;
        let container = identity.as_str();
        let mut setup = Some(self.starts.lock(container).await);
        let prepared = prepare(self, container, command, timeout).await?;
        let timeout = prepared.timeout;
        let created = quick("create exec", self.docker()?.create_exec(container, prepared.options())).await?;
        let exec_id = created.id;
        let mut completion = prepared.session(self, container)?;
        let started = match quick(
            "start exec",
            self.docker()?
                .start_exec(&exec_id, Some(StartExecOptions { detach: false, tty: false, output_capacity: None })),
        )
        .await
        {
            Ok(started) => started,
            Err(error) => {
                if let Some(completion) = &mut completion {
                    drop(setup.take());
                    return Err(completion.failed().await);
                }
                return Err(error);
            }
        };
        let StartExecResults::Attached { mut output, mut input } = started else {
            return Err(RuntimeError::Other("exec started detached".into()));
        };

        if let Some(completion) = &mut completion
            && !completion.establish(&mut output, &mut input).await
        {
            drop(setup.take());
            return Err(completion.failed().await);
        }
        drop(input);
        drop(setup.take());
        let mut result = ExecOutput::default();
        let collect = async {
            collect_output(&mut output, &mut result, max_output, &live, &mut completion).await?;
            // Output can close while the process still runs. The execution deadline covers both.
            result.exit_code = self.exec_exit(&exec_id).await?;
            Ok::<(), RuntimeError>(())
        };
        match tokio::time::timeout(timeout, collect).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if let Some(completion) = &mut completion {
                    return Err(completion.failed().await);
                }
                return Err(e);
            }
            Err(_) => {
                result.timed_out = true;
                // Docker has no API to stop an exec (moby/moby#35703). From the host, pryxd
                // can: find its pid, make sure it is still this container's, and kill its tree.
                // Under gVisor the exec's processes live in its own kernel, so a host pid names
                // none of them; that path is never taken there.
                if let Some(completion) = &mut completion {
                    return Err(completion.failed().await);
                }
                if self.oci_runtime.is_some() {
                    return Err(RuntimeError::Other(super::gvisor::EXEC_TIMEOUT_PROBLEM.into()));
                }
                if let Some((pid, container)) = self.exec_pid(&exec_id).await {
                    result.killed = self.kill_expired_exec(pid, &container, policy.as_ref()).await? > 0;
                }
            }
        }
        drop(output);
        if let Some(completion) = &mut completion
            && !completion.complete(&mut result, max_output, &live).await
        {
            return Err(completion.failed().await);
        }
        if result.timed_out {
            if result.killed {
                match tokio::time::timeout(KILLED_EXIT_WAIT, self.exec_exit(&exec_id)).await {
                    Ok(Ok(code)) => result.exit_code = code,
                    Ok(Err(error)) => return Err(error),
                    Err(_) => {}
                }
            } else {
                let inspect = quick("inspect exec", self.docker()?.inspect_exec(&exec_id)).await?;
                if inspect.running != Some(true) {
                    result.exit_code = inspect.exit_code;
                }
            }
        }
        Ok(result)
    }

    async fn exec_exit(&self, exec_id: &str) -> Result<Option<i64>, RuntimeError> {
        loop {
            let inspect = quick("inspect exec", self.docker()?.inspect_exec(exec_id)).await?;
            if inspect.running != Some(true) {
                return Ok(inspect.exit_code);
            }
            tokio::time::sleep(EXIT_POLL).await;
        }
    }
    async fn kill_expired_exec(
        &self,
        pid: i32,
        container: &str,
        policy: Option<&RootlessExec>,
    ) -> Result<usize, RuntimeError> {
        let Some(policy) = policy else {
            return Ok(termination::kill_exec_tree(pid, container));
        };
        if policy.container != container || policy.init.pid == pid || !policy.init.alive() {
            return Err(RuntimeError::Other("exec-timeout refused an unverified process identity".into()));
        }
        tokio::time::timeout(Duration::from_secs(2), self.kill_rootless_exec(pid, policy))
            .await
            .map_err(|_| RuntimeError::Timeout("exec-timeout process termination was not confirmed".into()))?
    }

    async fn kill_rootless_exec(&self, pid: i32, policy: &RootlessExec) -> Result<usize, RuntimeError> {
        let root = termination::Held::capture(pid)
            .ok_or_else(|| RuntimeError::Other("exec-timeout process is no longer identifiable".into()))?;
        let mut held = BTreeMap::from([(pid, root)]);
        loop {
            if !held[&pid].alive() {
                return Ok(held.values_mut().map(termination::Held::kill).filter(|killed| *killed).count());
            }
            let current = termination::process_tree(pid);
            for target in &current {
                if !held.get(target).is_some_and(termination::Held::alive)
                    && let Some(process) = termination::Held::capture(*target)
                {
                    held.insert(*target, process);
                }
            }
            let allowed = current
                .iter()
                .copied()
                .filter(|target| policy.owner.owns(*target))
                .collect::<std::collections::BTreeSet<_>>();
            if !allowed.contains(&pid) {
                return Err(RuntimeError::Other(
                    "exec-timeout is unavailable: the selected engine did not prove process ownership".into(),
                ));
            }
            if !held.get_mut(&pid).expect("captured exec root").freeze() {
                return Err(RuntimeError::Other("exec-timeout could not stop its process".into()));
            }
            for (target, process) in &mut held {
                if *target != pid && allowed.contains(target) {
                    process.freeze();
                }
            }
            let current = termination::process_tree(pid);
            if current.iter().all(|target| held.get(target).is_some_and(termination::Held::stopped)) {
                let mut root = held.remove(&pid).expect("captured exec root");
                let children = held.values_mut().map(termination::Held::kill).filter(|killed| *killed).count();
                return Ok(children + usize::from(root.kill()));
            }
            tokio::task::yield_now().await;
        }
    }
}

/// Docker merges exec overrides with Config.Env. Blank bootstrap material before the process
/// exists, then remove its names in the trusted launcher before user code starts; dumpable=0
/// alone cannot protect the interval between execve and the launcher's first syscall.
struct Prepared {
    command: Vec<String>,
    timeout: Duration,
    authenticated: bool,
    environment: Vec<String>,
    start: Option<OffsetDateTime>,
}

impl Prepared {
    fn options(&self) -> CreateExecOptions<String> {
        CreateExecOptions {
            cmd: Some(self.command.clone()),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            attach_stdin: Some(self.authenticated),
            env: Some(self.environment.clone()),
            tty: Some(false),
            privileged: Some(false),
            ..Default::default()
        }
    }
    fn session(
        &self,
        runtime: &DockerRuntime,
        container: &str,
    ) -> Result<Option<super::exec_completion::Session>, RuntimeError> {
        if self.authenticated {
            Ok(Some(super::exec_completion::Session::new(
                runtime.docker()?,
                container,
                self.start.expect("authenticated exec has a start"),
                runtime.starts.clone(),
            )?))
        } else {
            Ok(None)
        }
    }
}

async fn prepare(
    runtime: &DockerRuntime,
    container: &str,
    command: &[String],
    timeout: Duration,
) -> Result<Prepared, RuntimeError> {
    let inspect = quick("inspect exec bootstrap", runtime.docker()?.inspect_container(container, None)).await?;
    let mounts = inspect.mounts.unwrap_or_default();
    let injected = mounts.iter().find(|mount| {
        mount.destination.as_deref() == Some(pryx_agent::contract::BINARY_PATH) && mount.rw == Some(false)
    });
    let mut prepared = Prepared {
        command: command.to_vec(),
        timeout,
        authenticated: false,
        environment: Vec::new(),
        start: inspect.state.and_then(|state| parse_time(state.started_at.as_deref())),
    };
    let Some(injected) = injected else {
        return Ok(prepared);
    };
    let config = inspect.config.unwrap_or_default();
    let labels = config.labels.unwrap_or_default();
    let version = labels.get(crate::labels::LABEL_EXEC_COMPLETION).and_then(|version| version.parse::<u16>().ok());
    if version.is_none_or(|version| version < pryx_agent::exec_completion::VERSION) {
        return Err(RuntimeError::Other("exec-timeout and exec-bootstrap-secret-protection are unavailable: this injected agent does not support authenticated cleanup; recreate the sandbox with a current agent".into()));
    }
    if runtime.oci_runtime.is_some()
        && !injected.source.as_deref().is_some_and(|source| super::exec_agent::protected(std::path::Path::new(source)))
    {
        return Err(RuntimeError::Other("exec-timeout is unavailable: the injected agent is readable before execution; recreate the sandbox with a protected agent".into()));
    }
    if runtime.oci_runtime.is_some() {
        let source = injected.source.as_deref().expect("protected source was checked");
        if !crate::runtime::agent_metadata::query(std::path::Path::new(source))
            .await
            .is_some_and(|capabilities| capabilities.exec_completion_version >= pryx_agent::exec_completion::VERSION)
        {
            return Err(RuntimeError::Other(
                "exec-timeout is unavailable: the actual protected agent lacks authenticated cleanup".into(),
            ));
        }
    }
    let secret = labels.get(pryx_agent::contract::KEY_LABEL);
    prepared.environment = config
        .env
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            let (name, _) = entry.split_once('=')?;
            (name.starts_with(pryx_agent::contract::SECRET_ENV_PREFIX)
                || Some(name) == secret.map(String::as_str)
                || name == pryx_agent::contract::MOUNTS_ENV
                || name == pryx_agent::process_policy::POLICY_ENV)
                .then(|| format!("{name}="))
        })
        .collect();
    prepared.command = vec![pryx_agent::contract::BINARY_PATH.into()];
    if runtime.oci_runtime.is_some() {
        if prepared.start.is_none() {
            return Err(RuntimeError::Other("exec-timeout cannot identify this sandbox start".into()));
        }
        prepared.authenticated = true;
        prepared.timeout += DEADLINE_GRACE;
        prepared.command.extend([pryx_agent::exec_deadline::MODE.into(), timeout.as_millis().to_string()]);
    } else {
        prepared.command.push(pryx_agent::exec_deadline::CLEAN_MODE.into());
    }
    if let Some(secret) = secret {
        prepared.command.extend(["--secret-env".into(), secret.clone()]);
    }
    prepared.command.push("--".into());
    prepared.command.extend(command.iter().cloned());
    Ok(prepared)
}

/// The engine wait outlasts successful descendant retirement inside the guest.
const DEADLINE_GRACE: Duration = Duration::from_secs(3);

async fn collect_output<S>(
    output: &mut S,
    result: &mut ExecOutput,
    max_output: usize,
    live: &Option<tokio::sync::mpsc::Sender<LogChunk>>,
    completion: &mut Option<super::exec_completion::Session>,
) -> Result<(), RuntimeError>
where
    S: futures_util::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Unpin,
{
    while let Some(item) = output.next().await {
        let item = match item.map_err(map_err)? {
            LogOutput::StdErr { message } if completion.is_some() => {
                LogOutput::StdErr { message: completion.as_mut().unwrap().stderr(&message) }
            }
            other => other,
        };
        let (buffer, truncated, stream, bytes) = match item {
            LogOutput::StdOut { message } => {
                (&mut result.stdout, &mut result.stdout_truncated, LogStream::Stdout, message)
            }
            LogOutput::StdErr { message } => {
                (&mut result.stderr, &mut result.stderr_truncated, LogStream::Stderr, message)
            }
            _ => continue,
        };
        let room = max_output.saturating_sub(buffer.len());
        buffer.extend_from_slice(&bytes[..bytes.len().min(room)]);
        *truncated |= bytes.len() > room;
        if let Some(live) = &live {
            for chunk in bytes[..bytes.len().min(room)].chunks(crate::protocol::EXEC_CHUNK_BYTES) {
                if live.send(LogChunk { stream, bytes: bytes::Bytes::copy_from_slice(chunk) }).await.is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::Request, routing::any};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn a_legacy_injected_agent_is_refused_before_exec_or_key_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let writes = Arc::new(AtomicUsize::new(0));
        let seen = writes.clone();
        let spoof = Arc::new(AtomicBool::new(false));
        let forged = spoof.clone();
        let app = Router::new().fallback(any(move |request: Request| {
            let seen = seen.clone();
            let forged = forged.load(Ordering::SeqCst);
            async move {
                if request.method() != axum::http::Method::GET {
                    seen.fetch_add(1, Ordering::SeqCst);
                }
                Json(bollard::models::ContainerInspectResponse {
                    id: Some("legacy".into()),
                    config: Some(bollard::models::ContainerConfig {
                        labels: Some(if forged {
                            HashMap::from([(crate::labels::LABEL_EXEC_COMPLETION.into(), "1".into())])
                        } else {
                            HashMap::new()
                        }),
                        ..Default::default()
                    }),
                    mounts: Some(vec![bollard::models::MountPoint {
                        source: Some(
                            Path::new(env!("CARGO_MANIFEST_DIR"))
                                .join("tests/fixtures/agent-metadata")
                                .to_string_lossy()
                                .into_owned(),
                        ),
                        destination: Some(pryx_agent::contract::BINARY_PATH.into()),
                        rw: Some(false),
                        ..Default::default()
                    }]),
                    ..Default::default()
                })
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let runtime = DockerRuntime::new(&path).unwrap().gvisor();
        *runtime.info.lock().unwrap() = Some(RuntimeInfo::default());
        for forged in [false, true] {
            spoof.store(forged, Ordering::SeqCst);
            let error =
                runtime.exec_command("legacy", &["true".into()], Duration::from_secs(1), 1024, None).await.unwrap_err();
            assert!(error.to_string().contains("unavailable"), "legacy and forged-label mounts are refused");
        }
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        server.abort();
    }

    #[tokio::test]
    async fn bootstrap_material_is_blank_before_the_exec_process_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let (_, material) = pryx_agent::auth::SigningKey::generate().unwrap();
        let prefix = format!("{}fixture", pryx_agent::contract::SECRET_ENV_PREFIX);
        let hidden = prefix.clone();
        let opaque = material.clone();
        let app = Router::new().fallback(any(move || {
            let hidden = hidden.clone();
            let opaque = opaque.clone();
            async move {
                Json(bollard::models::ContainerInspectResponse {
                    config: Some(bollard::models::ContainerConfig {
                        labels: Some(HashMap::from([
                            (
                                crate::labels::LABEL_EXEC_COMPLETION.into(),
                                pryx_agent::exec_completion::VERSION.to_string(),
                            ),
                            (pryx_agent::contract::KEY_LABEL.into(), "CUSTOM_BOOTSTRAP".into()),
                        ])),
                        env: Some(vec![
                            format!("{hidden}={opaque}"),
                            format!("CUSTOM_BOOTSTRAP={opaque}"),
                            "ORDINARY_FIXTURE=kept".into(),
                        ]),
                        ..Default::default()
                    }),
                    mounts: Some(vec![bollard::models::MountPoint {
                        destination: Some(pryx_agent::contract::BINARY_PATH.into()),
                        rw: Some(false),
                        ..Default::default()
                    }]),
                    ..Default::default()
                })
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let runtime = DockerRuntime::new(&path).unwrap();
        let prepared = prepare(&runtime, "owned", &["true".into()], Duration::from_secs(1)).await.unwrap();
        assert!(prepared.environment == vec![format!("{prefix}="), "CUSTOM_BOOTSTRAP=".into()]);
        assert!(
            !prepared.environment.iter().any(|entry| entry.contains(&material)),
            "bootstrap material never reaches exec creation"
        );
        assert!(
            !prepared.environment.iter().any(|entry| entry.starts_with("ORDINARY_FIXTURE=")),
            "ordinary variables retain Docker's original environment"
        );
        server.abort();
    }
}
