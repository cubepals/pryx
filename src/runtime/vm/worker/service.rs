// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! The worker's serving lifetime: accept exactly one frontend lifetime on its private endpoint,
//! then serve that frontend's admitted requests until it retires. Retirement runs the caller's
//! compute cleanup after every admitted operation drains, and the requests endpoint closes only
//! afterwards. Another frontend is never accepted by the same serving lifetime.
use super::{authority::Authority, http, transport::Endpoint};
use crate::runtime::vm::instance::VmRuntime;
use std::{future::Future, io, sync::Arc};
use tokio_util::sync::CancellationToken;

/// Serve the installed VM backend for one frontend.
pub async fn serve(
    lifetime: Endpoint,
    requests: Endpoint,
    uid: u32,
    runtime: Arc<VmRuntime>,
    retire: impl Future<Output = io::Result<()>> + Send + 'static,
) -> io::Result<()> {
    serve_app(lifetime, requests, uid, move |authority| super::router(runtime, authority), retire).await
}

pub(super) async fn serve_app(
    lifetime: Endpoint,
    requests: Endpoint,
    uid: u32,
    app: impl FnOnce(Authority) -> axum::Router,
    retire: impl Future<Output = io::Result<()>> + Send + 'static,
) -> io::Result<()> {
    let connection = lifetime.accept().await?;
    let (authority, retired) = Authority::watch(connection, uid, retire)?;
    let cancel = CancellationToken::new();
    let server = tokio::spawn(http::serve(requests, app(authority), cancel.clone()));
    let result = retired.await.map_err(io::Error::other)?;
    cancel.cancel();
    server.await.map_err(io::Error::other)??;
    // The lifetime endpoint's lease outlives the served frontend, so no second one can bind.
    drop(lifetime);
    result
}
