// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! HTTP state and error responses compose agent services without a dependency on the router.

use crate::{
    auth::SigningKey,
    paths::DataDir,
    ports::{PortError, Ports},
    process::{ProcessError, Processes},
};
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub struct Agent {
    pub vm_resources: bool,
    pub sandbox: String,
    pub epoch: u64,
    pub key: SigningKey,
    pub public_key: String,
    /// A fork's own identity, once its trusted init handed it over (`vm_sleep.rs`).
    pub forked: std::sync::RwLock<Option<Arc<Forked>>>,
    pub retirement_nonce: std::sync::Mutex<Option<String>>,
    pub processes: Processes,
    pub file_tasks: tokio_util::task::TaskTracker,
    pub shells: crate::shell_state::Shells,
    pub data: DataDir,
    pub ports: Arc<Ports>,
    pub telemetry: Arc<crate::telemetry::Telemetry>,
    pub logs: Arc<crate::logbook::Logbook>,
}

/// The identity a VM fork with memory answers as in place of its source's.
pub struct Forked {
    pub sandbox: String,
    pub epoch: u64,
    pub key: SigningKey,
    pub public_key: String,
}

impl Agent {
    /// Runs `answer` with the sandbox, epoch, signing key and public key this agent answers as:
    /// a fork's, once it has one, otherwise its own.
    pub fn as_identity<T>(&self, answer: impl FnOnce(&str, u64, &SigningKey, &str) -> T) -> T {
        let forked = self.forked.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        match forked {
            Some(forked) => answer(&forked.sandbox, forked.epoch, &forked.key, &forked.public_key),
            None => answer(&self.sandbox, self.epoch, &self.key, &self.public_key),
        }
    }
}

pub struct Failure(pub StatusCode, pub &'static str, pub String);

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({"error":{"code":self.1,"message":self.2}}))).into_response()
    }
}

impl From<ProcessError> for Failure {
    fn from(error: ProcessError) -> Self {
        let (status, code) = match error {
            ProcessError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            ProcessError::KeyConflict => (StatusCode::CONFLICT, "idempotency_key_reused"),
            ProcessError::Full => (StatusCode::TOO_MANY_REQUESTS, "process_capacity"),
            ProcessError::Invalid(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"),
            ProcessError::Io(_) => (StatusCode::UNPROCESSABLE_ENTITY, "process_failed"),
        };
        Self(status, code, error.to_string())
    }
}

impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        let status = if error.kind() == std::io::ErrorKind::NotFound {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::UNPROCESSABLE_ENTITY
        };
        Self(status, "file_failed", error.to_string())
    }
}

impl From<PortError> for Failure {
    fn from(error: PortError) -> Self {
        let (status, code) = match error {
            PortError::NoSparePort => (StatusCode::CONFLICT, "no_spare_port"),
            PortError::Invalid => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"),
            PortError::Persistence => (StatusCode::INTERNAL_SERVER_ERROR, "port_checkpoint_failed"),
        };
        Self(status, code, error.to_string())
    }
}
