// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! The worker's serving lifetime: accept exactly one frontend lifetime on its private endpoint,
//! then serve that frontend's admitted requests until it retires. Retirement runs the caller's
//! compute cleanup after every admitted operation drains, and the requests endpoint closes only
//! afterwards. Another frontend is never accepted by the same serving lifetime.
use super::{authority::Authority, http, transport::Endpoint};
use crate::runtime::vm::instance::VmRuntime;
use std::{
    future::Future,
    io,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    sync::Arc,
};
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
    // The lease outlives the served frontend, so no second server can bind meanwhile; another
    // frontend is refused at once and connects to this worker's successor.
    let _lease = lifetime.into_lease();
    let (authority, retired) = Authority::watch(connection, uid, retire)?;
    let cancel = CancellationToken::new();
    let server = tokio::spawn(http::serve(requests, app(authority), cancel.clone()));
    let result = retired.await.map_err(io::Error::other)?;
    cancel.cancel();
    server.await.map_err(io::Error::other)??;
    result
}

/// Where root's worker and pryxd meet: one private root-owned directory in the node's state.
pub fn paths(state: &Path) -> (PathBuf, PathBuf) {
    let directory = state.join("vm-worker");
    (directory.join("lifetime.sock"), directory.join("requests.sock"))
}

/// `pryxd vm-worker --check`: whether this host can serve root's VM tier, without serving it.
pub fn check(config: &crate::config::Config) -> anyhow::Result<()> {
    crate::runtime::vm::policy::worker_settings(config)?;
    Ok(())
}

/// `pryxd vm-worker`: serve root's VM backend to this node's pryxd until that pryxd retires
/// it. The process then exits, and its supervisor starts a successor for the next pryxd.
pub async fn run(config: &crate::config::Config) -> anyhow::Result<()> {
    let settings = crate::runtime::vm::policy::worker_settings(config)?;
    let (lifetime, requests) = paths(&config.state_dir);
    let directory = lifetime.parent().expect("worker sockets have a directory");
    match std::fs::DirBuilder::new().mode(0o700).create(directory) {
        Err(problem) if problem.kind() != io::ErrorKind::AlreadyExists => return Err(problem.into()),
        _ => (),
    }
    let uid = rustix::process::geteuid().as_raw();
    // Listening before the runtime exists lets pryxd's connection wait for its acknowledgement.
    let requests = Endpoint::bind(&requests, uid).await?;
    let lifetime = Endpoint::bind(&lifetime, uid).await?;
    let runtime = Arc::new(VmRuntime::new(&config.state_dir, settings).await?);
    serve(lifetime, requests, uid, runtime, async { Ok(()) }).await?;
    Ok(())
}
