// SPDX-License-Identifier: FSL-1.1-ALv2
//! Tool invocations reuse supervised process limits and bounded output. No user string is
//! interpreted as shell syntax, and failures never include secret-bearing command/environment data.
use super::state::failure;
use crate::{
    auth::Claims,
    context::{Agent, Failure},
    process::{Command, Event, Process, Start},
};
use std::{collections::BTreeMap, sync::Arc};
pub(crate) struct Starting(pub(crate) Option<Arc<Process>>);
impl Drop for Starting {
    fn drop(&mut self) {
        if let Some(process) = &self.0 {
            let _ = process.signal(libc::SIGKILL);
        }
    }
}

pub(crate) fn start(
    agent: &Agent,
    argv: Vec<String>,
    env: BTreeMap<String, String>,
    timeout: Option<u32>,
    mount: bool,
) -> Result<Arc<Process>, Failure> {
    let body = Start {
        command: Command::Argv(argv),
        env,
        cwd: None,
        stdin: false,
        pty: false,
        cols: None,
        rows: None,
        timeout,
    };
    let key = uuid::Uuid::new_v4().to_string();
    let process = if mount {
        agent.processes.start_mount(body, &key, timeout)
    } else {
        agent.processes.start(body, &key, timeout)
    };
    process.map_err(|_| failure("tool_unavailable", "Required native tool is unavailable; use its prepared template"))
}
pub(crate) async fn run(
    agent: &Agent,
    claims: &Claims,
    argv: Vec<String>,
    input: Option<&[u8]>,
) -> Result<Vec<u8>, Failure> {
    let timeout = claims.max_exec_seconds.unwrap_or(30).min(30);
    let body = Start {
        command: Command::Argv(argv),
        env: BTreeMap::new(),
        cwd: None,
        stdin: input.is_some(),
        pty: false,
        cols: None,
        rows: None,
        timeout: Some(timeout),
    };
    let process = agent
        .processes
        .start(body, &uuid::Uuid::new_v4().to_string(), Some(timeout))
        .map_err(|_| failure("tool_unavailable", "Required native tool is unavailable; use its prepared template"))?;
    let _active = agent.telemetry.active();
    if let Some(input) = input {
        if input.len() > crate::contract::CAPTURE_BYTES {
            let _ = process.signal(libc::SIGKILL);
            return Err(failure("invalid_request", "Tool input exceeded its budget"));
        }
        if process.write(input).await.is_err() {
            let _ = process.signal(libc::SIGKILL);
            return Err(failure("tool_failed", "Tool input delivery failed"));
        }
        process.close_input().await;
    }
    let (replay, mut receiver) = process.subscribe().await;
    let mut replay: std::collections::VecDeque<_> = replay.into();
    let mut out = Vec::new();
    use base64::Engine;
    loop {
        let event = if let Some(event) = replay.pop_front() { Some(event) } else { receiver.recv().await };
        match event {
            Some(Event::Output { stream: "stdout", bytes }) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(bytes)
                    .map_err(|_| failure("tool_failed", "Invalid tool output"))?;
                out.extend(bytes);
            }
            Some(Event::Exit { result }) => {
                // The output is consumed here; nothing replays it, so the entry need not be kept.
                agent.processes.release(&process.id);
                if result.code != Some(0) || result.timed_out {
                    return Err(failure("tool_failed", "Native tool failed; check template and host requirements"));
                }
                return Ok(out);
            }
            Some(Event::Truncated) | None => {
                let _ = process.signal(libc::SIGKILL);
                return Err(failure("tool_failed", "Native tool output was incomplete"));
            }
            _ => {}
        }
        if out.len() > crate::contract::CAPTURE_BYTES {
            let _ = process.signal(libc::SIGKILL);
            return Err(failure("output_overflow", "Tool output exceeded its capture budget"));
        }
    }
}
pub(crate) async fn image(path: &std::path::Path) -> Result<Vec<u8>, Failure> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path).await?;
    let mut bytes = Vec::new();
    file.take(super::defaults::MAX_IMAGE_BYTES as u64 + 1).read_to_end(&mut bytes).await?;
    let _ = tokio::fs::remove_file(path).await;
    if bytes.len() > super::defaults::MAX_IMAGE_BYTES || !bytes.starts_with(super::defaults::PNG) {
        return Err(failure("invalid_image", "Tool did not produce a PNG within its image budget"));
    }
    Ok(bytes)
}
