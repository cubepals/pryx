// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Docker exec attachment, bounded output, exit inspection and timeout tree containment.

use super::*;

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
        // gVisor: the deadline is kept inside by the injected agent where there is one.
        let deadline = deadline(self, container, command, timeout).await?;
        let (command, timeout) = deadline.as_ref().map_or((command, timeout), |(wrapped, outer)| (wrapped, *outer));
        let began = tokio::time::Instant::now();
        let options = CreateExecOptions::<String> {
            cmd: Some(command.to_vec()),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            attach_stdin: Some(false),
            tty: Some(false),
            privileged: Some(false),
            ..Default::default()
        };
        let created = quick("create exec", self.docker()?.create_exec(container, options)).await?;
        let exec_id = created.id;
        let started = quick(
            "start exec",
            self.docker()?
                .start_exec(&exec_id, Some(StartExecOptions { detach: false, tty: false, output_capacity: None })),
        )
        .await?;
        let StartExecResults::Attached { mut output, .. } = started else {
            return Err(RuntimeError::Other("exec started detached".into()));
        };

        let mut result = ExecOutput::default();
        let collect = async {
            while let Some(item) = output.next().await {
                let (buffer, truncated, stream, bytes) = match item.map_err(map_err)? {
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
            // Output can close while the process still runs. The execution deadline covers both.
            result.exit_code = self.exec_exit(&exec_id).await?;
            Ok::<(), RuntimeError>(())
        };
        match tokio::time::timeout(timeout, collect).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                result.timed_out = true;
                // Docker has no API to stop an exec (moby/moby#35703). From the host, pryxd
                // can: find its pid, make sure it is still this container's, and kill its tree.
                // Under gVisor the exec's processes live in its own kernel, so a host pid names
                // none of them; that path is never taken there.
                if self.oci_runtime.is_some() {
                    return Err(RuntimeError::Other(super::gvisor::EXEC_TIMEOUT_PROBLEM.into()));
                }
                if let Some((pid, container)) = self.exec_pid(&exec_id).await {
                    result.killed = self.kill_expired_exec(pid, &container, policy.as_ref()).await? > 0;
                }
            }
        }
        drop(output);
        if deadline.is_some() && !result.timed_out {
            kept_deadline(&mut result, began.elapsed(), timeout);
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

/// How long Docker's own wait outlasts the deadline the supervisor keeps inside the sandbox.
const DEADLINE_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// A gvisor node exec run under the agent's in-sandbox deadline supervisor, with the longer wait
/// Docker gives it, where the container has the injected agent. Without it (an agent-less
/// workload), None: an exec past its deadline is then refused by name, as before.
async fn deadline(
    runtime: &DockerRuntime,
    container: &str,
    command: &[String],
    timeout: std::time::Duration,
) -> Result<Option<(Vec<String>, std::time::Duration)>, crate::runtime::RuntimeError> {
    if runtime.oci_runtime.is_none() {
        return Ok(None);
    }
    let inspect = quick("inspect exec target", runtime.docker()?.inspect_container(container, None)).await?;
    let injected = inspect.mounts.unwrap_or_default().iter().any(|mount| {
        mount.destination.as_deref() == Some(pryx_agent::contract::BINARY_PATH) && mount.rw == Some(false)
    });
    if !injected {
        return Ok(None);
    }
    let mut wrapped = vec![
        pryx_agent::contract::BINARY_PATH.to_owned(),
        pryx_agent::exec_deadline::MODE.to_owned(),
        timeout.as_millis().to_string(),
        "--".to_owned(),
    ];
    wrapped.extend(command.iter().cloned());
    Ok(Some((wrapped, timeout + DEADLINE_GRACE)))
}

/// The supervisor ends a command at its deadline with SIGKILL's status; only then, at or past the
/// deadline Docker's wait outlasted, is the exec reported timed out and killed.
fn kept_deadline(result: &mut ExecOutput, elapsed: std::time::Duration, outer: std::time::Duration) {
    if result.exit_code == Some(crate::runtime::exit_by(libc::SIGKILL)) && elapsed + DEADLINE_GRACE >= outer {
        result.timed_out = true;
        result.killed = true;
    }
}
