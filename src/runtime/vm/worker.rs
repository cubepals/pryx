// SPDX-License-Identifier: FSL-1.1-ALv2
//! Separate host-owned VM authority. Its private Unix transport authenticates kernel peers;
//! this foundation does not register a backend or grant the main daemon more privileges.
//!
//! Parts (`worker/`):
//! - `authority.rs`: frontend lifetime, same-process admission and drained compute retirement.
//! - `commands.rs`: typed runtime commands and cancellation-owned unary dispatch.
//! - `http.rs`: maintained HTTP framing, bounded control calls and one submission per operation.
//! - `streams.rs`: typed log/event observation streams and frontend-owned cancellation.
//! - `transport.rs`: private socket ownership, single server lease and kernel peer admission.
pub mod authority;
pub mod commands;
pub mod http;
pub mod streams;
pub mod transport;

pub fn router(runtime: std::sync::Arc<super::instance::VmRuntime>, authority: authority::Authority) -> axum::Router {
    commands::router(runtime.clone(), authority.clone()).merge(streams::router(runtime, authority))
}
