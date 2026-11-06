// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Streamed remote file transfers. Local path confinement remains in DataDir; watches and
//! direct file operations remain in files.rs.

use crate::context::{Agent, Failure};
use axum::{Json, extract::State, http::StatusCode};
use futures_util::TryStreamExt;
use serde::Deserialize;
use std::{io, sync::Arc};
use tokio_util::io::{ReaderStream, StreamReader};

#[derive(Deserialize)]
pub(crate) struct Transfer {
    path: String,
    url: String,
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

pub(crate) async fn upload(State(agent): State<Arc<Agent>>, Json(body): Json<Transfer>) -> Result<StatusCode, Failure> {
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

pub(crate) async fn download(
    State(agent): State<Arc<Agent>>,
    Json(body): Json<Transfer>,
) -> Result<StatusCode, Failure> {
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
