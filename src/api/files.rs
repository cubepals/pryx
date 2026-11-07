// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Node data streams keep their workload guard until completion. They bypass JSON body limits,
//! never buffer whole files, and check host disk admission before each incoming chunk.

use axum::extract::DefaultBodyLimit;
use futures_util::TryStreamExt;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use super::*;
use crate::manager::files::file_error;
use crate::protocol::{AgentTokenRequest, FileList};

#[derive(Deserialize)]
struct DataPath {
    path: String,
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/workloads/{id}/files/content", get(read).put(write).layer(DefaultBodyLimit::disable()))
        .route("/v1/workloads/{id}/files/list", get(list))
        .route("/v1/workloads/{id}/files/stat", get(stat))
        .route("/v1/workloads/{id}/agent-token", post(token))
}

async fn read(
    State(app): State<AppState>,
    WorkloadPath(id): WorkloadPath,
    headers: HeaderMap,
    ApiQuery(path): ApiQuery<DataPath>,
) -> Result<Response, Response> {
    let access = app.manager.data_access(&id, epoch(&headers)?).await.map_err(IntoResponse::into_response)?;
    let file = access.directory.read(&path.path).map_err(|e| file_error(e).into_response())?;
    let length = file.metadata().map_err(|e| file_error(e).into_response())?.len();
    let guard = access.guard;
    let stream = ReaderStream::new(tokio::fs::File::from_std(file)).map(move |chunk| {
        let _held = &guard;
        chunk
    });
    Ok((
        [(header::CONTENT_LENGTH, length.to_string()), (header::CONTENT_TYPE, "application/octet-stream".into())],
        Body::from_stream(stream),
    )
        .into_response())
}

async fn write(
    State(app): State<AppState>,
    WorkloadPath(id): WorkloadPath,
    headers: HeaderMap,
    ApiQuery(path): ApiQuery<DataPath>,
    body: Body,
) -> Result<Response, Response> {
    let access = app.manager.data_access(&id, epoch(&headers)?).await.map_err(IntoResponse::into_response)?;
    let mut file =
        tokio::fs::File::from_std(access.directory.write(&path.path).map_err(|e| file_error(e).into_response())?);
    let mut input = body.into_data_stream();
    while let Some(chunk) = input
        .try_next()
        .await
        .map_err(|_| error_response(StatusCode::BAD_REQUEST, "file_interrupted", "File upload interrupted", None))?
    {
        app.manager.file_room(chunk.len() as u64).map_err(IntoResponse::into_response)?;
        file.write_all(&chunk).await.map_err(|e| file_error(e).into_response())?;
    }
    file.sync_data().await.map_err(|e| file_error(e).into_response())?;
    drop(access);
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn list(
    State(app): State<AppState>,
    WorkloadPath(id): WorkloadPath,
    headers: HeaderMap,
    ApiQuery(path): ApiQuery<DataPath>,
) -> Result<Response, Response> {
    let access = app.manager.data_access(&id, epoch(&headers)?).await.map_err(IntoResponse::into_response)?;
    Ok(Json(FileList { files: access.directory.list(&path.path).map_err(|e| file_error(e).into_response())? })
        .into_response())
}

async fn stat(
    State(app): State<AppState>,
    WorkloadPath(id): WorkloadPath,
    headers: HeaderMap,
    ApiQuery(path): ApiQuery<DataPath>,
) -> Result<Response, Response> {
    let access = app.manager.data_access(&id, epoch(&headers)?).await.map_err(IntoResponse::into_response)?;
    Ok(Json(access.directory.stat(&path.path).map_err(|e| file_error(e).into_response())?).into_response())
}

async fn token(
    State(app): State<AppState>,
    WorkloadPath(id): WorkloadPath,
    headers: HeaderMap,
    ApiJson(request): ApiJson<AgentTokenRequest>,
) -> Result<Response, Response> {
    Ok(Json(app.manager.agent_token(id, epoch(&headers)?, request).await.map_err(IntoResponse::into_response)?)
        .into_response())
}
