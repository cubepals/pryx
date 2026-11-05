// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Streamed persistent data and bounded filesystem notifications. Paths are opened through the
//! shared descriptor walk; a request cannot substitute a symlink or make the agent open a FIFO.

use std::io;
use std::os::fd::AsRawFd;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Extension, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::TryStreamExt;
use notify::{RecursiveMode, Watcher};
use serde::Deserialize;
use tokio_util::io::{ReaderStream, StreamReader};

use crate::context::{Agent, Failure};

#[derive(Deserialize)]
struct Path {
    path: String,
}
#[derive(Deserialize)]
struct Move {
    from: String,
    to: String,
}
#[derive(Deserialize)]
struct Transfer {
    path: String,
    url: String,
}

pub(crate) fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/_pryx/files/content", get(read).put(write).delete(delete).layer(DefaultBodyLimit::disable()))
        .route("/_pryx/files/list", get(list))
        .route("/_pryx/files/stat", get(stat))
        .route("/_pryx/files/mkdir", post(mkdir))
        .route("/_pryx/files/move", post(move_to))
        .route("/_pryx/files/watch", get(watch))
        .route("/_pryx/files/upload", post(upload))
        .route("/_pryx/files/download", post(download))
        .route("/_pryx/files/token", post(file_token))
}

async fn file_token(
    State(agent): State<Arc<Agent>>,
    Extension(parent): Extension<crate::auth::Claims>,
    Json(file): Json<crate::auth::FileAccess>,
) -> Result<Response, Failure> {
    let now = crate::telemetry::now();
    let expires_at = parent.expires_at.min(now + crate::auth::FILE_TOKEN_LIFETIME_SECONDS);
    let claims = crate::auth::Claims {
        scope: crate::auth::Scope::File,
        issued_at: now,
        expires_at,
        file: Some(file.clone()),
        ..parent
    };
    let token = agent
        .key
        .sign(&claims)
        .map_err(|_| Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request", "Cannot issue file token".into()))?;
    let mut url = reqwest::Url::parse("http://agent/_pryx/files/content").expect("static URL");
    url.query_pairs_mut().append_pair("path", &file.path).append_pair("token", &token);
    Ok(Json(
        serde_json::json!({"url":format!("{}?{}",url.path(),url.query().unwrap_or_default()), "expiresAt":expires_at}),
    )
    .into_response())
}

async fn read(State(agent): State<Arc<Agent>>, Query(path): Query<Path>) -> Result<Response, Failure> {
    let file = tokio::fs::File::from_std(agent.data.read(&path.path)?);
    let length = file.metadata().await?.len();
    Ok((
        [
            (axum::http::header::CONTENT_LENGTH, length.to_string()),
            (axum::http::header::CONTENT_TYPE, "application/octet-stream".into()),
        ],
        Body::from_stream(ReaderStream::new(file)),
    )
        .into_response())
}

async fn write(State(agent): State<Arc<Agent>>, Query(path): Query<Path>, body: Body) -> Result<StatusCode, Failure> {
    let mut file = tokio::fs::File::from_std(agent.data.write(&path.path)?);
    let stream = body.into_data_stream().map_err(io::Error::other);
    let mut input = StreamReader::new(stream);
    tokio::io::copy(&mut input, &mut file).await?;
    file.sync_data().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list(State(agent): State<Arc<Agent>>, Query(path): Query<Path>) -> Result<Response, Failure> {
    Ok(Json(serde_json::json!({"files":agent.data.list(&path.path)?})).into_response())
}
async fn stat(State(agent): State<Arc<Agent>>, Query(path): Query<Path>) -> Result<Response, Failure> {
    Ok(Json(agent.data.stat(&path.path)?).into_response())
}
async fn mkdir(State(agent): State<Arc<Agent>>, Json(path): Json<Path>) -> Result<StatusCode, Failure> {
    agent.data.mkdir(&path.path)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn move_to(State(agent): State<Arc<Agent>>, Json(body): Json<Move>) -> Result<StatusCode, Failure> {
    agent.data.move_to(&body.from, &body.to)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete(State(agent): State<Arc<Agent>>, Query(path): Query<Path>) -> Result<StatusCode, Failure> {
    agent.data.delete(&path.path)?;
    Ok(StatusCode::NO_CONTENT)
}

fn client(url: &str) -> Result<reqwest::Client, Failure> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request", "Invalid transfer URL".into()))?;
    if !matches!(parsed.scheme(), "https" | "http") || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_request",
            "Transfer requires an HTTP URL without user information".into(),
        ));
    }
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(transfer_failed)
}

fn transfer_failed(_error: reqwest::Error) -> Failure {
    // The upstream URL can contain a signed credential. It never enters an error or log.
    Failure(StatusCode::BAD_GATEWAY, "transfer_failed", "Remote file transfer failed".into())
}

async fn upload(State(agent): State<Arc<Agent>>, Json(body): Json<Transfer>) -> Result<StatusCode, Failure> {
    let response = client(&body.url)?
        .get(&body.url)
        .send()
        .await
        .map_err(transfer_failed)?
        .error_for_status()
        .map_err(transfer_failed)?;
    let mut input = StreamReader::new(response.bytes_stream().map_err(io::Error::other));
    let mut file = tokio::fs::File::from_std(agent.data.write(&body.path)?);
    tokio::io::copy(&mut input, &mut file).await?;
    file.sync_data().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn download(State(agent): State<Arc<Agent>>, Json(body): Json<Transfer>) -> Result<StatusCode, Failure> {
    let file = tokio::fs::File::from_std(agent.data.read(&body.path)?);
    let length = file.metadata().await?.len();
    client(&body.url)?
        .put(&body.url)
        .header(reqwest::header::CONTENT_LENGTH, length)
        .body(reqwest::Body::wrap_stream(ReaderStream::new(file)))
        .send()
        .await
        .map_err(transfer_failed)?
        .error_for_status()
        .map_err(transfer_failed)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn watch(
    State(agent): State<Arc<Agent>>,
    Query(path): Query<Path>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Failure> {
    let directory = agent.data.directory_file(&path.path)?;
    let watched = std::path::PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
    let (sender, mut events) = tokio::sync::mpsc::channel(64);
    let overflow = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let dropped = overflow.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event
            && sender.try_send(event).is_err()
        {
            dropped.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    })
    .map_err(|_| io::Error::other("Cannot watch directory"))?;
    watcher
        .configure(notify::Config::default().with_follow_symlinks(false))
        .map_err(|_| io::Error::other("Cannot configure directory watch"))?;
    watcher.watch(&watched, RecursiveMode::Recursive).map_err(|_| io::Error::other("Cannot watch directory"))?;
    let active = agent.telemetry.active();
    Ok(upgrade.protocols(["pryx"]).on_upgrade(move |mut socket| async move {
        let _active = active;
        if socket.send(Message::Text(serde_json::json!({"type":"ready"}).to_string().into())).await.is_err() { return; }
        let (_watcher, _directory) = (watcher, directory);
        loop {
            tokio::select! {
                event = events.recv() => {
                    let Some(event) = event else { break };
                    let paths = event.paths.iter().filter_map(|p| p.strip_prefix(&watched).ok()).map(|p| p.to_string_lossy()).collect::<Vec<_>>();
                    let message = serde_json::json!({"type":"change","kind":format!("{:?}",event.kind),"paths":paths,"overflow":overflow.swap(false,std::sync::atomic::Ordering::Relaxed)});
                    if socket.send(Message::Text(message.to_string().into())).await.is_err() { break; }
                },
                message = socket.recv() => {
                    if !matches!(message, Some(Ok(Message::Ping(_) | Message::Pong(_)))) { break; }
                }
            }
        }
    }))
}
