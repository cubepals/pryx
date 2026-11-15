// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Forward the file owner's four fixed operations within an admitted worker session.
//! Input and output stream without control-message limits. The session's read guard stays
//! with the response body so cleanup cannot retire a maintenance guest during an operation.
use super::{sessions::Sessions, transport::Peer};
use crate::runtime::{RuntimeError, files::Operation};
use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, Path, Query, State},
    response::{IntoResponse, Response},
    routing::post,
};
use futures_util::{StreamExt, stream};
use http_body_util::BodyExt;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use std::io;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub const ERROR_HEADER: &str = "x-pryx-worker-file-error";
const ROUTE: &str = "/runtime/v1/sessions/{id}/files";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub operation: Operation,
    pub path: String,
}

pub fn route(id: Uuid, request: &Request) -> Result<String, RuntimeError> {
    let mut url = reqwest::Url::parse(&format!("http://pryx-worker/runtime/v1/sessions/{id}/files"))
        .map_err(|_| RuntimeError::Other("Invalid worker file route".into()))?;
    url.query_pairs_mut()
        .append_pair("operation", serde_json::to_value(request.operation).unwrap().as_str().unwrap())
        .append_pair("path", &request.path);
    Ok(format!("{}?{}", url.path(), url.query().unwrap_or_default()))
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Worker data session retired during transfer")
}

fn input(body: Body, cancel: CancellationToken) -> crate::http_client::Body {
    let body = std::sync::Mutex::new(body.into_data_stream());
    let body = stream::poll_fn(move |context| body.lock().unwrap().poll_next_unpin(context));
    let chunks = stream::unfold((body, cancel, false), |(mut body, cancel, finished)| async move {
        if finished {
            return None;
        }
        let item = tokio::select! {
            () = cancel.cancelled() => Err(interrupted()),
            item = body.next() => item?.map_err(io::Error::other),
        };
        let finished = item.is_err();
        Some((item.map(hyper::body::Frame::data), (body, cancel, finished)))
    });
    BodyExt::boxed(http_body_util::StreamBody::new(chunks))
}

async fn forward(sessions: Sessions, id: Uuid, request: Request, body: Body) -> Result<Response, RuntimeError> {
    let session = sessions.get(id).await?;
    let files =
        session.files.as_ref().ok_or_else(|| RuntimeError::Other("Worker session is not a file service".into()))?;
    let guard = files.clone().read_owned().await;
    if session.cancel.is_cancelled() {
        return Err(RuntimeError::Unavailable("Worker file session is retiring".into()));
    }
    let service = guard.as_ref().ok_or_else(|| RuntimeError::Unavailable("Worker file session is closed".into()))?;
    let response = service.request(request.operation, &request.path, input(body, session.cancel.clone())).await?;
    let (mut parts, body) = response.into_parts();
    parts.headers.remove(hyper::header::CONNECTION);
    parts.headers.remove(hyper::header::TRANSFER_ENCODING);
    parts.headers.remove(ERROR_HEADER);
    let (output, chunks) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        let _guard = guard;
        let mut body = body.into_data_stream();
        loop {
            let item = tokio::select! {
                () = session.cancel.cancelled() => break,
                () = output.closed() => break,
                item = body.next() => match item {
                    Some(item) => item.map_err(io::Error::other),
                    None => break,
                },
            };
            let finished = item.is_err();
            tokio::select! {
                () = session.cancel.cancelled() => break,
                result = output.send(item) => if result.is_err() { break; },
            }
            if finished {
                break;
            }
        }
    });
    let chunks = stream::unfold(chunks, |mut chunks| async move { chunks.recv().await.map(|item| (item, chunks)) });
    Ok(Response::from_parts(parts, Body::from_stream(chunks)))
}

fn failed(problem: RuntimeError) -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, [(ERROR_HEADER, "1")], Json(problem)).into_response()
}

async fn request(
    State(sessions): State<Sessions>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    Path(id): Path<Uuid>,
    Query(request): Query<Request>,
    body: Body,
) -> Response {
    let permit = match sessions.authority.admit_peer(&peer).await {
        Ok(permit) => permit,
        Err(_) => return failed(RuntimeError::Unavailable("Worker frontend authority is unavailable".into())),
    };
    // An interrupted HTTP handler cannot abandon a submitted operation or its session guard.
    match tokio::spawn(async move {
        let _permit = permit;
        forward(sessions, id, request, body).await
    })
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(problem)) => failed(problem),
        Err(_) => failed(RuntimeError::Unavailable("Worker file operation ended without an answer".into())),
    }
}

pub(super) fn router(sessions: Sessions) -> Router {
    Router::new().route(ROUTE, post(request)).layer(axum::extract::DefaultBodyLimit::disable()).with_state(sessions)
}
