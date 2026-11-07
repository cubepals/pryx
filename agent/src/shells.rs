// SPDX-License-Identifier: FSL-1.1-ALv2
//! Persistent pipe shells serialize commands. Independent random markers delimit stdout and
//! stderr, retaining exact status without PTY echo or stream merging. Broken sessions close.
use crate::{
    context::{Agent, Failure},
    process::{Event, Start},
};
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    routing::{delete, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

use crate::shell_state::{Shell, ShellOutput};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub(crate) struct Open {
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub(crate) struct Run {
    command: String,
    timeout: f64,
}
pub(crate) fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/_pryx/shells", post(open))
        .route("/_pryx/shells/{id}/run", post(run))
        .route("/_pryx/shells/{id}", delete(close))
}
fn failure(code: &'static str, message: &str) -> Failure {
    Failure(StatusCode::CONFLICT, code, message.into())
}
async fn open(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<crate::auth::Claims>,
    Json(body): Json<Open>,
) -> Result<Json<serde_json::Value>, Failure> {
    let process = agent.processes.start(
        Start {
            command: crate::process::Command::Shell(
                r#"while IFS= read -r __pryx_line; do eval "$(command printf '%b' "$__pryx_line")"; done"#.into(),
            ),
            cwd: body.cwd,
            env: body.env,
            stdin: true,
            pty: false,
            cols: None,
            rows: None,
            timeout: None,
        },
        &uuid::Uuid::new_v4().to_string(),
        claims.max_exec_seconds,
    )?;
    let (replay, live) = process.subscribe().await;
    let id = process.id.clone();
    let admitted = {
        let mut held = agent.shells.0.lock().unwrap();
        held.retain(|_, s| s.process.view().running);
        if held.len() >= 32 {
            false
        } else {
            held.insert(
                id.clone(),
                Arc::new(Shell {
                    process: process.clone(),
                    output: AsyncMutex::new(ShellOutput { replay: replay.into(), live }),
                }),
            );
            true
        }
    };
    if !admitted {
        process.signal_awaited(libc::SIGKILL).await?;
        return Err(failure("shell_capacity", "Shell session capacity reached"));
    }
    Ok(Json(serde_json::json!({"id":id})))
}
async fn run(
    State(agent): State<Arc<Agent>>,
    Path(id): Path<String>,
    Json(body): Json<Run>,
) -> Result<Json<serde_json::Value>, Failure> {
    if !body.timeout.is_finite()
        || body.timeout <= 0.0
        || body.timeout > crate::contract::MAX_WAIT.as_secs_f64()
        || body.command.contains('\0')
    {
        return Err(failure("invalid_request", "Invalid shell command or timeout"));
    }
    let shell =
        agent.shells.0.lock().unwrap().get(&id).cloned().ok_or_else(|| failure("not_found", "No such shell"))?;
    let started = Instant::now();
    let work = async {
        let mut output = shell.output.lock().await;
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let command = format!(
            "{}\ncommand printf '\\036{nonce}:%d\\037' \"$?\"; command printf '\\036{nonce}\\037' >&2\n",
            body.command
        );
        let encoded = command.bytes().map(|b| format!("\\0{b:03o}")).collect::<String>() + "\n";
        shell.process.write(encoded.as_bytes()).await?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let out_marker = format!("\x1e{nonce}:");
        let err_marker = format!("\x1e{nonce}\x1f");
        loop {
            let event = match output.replay.pop_front() {
                Some(e) => Some(e),
                None => output.live.recv().await,
            };
            match event {
                Some(Event::Output { stream, bytes }) => {
                    let decoded =
                        STANDARD.decode(bytes).map_err(|_| failure("shell_protocol", "Invalid shell output"))?;
                    let target = if stream == "stdout" { &mut stdout } else { &mut stderr };
                    target.extend(decoded);
                    if target.len() > crate::contract::CAPTURE_BYTES {
                        return Err(failure("output_overflow", "Shell output exceeded capture budget"));
                    }
                }
                Some(Event::Exit { result }) => {
                    return Ok(Json(
                        serde_json::json!({"stdout":String::from_utf8_lossy(&stdout),"stderr":String::from_utf8_lossy(&stderr),"code":result.code,"timedOut":result.timed_out,"killed":result.killed,"durationMs":started.elapsed().as_millis(),"stdoutTruncated":false,"stderrTruncated":false}),
                    ));
                }
                Some(Event::Truncated) => return Err(failure("output_overflow", "Shell output truncated")),
                _ => return Err(failure("process_exited", "Shell exited before the command completed")),
            }
            let out = stdout.windows(out_marker.len()).position(|w| w == out_marker.as_bytes());
            let err = stderr.windows(err_marker.len()).position(|w| w == err_marker.as_bytes());
            if let (Some(out), Some(err)) = (out, err) {
                let status_at = out + out_marker.len();
                if let Some(end) = stdout[status_at..].iter().position(|b| *b == 0x1f) {
                    let code = std::str::from_utf8(&stdout[status_at..status_at + end])
                        .ok()
                        .and_then(|s| s.parse::<i32>().ok())
                        .ok_or_else(|| failure("shell_protocol", "Invalid shell exit status"))?;
                    if status_at + end + 1 != stdout.len() || err + err_marker.len() != stderr.len() {
                        return Err(failure("shell_protocol", "Unattributed background shell output"));
                    }
                    return Ok(Json(
                        serde_json::json!({"stdout":String::from_utf8_lossy(&stdout[..out]),"stderr":String::from_utf8_lossy(&stderr[..err]),"code":code,"timedOut":false,"killed":false,"durationMs":started.elapsed().as_millis(),"stdoutTruncated":false,"stderrTruncated":false}),
                    ));
                }
            }
        }
    };
    let answer = tokio::time::timeout(Duration::from_secs_f64(body.timeout), work)
        .await
        .unwrap_or_else(|_| Err(failure("wait_timeout", "Shell command timed out; session closed")));
    if answer.is_err() {
        agent.shells.0.lock().unwrap().remove(&id);
        let process = &shell.process;
        if process.view().running {
            let _ = process.signal_awaited(libc::SIGKILL).await;
        }
    }
    answer
}
async fn close(State(agent): State<Arc<Agent>>, Path(id): Path<String>) -> Result<StatusCode, Failure> {
    let shell = agent.shells.0.lock().unwrap().remove(&id);
    if let Some(shell) = shell {
        let process = &shell.process;
        if process.view().running {
            process.signal_awaited(libc::SIGKILL).await?;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
