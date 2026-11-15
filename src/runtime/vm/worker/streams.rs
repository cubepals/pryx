// SPDX-License-Identifier: FSL-1.1-ALv2
//! Typed worker log/event streams use Serde JSON lines and Tokio's bounded line codec.
//! They hold frontend admission until completion or cancellation; transport loss never
//! replays a command. This streams observations only, never file bodies or sandbox mutations.
use super::{authority::Authority, http, transport::Peer};
use crate::{
    http_client,
    runtime::{ContainerRuntime, LogChunk, LogOptions, RuntimeError, RuntimeEvent, vm::instance::VmRuntime},
};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{ConnectInfo, State},
    response::{IntoResponse, Response},
    routing::post,
};
use futures_util::{
    StreamExt, TryStreamExt,
    stream::{self, BoxStream},
};
use http_body_util::BodyExt;
use hyper::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::time::Instant;
use tokio_util::{
    codec::{FramedRead, LinesCodec},
    io::StreamReader,
};

pub const PATH: &str = "/runtime/v1/streams";

#[derive(Serialize, Deserialize)]
#[serde(tag = "stream", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Logs { name: String, options: LogOptions },
    Events { filters: Vec<String> },
}

#[derive(Serialize, Deserialize)]
pub enum Item {
    Log(LogChunk),
    Event(RuntimeEvent),
}

#[derive(Clone)]
struct Dispatch {
    runtime: Arc<dyn ContainerRuntime>,
    authority: Authority,
}

fn encode(item: Result<Item, RuntimeError>) -> (Vec<u8>, bool) {
    match serde_json::to_vec(&item) {
        Ok(mut bytes) if bytes.len() <= http::MAX_CONTROL_BYTES => {
            bytes.push(b'\n');
            (bytes, false)
        }
        _ => {
            let problem: Result<Item, RuntimeError> =
                Err(RuntimeError::Other("Worker stream frame exceeds its bound or cannot encode".into()));
            let mut bytes = serde_json::to_vec(&problem).expect("static worker stream error encodes");
            bytes.push(b'\n');
            (bytes, true)
        }
    }
}

async fn open(State(dispatch): State<Dispatch>, ConnectInfo(peer): ConnectInfo<Peer>, bytes: Bytes) -> Response {
    let admitted = match dispatch.authority.admit_peer(&peer).await {
        Ok(admitted) => admitted,
        Err(_) => return (StatusCode::SERVICE_UNAVAILABLE, "Worker frontend authority is unavailable").into_response(),
    };
    let request: Request = match serde_json::from_slice(&bytes) {
        Ok(request) => request,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid worker stream request").into_response(),
    };
    let source: BoxStream<'static, Result<Item, RuntimeError>> = match request {
        Request::Logs { name, options } => {
            dispatch.runtime.logs(&name, options).map(|item| item.map(Item::Log)).boxed()
        }
        Request::Events { filters } => dispatch.runtime.events(&filters).map(|item| item.map(Item::Event)).boxed(),
    };
    let state = (source, admitted, dispatch.authority.cancellation(), dispatch.runtime, false);
    let frames = stream::unfold(state, |(mut source, admitted, cancel, runtime, finished)| async move {
        if finished {
            return None;
        }
        let item = tokio::select! {
            () = cancel.cancelled() => return None,
            item = source.next() => item?,
        };
        let (bytes, finished) = encode(item);
        Some((Ok::<_, io::Error>(bytes), (source, admitted, cancel, runtime, finished)))
    });
    Body::from_stream(frames).into_response()
}

fn app(runtime: Arc<dyn ContainerRuntime>, authority: Authority) -> Router {
    Router::new()
        .route(PATH, post(open))
        .layer(axum::extract::DefaultBodyLimit::max(http::MAX_CONTROL_BYTES))
        .with_state(Dispatch { runtime, authority })
}

pub fn router(runtime: Arc<VmRuntime>, authority: Authority) -> Router {
    app(runtime, authority)
}

pub fn receive(
    path: PathBuf,
    uid: u32,
    request: Request,
    deadline: Instant,
) -> BoxStream<'static, Result<Item, RuntimeError>> {
    stream::once(async move {
        let bytes = serde_json::to_vec(&request)
            .map_err(|_| RuntimeError::Other("Cannot encode worker stream request".into()))?;
        if bytes.len() > http::MAX_CONTROL_BYTES {
            return Err(RuntimeError::Other("Worker stream request exceeds its bound".into()));
        }
        let response =
            http::send(&path, uid, Method::POST, PATH, http_client::full(bytes), "application/json", deadline).await?;
        if !response.status().is_success() {
            return Err(RuntimeError::Unavailable(format!("Worker stream refused HTTP {}", response.status())));
        }
        let body = response.into_body().into_data_stream().map_err(io::Error::other);
        let reader = FramedRead::new(StreamReader::new(body), LinesCodec::new_with_max_length(http::MAX_CONTROL_BYTES));
        Ok::<_, RuntimeError>(
            reader
                .map(|line| match line {
                    Ok(line) => serde_json::from_str::<Result<Item, RuntimeError>>(&line)
                        .map_err(|_| RuntimeError::Other("Invalid worker stream frame".into()))
                        .and_then(std::convert::identity),
                    Err(_) => {
                        Err(RuntimeError::Unavailable("Worker stream ended, is invalid or exceeds its bound".into()))
                    }
                })
                .boxed(),
        )
    })
    .try_flatten()
    .boxed()
}

pub fn logs(
    path: &Path,
    uid: u32,
    name: String,
    options: LogOptions,
    deadline: Instant,
) -> BoxStream<'static, Result<LogChunk, RuntimeError>> {
    receive(path.into(), uid, Request::Logs { name, options }, deadline)
        .map(|item| match item? {
            Item::Log(chunk) => Ok(chunk),
            Item::Event(_) => Err(RuntimeError::Other("Worker mixed observation streams".into())),
        })
        .boxed()
}

pub fn events(
    path: &Path,
    uid: u32,
    filters: Vec<String>,
    deadline: Instant,
) -> BoxStream<'static, Result<RuntimeEvent, RuntimeError>> {
    receive(path.into(), uid, Request::Events { filters }, deadline)
        .map(|item| match item? {
            Item::Event(event) => Ok(event),
            Item::Log(_) => Err(RuntimeError::Other("Worker mixed observation streams".into())),
        })
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::super::{authority::Owner, transport::Endpoint};
    use super::*;
    use crate::runtime::process::ProcessRuntime;
    use std::{
        fs,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
        time::Duration,
    };
    use tokio::time::timeout;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn real_quiet_stream_retires_on_owner_loss_with_its_lease_and_unchanged_errors() {
        let root = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
        let path = root.path().join("streams.sock");
        let uid = rustix::process::geteuid().as_raw();
        let endpoint = Endpoint::bind(&path, uid).await.unwrap();
        let (owner, lifetime) = tokio::net::UnixStream::pair().unwrap();
        let lock_path = root.path().join("compute.lock");
        let lease =
            fs::OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&lock_path).unwrap();
        rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
        let observer = fs::OpenOptions::new().read(true).write(true).open(&lock_path).unwrap();
        let (authority, retired) = Authority::watch(lifetime, uid, async move {
            drop(lease);
            Ok(())
        })
        .unwrap();
        let owner = Owner::ready(owner).await.unwrap();
        let runtime = Arc::new(ProcessRuntime::new(root.path(), Default::default()).unwrap());
        let cancel = CancellationToken::new();
        let server = tokio::spawn(http::serve(endpoint, app(runtime.clone(), authority), cancel.clone()));
        let name = uuid::Uuid::new_v4().to_string();
        let actual = runtime.logs(&name, LogOptions::default()).next().await.unwrap().unwrap_err();
        let transported = logs(&path, uid, name, LogOptions::default(), Instant::now() + Duration::from_secs(2))
            .next()
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(transported.to_string(), actual.to_string());
        let request = serde_json::to_vec(&Request::Events { filters: Vec::new() }).unwrap();
        let response = http::send(
            &path,
            uid,
            Method::POST,
            PATH,
            http_client::full(request),
            "application/json",
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            rustix::fs::flock(&observer, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap_err(),
            rustix::io::Errno::WOULDBLOCK
        );
        drop(owner);
        timeout(Duration::from_secs(2), retired).await.unwrap().unwrap().unwrap();
        rustix::fs::flock(&observer, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
        assert!(http_client::read_body(response, http::MAX_CONTROL_BYTES).await.unwrap().is_empty());
        cancel.cancel();
        server.await.unwrap().unwrap();
    }
}
