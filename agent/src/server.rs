// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The sandbox's authenticated process, terminal, file and port API. There is no fleet client,
//! lifecycle database, host shell or request logger here. Verification uses only local state.

use std::convert::Infallible;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Extension, Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

use crate::auth::{self, Claims, Scope};
use crate::process::{Event, Process, Start};

pub use crate::context::{Agent, Failure};
pub use crate::telemetry::now;

async fn authenticate(State(agent): State<Arc<Agent>>, mut request: Request, next: Next) -> Response {
    #[derive(Deserialize, Default)]
    struct FileQuery {
        path: Option<String>,
        token: Option<String>,
    }
    let query = axum::extract::Query::<FileQuery>::try_from_uri(request.uri()).map(|q| q.0).unwrap_or_default();
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| {
            request
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(',').find_map(|protocol| protocol.trim().strip_prefix("pryx-token.")))
        })
        .or(query.token.as_deref());
    let claims =
        token.and_then(|token| auth::verify(token, &agent.public_key, &agent.sandbox, agent.epoch, now()).ok());
    let Some(claims) = claims else {
        return Failure(StatusCode::UNAUTHORIZED, "unauthorized", "Sandbox token refused".into()).into_response();
    };
    if claims.scope != Scope::Sandbox
        && !claims.allows_file(request.uri().path(), request.method().as_str(), query.path.as_deref())
        && !(claims.scope == Scope::Routes
            && request.method() == axum::http::Method::GET
            && matches!(request.uri().path(), "/_pryx/ports" | "/_pryx/health"))
    {
        return Failure(StatusCode::FORBIDDEN, "token_scope", "Sandbox token has another scope".into()).into_response();
    }
    let active = (claims.scope != Scope::Routes).then(|| agent.telemetry.active());
    request.extensions_mut().insert(claims);
    let response = next.run(request).await;
    let (parts, body) = response.into_parts();
    let stream = futures_util::stream::unfold((body.into_data_stream(), active), |(mut stream, active)| async move {
        stream.next().await.map(|part| (part, (stream, active)))
    });
    Response::from_parts(parts, Body::from_stream(stream))
}

/// Trusted maintenance guests expose data operations and readiness only.
pub fn files_router(agent: Arc<Agent>) -> Router {
    Router::new()
        .route("/_pryx/health", get(health))
        .merge(crate::files::data_routes())
        .layer(DefaultBodyLimit::max(crate::contract::MAX_REQUEST_BYTES))
        .route_layer(middleware::from_fn_with_state(agent.clone(), authenticate))
        .with_state(agent)
}

pub fn router(agent: Arc<Agent>) -> Router {
    let router = Router::new()
        .route("/_pryx/health", get(health))
        .route("/_pryx/logs", get(logs))
        .route("/_pryx/processes", get(list).post(start))
        .route("/_pryx/processes/{id}", get(view))
        .route("/_pryx/processes/{id}/wait", get(wait))
        .route("/_pryx/processes/{id}/stdin", post(stdin).delete(close_stdin))
        .route("/_pryx/processes/{id}/signal", post(signal))
        .route("/_pryx/processes/{id}/resize", post(resize))
        .route("/_pryx/processes/{id}/output", get(output))
        .route("/_pryx/processes/{id}/ws", get(websocket))
        .route("/_pryx/terminal", get(terminal))
        .route("/_pryx/ports", get(ports).post(expose))
        .route("/_pryx/ports/{port}", delete(unexpose))
        .merge(crate::files::routes())
        .merge(crate::field_process::routes())
        .merge(crate::shells::routes())
        .merge(crate::kernels::routes())
        .merge(crate::tools::routes())
        .merge(crate::file_search::routes());
    #[cfg(feature = "mcp")]
    let router = router.merge(crate::mcp::routes());
    router
        .layer(DefaultBodyLimit::max(crate::contract::MAX_REQUEST_BYTES))
        .route_layer(middleware::from_fn_with_state(agent.clone(), authenticate))
        .with_state(agent)
}

async fn health(State(agent): State<Arc<Agent>>, headers: HeaderMap) -> Json<serde_json::Value> {
    let mut value = serde_json::json!({"status":"ok","apiVersion":crate::contract::API_VERSION,"kernelsVersion":crate::kernels::VERSION,"toolsVersion":crate::tools::VERSION,"mountsPending":crate::tools::mounts_pending(&agent)});
    value["state"] = serde_json::to_value(crate::state::Snapshot {
        ports: agent.ports.snapshot().await,
        activity: agent.telemetry.snapshot(),
        vm_resources: crate::vm_metrics::snapshot(agent.vm_resources),
    })
    .expect("Agent state serializes");
    if crate::process_policy::ipc_scoped() {
        value["processIpcScoped"] = true.into();
    }
    if let Some(nonce) = headers
        .get(crate::process_observation::NONCE_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|nonce| crate::process_observation::nonce_valid(nonce))
    {
        let proof = crate::process_observation::sign(
            &agent.key,
            crate::process_observation::Observation {
                nonce: nonce.into(),
                sandbox: agent.sandbox.clone(),
                epoch: agent.epoch,
                pid: std::process::id(),
                ipc_scoped: crate::process_policy::ipc_scoped(),
            },
        );
        value["processObservation"] = serde_json::to_value(proof).expect("fixed observation serializes");
    }
    Json(value)
}

async fn list(State(agent): State<Arc<Agent>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"processes": agent.processes.list()}))
}

#[derive(Deserialize, Default)]
struct LogQuery {
    #[serde(default)]
    follow: bool,
}
async fn logs(
    State(agent): State<Arc<Agent>>,
    axum::extract::Query(query): axum::extract::Query<LogQuery>,
) -> Response {
    let (replay, live) = agent.logs.subscribe();
    let stream = futures_util::stream::iter(replay)
        .chain(futures_util::stream::unfold(query.follow.then_some(live), |live| async move {
            let mut live = live?;
            let frame = match live.recv().await {
                Ok(frame) => frame,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => crate::logbook::Frame::Gap,
                Err(_) => return None,
            };
            Some((frame, Some(live)))
        }))
        .map(|frame| {
            Ok::<_, Infallible>(format!("{}\n", serde_json::to_string(&frame).expect("Log frame serializes")))
        });
    ([(axum::http::header::CONTENT_TYPE, "application/x-ndjson")], Body::from_stream(stream)).into_response()
}

async fn start(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
    headers: HeaderMap,
    Json(body): Json<Start>,
) -> Result<Response, Failure> {
    let key = headers.get(crate::contract::IDEMPOTENCY_HEADER).and_then(|s| s.to_str().ok()).unwrap_or("");
    let process = agent.processes.start(body, key, claims.max_exec_seconds)?;
    Ok((StatusCode::CREATED, Json(process.view())).into_response())
}

async fn view(State(agent): State<Arc<Agent>>, Path(id): Path<String>) -> Result<Response, Failure> {
    Ok(Json(agent.processes.get(&id)?.view()).into_response())
}

async fn wait(State(agent): State<Arc<Agent>>, Path(id): Path<String>) -> Result<Response, Failure> {
    Ok(Json(agent.processes.get(&id)?.wait().await).into_response())
}

async fn stdin(
    State(agent): State<Arc<Agent>>,
    Path(id): Path<String>,
    bytes: axum::body::Bytes,
) -> Result<StatusCode, Failure> {
    agent.processes.get(&id)?.write(&bytes).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn close_stdin(State(agent): State<Arc<Agent>>, Path(id): Path<String>) -> Result<StatusCode, Failure> {
    agent.processes.get(&id)?.close_input_awaited().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Signal {
    signal: String,
}

fn signal_number(name: &str) -> Result<i32, Failure> {
    match name.trim_start_matches("SIG") {
        "TERM" => Ok(libc::SIGTERM),
        "KILL" => Ok(libc::SIGKILL),
        "INT" => Ok(libc::SIGINT),
        "HUP" => Ok(libc::SIGHUP),
        "USR1" => Ok(libc::SIGUSR1),
        "USR2" => Ok(libc::SIGUSR2),
        "WINCH" => Ok(libc::SIGWINCH),
        _ => Err(Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request", "Unsupported signal".into())),
    }
}

async fn signal(
    State(agent): State<Arc<Agent>>,
    Path(id): Path<String>,
    Json(body): Json<Signal>,
) -> Result<StatusCode, Failure> {
    agent.processes.get(&id)?.signal_awaited(signal_number(&body.signal)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Size {
    cols: u16,
    rows: u16,
}

async fn resize(
    State(agent): State<Arc<Agent>>,
    Path(id): Path<String>,
    Json(body): Json<Size>,
) -> Result<StatusCode, Failure> {
    agent.processes.get(&id)?.resize_awaited(body.cols, body.rows).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn output(State(agent): State<Arc<Agent>>, Path(id): Path<String>) -> Result<Response, Failure> {
    let process = agent.processes.get(&id)?;
    let (replay, live) = process.subscribe().await;
    let stream = futures_util::stream::iter(replay.into_iter().map(encode))
        .chain(futures_util::stream::unfold(live, |mut receiver| async move {
            receiver.recv().await.map(|event| (encode(event), receiver))
        }));
    Ok(([(axum::http::header::CONTENT_TYPE, "application/x-ndjson")], Body::from_stream(stream)).into_response())
}

fn encode(event: Event) -> Result<String, Infallible> {
    Ok(format!("{}\n", serde_json::to_string(&event).expect("Event serializes")))
}

async fn websocket(
    State(agent): State<Arc<Agent>>,
    Path(id): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Failure> {
    let process = agent.processes.get(&id)?;
    let active = agent.telemetry.active();
    Ok(upgrade.protocols([crate::contract::WEBSOCKET_PROTOCOL]).max_message_size(64 * 1024).on_upgrade(
        move |socket| async move {
            let _active = active;
            attached(socket, process).await
        },
    ))
}

async fn terminal(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Failure> {
    let command = agent.processes.shell()?;
    let process = agent.processes.start(
        Start {
            command,
            env: Default::default(),
            cwd: None,
            stdin: true,
            pty: true,
            cols: None,
            rows: None,
            timeout: None,
        },
        &uuid::Uuid::new_v4().to_string(),
        claims.max_exec_seconds,
    )?;
    let active = agent.telemetry.active();
    Ok(upgrade.protocols([crate::contract::WEBSOCKET_PROTOCOL]).max_message_size(64 * 1024).on_upgrade(
        move |socket| async move {
            let _active = active;
            attached(socket, process).await
        },
    ))
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Input {
    Stdin { bytes: String },
    Resize { cols: u16, rows: u16 },
    Signal { signal: String },
}

async fn attached(mut socket: WebSocket, process: Arc<Process>) {
    let (replay, mut live) = process.subscribe().await;
    for event in replay {
        if socket.send(Message::Text(serde_json::to_string(&event).unwrap().into())).await.is_err() {
            return;
        }
    }
    loop {
        tokio::select! {
            event = live.recv() => {
                let Some(event) = event else { break };
                if socket.send(Message::Text(serde_json::to_string(&event).unwrap().into())).await.is_err() { break; }
            },
            message = socket.recv() => {
                let text = match message {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    _ => break,
                };
                let result = match serde_json::from_str::<Input>(&text) {
                    Ok(Input::Stdin { bytes }) => match STANDARD.decode(bytes) {
                        Ok(bytes) => process.write(&bytes).await,
                        Err(_) => break,
                    },
                    Ok(Input::Resize { cols, rows }) => process.resize_awaited(cols, rows).await,
                    Ok(Input::Signal { signal }) => match signal_number(&signal) {
                        Ok(signal) => process.signal_awaited(signal).await, Err(_) => break,
                    },
                    Err(_) => break,
                };
                if result.is_err() { break; }
            }
        }
    }
}

async fn ports(State(agent): State<Arc<Agent>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"ports":agent.ports.list().await,"revision":agent.ports.revision()}))
}
#[derive(Deserialize)]
struct Port {
    port: u16,
}
async fn expose(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Port>,
) -> Result<Response, Failure> {
    Ok(Json(agent.ports.expose(body.port, claims.max_ports).await?).into_response())
}
async fn unexpose(State(agent): State<Arc<Agent>>, Path(port): Path<u16>) -> Result<StatusCode, Failure> {
    agent.ports.unexpose(port).await?;
    Ok(StatusCode::NO_CONTENT)
}

use futures_util::StreamExt;
