// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Separate host-owned VM authority. Its private Unix transport authenticates kernel peers;
//! this foundation does not register a backend or grant the main daemon more privileges.
//!
//! Parts (`worker/`):
//! - `authority.rs`: frontend lifetime, same-process admission and drained compute retirement.
//! - `commands.rs`: typed runtime commands and cancellation-owned unary dispatch.
//! - `data_client.rs`: response lifetimes and confirmed file cleanup on the frontend.
//! - `file_operations.rs`: admitted fixed file operations with streamed bodies and held cleanup.
//! - `frontend.rs`: the main daemon's backend that sends each runtime verb to the worker once.
//! - `frontend_tests.rs`: actual sockets, one submission per verb and lifetime retirement.
//! - `http.rs`: maintained HTTP framing, bounded control calls and one submission per operation.
//! - `service.rs`: one accepted frontend lifetime served until it retires.
//! - `sessions.rs`: response-owned file and cold-copy lifetimes with drained cleanup.
//! - `session_tests.rs`: actual HTTP streams, backend locks and interrupted cleanup.
//! - `streams.rs`: typed log/event observation streams and frontend-owned cancellation.
//! - `transport.rs`: private socket ownership, single server lease and kernel peer admission.
pub mod authority;
pub mod commands;
pub mod data_client;
pub mod file_operations;
pub mod frontend;
#[cfg(test)]
mod frontend_tests;
pub mod http;
pub mod service;
#[cfg(test)]
mod session_tests;
pub mod sessions;
pub mod streams;
pub mod transport;

use crate::runtime::ContainerRuntime;
use std::sync::Arc;

/// Every worker route for one frontend authority. Tests serve another real backend through it.
fn app(runtime: Arc<dyn ContainerRuntime>, authority: authority::Authority) -> axum::Router {
    let sessions = sessions::Sessions::new(runtime.clone(), authority.clone());
    commands::app(runtime.clone(), authority.clone())
        .merge(streams::app(runtime, authority))
        .merge(sessions::app(sessions.clone()))
        .merge(file_operations::router(sessions))
}

/// An installed worker serves only the VM backend.
pub fn router(runtime: Arc<super::instance::VmRuntime>, authority: authority::Authority) -> axum::Router {
    app(runtime, authority)
}
