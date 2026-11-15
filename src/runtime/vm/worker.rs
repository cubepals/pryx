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
//! - `http.rs`: maintained HTTP framing, bounded control calls and one submission per operation.
//! - `sessions.rs`: response-owned file and cold-copy lifetimes with drained cleanup.
//! - `session_tests.rs`: actual HTTP streams, backend locks and interrupted cleanup.
//! - `streams.rs`: typed log/event observation streams and frontend-owned cancellation.
//! - `transport.rs`: private socket ownership, single server lease and kernel peer admission.
pub mod authority;
pub mod commands;
pub mod data_client;
pub mod file_operations;
pub mod http;
#[cfg(test)]
mod session_tests;
pub mod sessions;
pub mod streams;
pub mod transport;

pub fn router(runtime: std::sync::Arc<super::instance::VmRuntime>, authority: authority::Authority) -> axum::Router {
    commands::router(runtime.clone(), authority.clone())
        .merge(streams::router(runtime.clone(), authority.clone()))
        .merge(sessions::router(runtime, authority))
}
