// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Authenticated language-kernel routes stream bounded events without a control-plane hop.
//! MCP calls reuse these owners and expose kernel-produced images as MCP image content.

use super::{
    events::{KernelComplete, KernelEvent, KernelOpen, KernelRun, KernelView},
    runtime::{self, Session, failure},
};
use crate::{
    auth::Claims,
    context::{Agent, Failure},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
#[cfg(feature = "mcp")]
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::mpsc;

pub(crate) fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/_pryx/kernels", get(list).post(open))
        .route("/_pryx/kernels/{id}", axum::routing::delete(close))
        .route("/_pryx/kernels/{id}/execute", post(execute))
        .route("/_pryx/kernels/{id}/interrupt", post(interrupt))
        .route("/_pryx/kernels/{id}/restart", post(restart))
        .route("/_pryx/kernels/{id}/complete", post(complete))
}

async fn list(State(agent): State<Arc<Agent>>) -> Json<Value> {
    Json(json!({"kernels":agent.processes.kernels.list()}))
}
async fn open(State(agent): State<Arc<Agent>>, Json(body): Json<KernelOpen>) -> Result<Json<KernelView>, Failure> {
    Ok(Json(agent.processes.kernels.open(&agent.processes, body.language, body.context).await?.view()))
}
async fn close(State(agent): State<Arc<Agent>>, Path(id): Path<String>) -> Result<StatusCode, Failure> {
    agent.processes.kernels.close(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn interrupt(State(agent): State<Arc<Agent>>, Path(id): Path<String>) -> Result<StatusCode, Failure> {
    agent.processes.kernels.get(&id)?.session()?.interrupt()?;
    Ok(StatusCode::NO_CONTENT)
}
async fn restart(State(agent): State<Arc<Agent>>, Path(id): Path<String>) -> Result<Json<KernelView>, Failure> {
    Ok(Json(agent.processes.kernels.get(&id)?.restart(&agent.processes).await?))
}
async fn complete(
    State(agent): State<Arc<Agent>>,
    Path(id): Path<String>,
    Json(body): Json<KernelComplete>,
) -> Result<Json<Value>, Failure> {
    Ok(Json(agent.processes.kernels.get(&id)?.session()?.complete(body.code).await?))
}

fn deadline(body: &KernelRun, claims: &Claims) -> Result<Duration, Failure> {
    let ceiling = claims.max_exec_seconds.map_or(crate::contract::MAX_WAIT, |n| Duration::from_secs(n.into()));
    let seconds = body.timeout.unwrap_or(runtime::DEFAULT_RUN.min(ceiling).as_secs_f64());
    if !seconds.is_finite()
        || seconds <= 0.0
        || seconds > ceiling.as_secs_f64()
        || seconds > crate::contract::MAX_WAIT.as_secs_f64()
        || body.code.contains('\0')
    {
        return Err(failure("invalid_request", "Invalid kernel execution or timeout"));
    }
    Ok(Duration::from_secs_f64(seconds))
}

fn begin(
    agent: &Agent,
    session: Arc<Session>,
    body: KernelRun,
    claims: &Claims,
) -> Result<mpsc::Receiver<KernelEvent>, Failure> {
    let limit = deadline(&body, claims)?;
    if !session.process.view().running {
        return Err(failure("kernel_died", "Kernel has exited; restart it explicitly"));
    }
    let permit = session
        .jobs
        .clone()
        .try_acquire_owned()
        .map_err(|_| failure("kernel_busy", "Kernel execution queue is full"))?;
    let (sender, receiver) = mpsc::channel(16);
    let active = agent.telemetry.active();
    tokio::spawn(async move {
        let (_permit, _active) = (permit, active);
        session.execute(body.code, limit, sender).await;
    });
    Ok(receiver)
}

async fn execute(
    State(agent): State<Arc<Agent>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<String>,
    Json(body): Json<KernelRun>,
) -> Result<Response, Failure> {
    let session = agent.processes.kernels.get(&id)?.session()?;
    let receiver = begin(&agent, session, body, &claims)?;
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|event| {
            (
                Ok::<_, Infallible>(format!("{}\n", serde_json::to_string(&event).expect("KernelEvent serializes"))),
                receiver,
            )
        })
    });
    Ok(([(axum::http::header::CONTENT_TYPE, "application/x-ndjson")], Body::from_stream(stream)).into_response())
}

#[cfg(feature = "mcp")]
pub(crate) async fn mcp(
    agent: &Agent,
    claims: &Claims,
    language: super::events::Language,
    args: &Value,
) -> Result<Value, Failure> {
    let context = args.get("context").and_then(Value::as_str).unwrap_or_default().to_owned();
    let body: KernelRun = serde_json::from_value(json!({"code":args["code"],"timeout":args.get("timeout")}))
        .map_err(|_| failure("invalid_request", "Kernel tool requires code and an optional timeout"))?;
    let kernel = agent.processes.kernels.open(&agent.processes, language, context).await?;
    let mut receiver = begin(agent, kernel.session()?, body, claims)?;
    let mut events = Vec::new();
    let mut images = Vec::new();
    let mut failed = false;
    while let Some(event) = receiver.recv().await {
        failed |= matches!(event, KernelEvent::Error { .. });
        if let KernelEvent::Display { data, .. } | KernelEvent::Result { data, .. } = &event {
            for mime in ["image/png", "image/jpeg"] {
                if let Some(encoded) = data.get(mime).and_then(Value::as_str) {
                    STANDARD
                        .decode(encoded)
                        .map_err(|_| failure("kernel_protocol", "Kernel returned an invalid image"))?;
                    images.push(json!({"type":"image","mimeType":mime,"data":encoded}));
                }
            }
        }
        events.push(event);
    }
    let mut content =
        vec![json!({"type":"text","text":serde_json::to_string(&events).expect("KernelEvent serializes")})];
    content.extend(images);
    Ok(json!({"content":content,"isError":failed}))
}
