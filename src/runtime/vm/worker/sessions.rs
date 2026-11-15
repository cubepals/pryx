// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! HTTP response lifetimes own worker file services and cold-copy leases. Acquisition and
//! cleanup run in an independent task even when the caller vanishes before readiness.
//! Frontend retirement closes sessions and drains their operations before compute retirement.
use super::{authority::Authority, http, transport::Peer};
use crate::runtime::{ContainerRuntime, RuntimeError, files::Service};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{ConnectInfo, State},
    response::{IntoResponse, Response},
    routing::post,
};
use futures_util::stream;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, RwLock, oneshot, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub const PATH: &str = "/runtime/v1/sessions";

#[derive(Serialize, Deserialize)]
#[serde(tag = "session", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Files { request: crate::runtime::files::Request },
    Copy { sandbox: String, data: PathBuf },
    Close { id: Uuid },
}

pub(super) type Files = Arc<RwLock<Option<Service>>>;

enum Resource {
    Files(Files),
    Copy(Box<dyn Send>),
}

impl Resource {
    async fn close(self) -> Result<(), RuntimeError> {
        match self {
            Self::Files(files) => match files.write().await.take() {
                Some(service) => service.close().await,
                None => Err(RuntimeError::Unavailable("Worker file session is already closed".into())),
            },
            Self::Copy(lease) => {
                drop(lease);
                Ok(())
            }
        }
    }
}

pub(super) struct Session {
    pub(super) files: Option<Files>,
    pub(super) cancel: CancellationToken,
    done: watch::Receiver<Option<Result<(), RuntimeError>>>,
}

impl Session {
    async fn close(&self) -> Result<(), RuntimeError> {
        self.cancel.cancel();
        let mut done = self.done.clone();
        done.wait_for(Option::is_some)
            .await
            .map_err(|_| RuntimeError::Unavailable("Worker session cleanup ended without confirmation".into()))?
            .as_ref()
            .expect("completion was observed")
            .clone()
    }
}

#[derive(Clone)]
pub(super) struct Sessions {
    runtime: Arc<dyn ContainerRuntime>,
    pub(super) authority: Authority,
    entries: Arc<Mutex<HashMap<Uuid, Arc<Session>>>>,
}

impl Sessions {
    pub(super) fn new(runtime: Arc<dyn ContainerRuntime>, authority: Authority) -> Self {
        Self { runtime, authority, entries: Arc::default() }
    }

    pub(super) async fn get(&self, id: Uuid) -> Result<Arc<Session>, RuntimeError> {
        self.entries
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(|| RuntimeError::NotFound("Worker data session is absent".into()))
    }

    async fn acquire(&self, request: Request) -> Result<Resource, RuntimeError> {
        match request {
            Request::Files { request } => {
                Ok(Resource::Files(Arc::new(RwLock::new(Some(self.runtime.data_service(request).await?)))))
            }
            Request::Copy { sandbox, data } => Ok(Resource::Copy(self.runtime.data_copy_lease(&sandbox, &data).await?)),
            Request::Close { .. } => Err(RuntimeError::Other("Close cannot acquire a worker data session".into())),
        }
    }
}

// Dropping an unpolled or disconnected HTTP body must still retire its prepared resource.
struct Lifetime(Option<oneshot::Sender<()>>);

impl Drop for Lifetime {
    fn drop(&mut self) {
        self.0.take();
    }
}

async fn control(State(sessions): State<Sessions>, ConnectInfo(peer): ConnectInfo<Peer>, bytes: Bytes) -> Response {
    let permit = match sessions.authority.admit_peer(&peer).await {
        Ok(permit) => permit,
        Err(_) => return (StatusCode::SERVICE_UNAVAILABLE, "Worker frontend authority is unavailable").into_response(),
    };
    let request: Request = match serde_json::from_slice(&bytes) {
        Ok(request) => request,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid worker data session request").into_response(),
    };
    if let Request::Close { id } = request {
        return Json(match sessions.get(id).await {
            Ok(session) => session.close().await,
            Err(problem) => Err(problem),
        })
        .into_response();
    }
    let (release, released) = oneshot::channel();
    let lifetime = Lifetime(Some(release));
    let (ready, readiness) = oneshot::channel();
    let cancel = sessions.authority.cancellation().child_token();
    let closing = cancel.clone();
    tokio::spawn(async move {
        let _permit = permit;
        let resource = match sessions.acquire(request).await {
            Ok(resource) => resource,
            Err(problem) => {
                let _ = ready.send(Err(problem));
                return;
            }
        };
        let id = Uuid::new_v4();
        let (done, completion) = watch::channel(None);
        let session = Arc::new(Session {
            files: match &resource {
                Resource::Files(files) => Some(files.clone()),
                Resource::Copy(_) => None,
            },
            cancel: closing.clone(),
            done: completion,
        });
        sessions.entries.lock().await.insert(id, session);
        if ready.send(Ok(id)).is_ok() {
            tokio::select! {
                _ = released => (),
                () = closing.cancelled() => (),
            }
        }
        closing.cancel();
        let result = resource.close().await;
        if let Err(problem) = &result {
            tracing::warn!(%problem, "Worker data session cleanup failed");
        }
        let _ = done.send(Some(result));
        sessions.entries.lock().await.remove(&id);
    });
    let result = readiness
        .await
        .unwrap_or_else(|_| Err(RuntimeError::Unavailable("Worker data acquisition ended without an answer".into())));
    let mut encoded = serde_json::to_vec(&result).expect("worker readiness result encodes");
    encoded.push(b'\n');
    let failed = result.is_err();
    let body =
        stream::unfold((Some(encoded), lifetime, cancel, failed), |(first, lifetime, cancel, failed)| async move {
            if let Some(first) = first {
                return Some((Ok::<_, io::Error>(first), (None, lifetime, cancel, failed)));
            }
            if !failed {
                cancel.cancelled().await;
            }
            None
        });
    Body::from_stream(body).into_response()
}

pub(super) fn app(sessions: Sessions) -> Router {
    Router::new()
        .route(PATH, post(control))
        .layer(axum::extract::DefaultBodyLimit::max(http::MAX_CONTROL_BYTES))
        .with_state(sessions)
}
