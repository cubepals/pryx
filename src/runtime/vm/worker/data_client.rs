// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Client ownership of private worker data sessions. The readiness response stays open
//! through file access or a cold copy. File close waits for confirmed backend cleanup;
//! dropping either session also closes its lifetime without replaying an uncertain request.
use super::{file_operations, http, sessions};
use crate::{
    http_client,
    runtime::{RuntimeError, files},
};
use async_trait::async_trait;
use http_body_util::BodyExt;
use hyper::{Method, Response, body::Incoming};
use std::{path::PathBuf, time::Duration};
use tokio::time::Instant;
use uuid::Uuid;

#[derive(Clone)]
pub struct Client {
    pub socket: PathBuf,
    pub uid: u32,
    pub timeout: Duration,
}

pub struct Lease {
    id: Uuid,
    client: Client,
    _lifetime: Incoming,
}

fn invalid() -> RuntimeError {
    RuntimeError::Unavailable("Invalid worker data session readiness".into())
}

impl Client {
    pub async fn open(&self, request: sessions::Request) -> Result<Lease, RuntimeError> {
        if matches!(request, sessions::Request::Close { .. }) {
            return Err(RuntimeError::Other("Close cannot open a worker session".into()));
        }
        let bytes = serde_json::to_vec(&request).map_err(|_| invalid())?;
        if bytes.len() > http::MAX_CONTROL_BYTES {
            return Err(RuntimeError::Other("Worker data session request exceeds its bound".into()));
        }
        let deadline = Instant::now() + self.timeout;
        let response = http::send(
            &self.socket,
            self.uid,
            Method::POST,
            sessions::PATH,
            http_client::full(bytes),
            "application/json",
            deadline,
        )
        .await?;
        if !response.status().is_success() {
            return Err(RuntimeError::Unavailable(format!("Worker data session refused HTTP {}", response.status())));
        }
        let mut body = response.into_body();
        let id = tokio::time::timeout_at(deadline, async {
            let mut bytes = Vec::new();
            loop {
                let frame = body.frame().await.ok_or_else(invalid)?.map_err(|_| invalid())?;
                let data = frame.into_data().map_err(|_| invalid())?;
                if data.len() > http::MAX_CONTROL_BYTES.saturating_sub(bytes.len()) {
                    return Err(invalid());
                }
                bytes.extend_from_slice(&data);
                if bytes.contains(&b'\n') {
                    return serde_json::from_slice::<Result<Uuid, RuntimeError>>(&bytes).map_err(|_| invalid())?;
                }
            }
        })
        .await
        .map_err(|_| RuntimeError::Timeout("Worker data session readiness exceeded its deadline".into()))??;
        Ok(Lease { id, client: self.clone(), _lifetime: body })
    }

    pub async fn files(&self, request: files::Request) -> Result<files::Service, RuntimeError> {
        let lease = self.open(sessions::Request::Files { request }).await?;
        let transport = Transport { id: lease.id, client: self.clone() };
        Ok(files::Service::with_transport(transport, async move { lease.close().await }))
    }

    pub async fn copy(&self, sandbox: &str, data: &std::path::Path) -> Result<Box<dyn Send>, RuntimeError> {
        Ok(Box::new(self.open(sessions::Request::Copy { sandbox: sandbox.into(), data: data.into() }).await?))
    }
}

impl Lease {
    pub async fn close(self) -> Result<(), RuntimeError> {
        let answer: Result<(), RuntimeError> = http::control_at(
            &self.client.socket,
            self.client.uid,
            sessions::PATH,
            &sessions::Request::Close { id: self.id },
            Instant::now() + self.client.timeout,
        )
        .await?;
        answer
    }
}

struct Transport {
    id: Uuid,
    client: Client,
}

#[async_trait]
impl files::Transport for Transport {
    async fn request(
        &self,
        operation: files::Operation,
        path: &str,
        body: http_client::Body,
    ) -> Result<Response<Incoming>, RuntimeError> {
        let route = file_operations::route(self.id, &file_operations::Request { operation, path: path.into() })?;
        let deadline = Instant::now() + self.client.timeout;
        let response = http::stream(
            &self.client.socket,
            self.client.uid,
            Method::POST,
            &route,
            body,
            "application/octet-stream",
            deadline,
        )
        .await?;
        if !response.headers().contains_key(file_operations::ERROR_HEADER) {
            return Ok(response);
        }
        let bytes = tokio::time::timeout_at(deadline, http_client::read_body(response, http::MAX_CONTROL_BYTES))
            .await
            .map_err(|_| RuntimeError::Timeout("Worker file refusal exceeded its deadline".into()))?
            .map_err(|_| RuntimeError::Unavailable("Worker file refusal is incomplete".into()))?;
        Err(serde_json::from_slice(&bytes)
            .map_err(|_| RuntimeError::Unavailable("Invalid worker file refusal".into()))?)
    }
}
