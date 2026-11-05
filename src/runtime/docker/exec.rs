// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Docker exec attachment, bounded output, exit inspection and timeout tree containment.

use super::*;

impl DockerRuntime {
    pub(super) async fn exec_command(
        &self,
        container: &str,
        command: &[String],
        timeout: Duration,
        max_output: usize,
    ) -> Result<ExecOutput, RuntimeError> {
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
                let (buffer, truncated, bytes) = match item.map_err(map_err)? {
                    LogOutput::StdOut { message } => (&mut result.stdout, &mut result.stdout_truncated, message),
                    LogOutput::StdErr { message } => (&mut result.stderr, &mut result.stderr_truncated, message),
                    _ => continue,
                };
                let room = max_output.saturating_sub(buffer.len());
                buffer.extend_from_slice(&bytes[..bytes.len().min(room)]);
                *truncated |= bytes.len() > room;
            }
            Ok::<(), RuntimeError>(())
        };
        match tokio::time::timeout(timeout, collect).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                result.timed_out = true;
                // Docker has no API to stop an exec (moby/moby#35703). From the host, pryxd
                // can: find its pid, make sure it is still this container's, and kill its tree.
                if let Some((pid, container)) = self.exec_pid(&exec_id).await {
                    result.killed = kill_exec_tree(pid, &container) > 0;
                }
            }
        }
        drop(output);
        // The exit code lands a moment after the stream ends.
        for _ in 0..50 {
            let inspect = quick("inspect exec", self.docker()?.inspect_exec(&exec_id)).await?;
            if inspect.running != Some(true) {
                result.exit_code = inspect.exit_code;
                break;
            }
            if result.timed_out && !result.killed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(result)
    }
}
