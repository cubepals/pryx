// SPDX-License-Identifier: FSL-1.1-ALv2
//! Bounded process readiness uses the runtime's own network, including virtual guests. Exits
//! before readiness differ from elapsed deadlines; no probe can choose an outside destination.
use crate::{
    context::{Agent, Failure},
    process::Event,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

pub(crate) fn routes() -> Router<Arc<Agent>> {
    Router::new().route("/_pryx/processes/{id}/ready", post(wait)).route("/_pryx/processes/kill-all", post(kill_all))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub(crate) struct Wait {
    kind: String,
    port: Option<u16>,
    mode: Option<String>,
    path: Option<String>,
    status: Option<u16>,
    pattern: Option<String>,
    timeout: f64,
}
fn invalid(message: &str) -> Failure {
    Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request", message.into())
}
fn exited() -> Failure {
    Failure(StatusCode::CONFLICT, "process_exited", "Process exited before it became ready".into())
}
async fn wait(
    State(agent): State<Arc<Agent>>,
    Path(id): Path<String>,
    Json(body): Json<Wait>,
) -> Result<Json<serde_json::Value>, Failure> {
    if !body.timeout.is_finite() || body.timeout <= 0.0 || body.timeout > crate::contract::MAX_WAIT.as_secs_f64() {
        return Err(invalid("timeout exceeds the supported readiness duration"));
    }
    let process = agent.processes.get(&id)?;
    let work = async {
        match body.kind.as_str() {
            "exit" => return Ok(Json(serde_json::to_value(process.wait().await).expect("result serializes"))),
            "log" => {
                let pattern = regex::bytes::RegexBuilder::new(
                    body.pattern.as_deref().ok_or_else(|| invalid("pattern required"))?,
                )
                .size_limit(crate::contract::CAPTURE_BYTES)
                .build()
                .map_err(|_| invalid("Invalid or excessive regular expression"))?;
                let (replay, mut live) = process.subscribe().await;
                let mut replay = std::collections::VecDeque::from(replay);
                let mut text = std::collections::HashMap::<&str, Vec<u8>>::new();
                loop {
                    let event = match replay.pop_front() {
                        Some(event) => Some(event),
                        None => live.recv().await,
                    };
                    match event {
                        Some(Event::Output { bytes, stream }) => {
                            let bytes = STANDARD.decode(bytes).map_err(|_| invalid("Invalid output"))?;
                            text.entry(stream).or_default().extend(bytes);
                            if text.values().map(Vec::len).sum::<usize>() > crate::contract::CAPTURE_BYTES {
                                return Err(Failure(
                                    StatusCode::CONFLICT,
                                    "output_overflow",
                                    "Log wait exceeded capture budget".into(),
                                ));
                            }
                            if text.values().any(|bytes| pattern.is_match(bytes)) {
                                return Ok(Json(serde_json::json!({"matched":true})));
                            }
                        }
                        Some(Event::Truncated) => {
                            return Err(Failure(
                                StatusCode::CONFLICT,
                                "output_overflow",
                                "Retained logs were truncated".into(),
                            ));
                        }
                        _ => return Err(exited()),
                    }
                }
            }
            "port" => {
                let port = body.port.filter(|p| *p > 0).ok_or_else(|| invalid("port required"))?;
                let path = match body.mode.as_deref().unwrap_or("tcp") {
                    "tcp" => None,
                    "http" => Some(body.path.as_deref().unwrap_or("/")),
                    _ => return Err(invalid("mode must be http or tcp")),
                };
                if path.is_some_and(|p| !p.starts_with('/') || p.contains(['\r', '\n'])) {
                    return Err(invalid("Invalid HTTP path"));
                }
                loop {
                    if !process.view().running {
                        return Err(exited());
                    }
                    let probe = async {
                        if let Some(forward) = &agent.ports.forward {
                            forward.probe(port, path).await
                        } else {
                            probe_stream(TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await?, path).await
                        }
                    };
                    if let Ok(Ok(status)) = tokio::time::timeout(Duration::from_secs(1), probe).await
                        && (path.is_none() || status == Some(body.status.unwrap_or(StatusCode::OK.as_u16())))
                    {
                        return Ok(Json(serde_json::json!({"ready":true})));
                    }
                    tokio::select! { _=process.wait()=>return Err(exited()), ()=tokio::time::sleep(Duration::from_millis(50))=>{} }
                }
            }
            _ => Err(invalid("Unknown readiness kind")),
        }
    };
    tokio::time::timeout(Duration::from_secs_f64(body.timeout), work)
        .await
        .map_err(|_| Failure(StatusCode::REQUEST_TIMEOUT, "wait_timeout", "Readiness deadline elapsed".into()))?
}
pub async fn probe_stream(
    mut stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    path: Option<&str>,
) -> std::io::Result<Option<u16>> {
    let Some(path) = path else { return Ok(None) };
    stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes()).await?;
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 1024];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(at) = bytes.windows(2).position(|p| p == b"\r\n") {
            let mut headers = [];
            let mut response = httparse::Response::new(&mut headers);
            response
                .parse(&bytes[..at + 2])
                .or_else(|e| if e == httparse::Error::TooManyHeaders { Ok(httparse::Status::Partial) } else { Err(e) })
                .map_err(std::io::Error::other)?;
            return response.code.map(Some).ok_or_else(|| std::io::ErrorKind::InvalidData.into());
        }
        if bytes.len() > 8192 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    }
}
async fn kill_all(State(agent): State<Arc<Agent>>) -> Result<Json<serde_json::Value>, Failure> {
    let mut killed = Vec::new();
    for view in agent.processes.list().into_iter().filter(|p| p.running) {
        let process = agent.processes.get(&view.id)?;
        if process.view().running {
            if let Err(error) = process.signal_awaited(libc::SIGKILL).await {
                if process.view().running {
                    return Err(error.into());
                }
            } else {
                killed.push(view.id);
            }
        }
    }
    Ok(Json(serde_json::json!({"killed":killed})))
}
