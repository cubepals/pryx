// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private worker control commands reuse the runtime owner's actual types and errors.
//! Dispatch owns an admitted operation across HTTP caller loss. Monitor executables, IDs
//! and bundle pins remain installed host policy; this does not start an installed service.
use super::http::{CONTROL_PATH, MAX_CONTROL_BYTES};
use super::{authority::Authority, transport::Peer};
use crate::runtime::{ContainerRuntime, ContainerSpec, RuntimeError, vm::instance::VmRuntime};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{ConnectInfo, State},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Info,
    Limits,
    EnsureNetwork { name: String, labels: BTreeMap<String, String> },
    EnsureInternalNetwork { name: String, labels: BTreeMap<String, String> },
    NetworkIsolated { name: String },
    EnsureImage { image: String, timeout: Duration },
    Create { spec: Box<ContainerSpec> },
    Start { name: String },
    Stop { name: String, signal: String, grace: Duration },
    Kill { name: String },
    Pause { name: String },
    Unpause { name: String },
    DisableRestart { name: String },
    Remove { name: String },
    Inspect { name: String },
    List { filters: Vec<String> },
    Exec { name: String, command: Vec<String>, timeout: Duration, max_output: usize },
    Stats { name: String },
}

fn encode(value: impl Serialize) -> Result<Value, RuntimeError> {
    serde_json::to_value(value).map_err(|_| RuntimeError::Other("Cannot encode private worker result".into()))
}

async fn execute(runtime: &dyn ContainerRuntime, command: Command) -> Result<Value, RuntimeError> {
    match command {
        Command::Info => encode(runtime.info().await?),
        Command::Limits => encode(runtime.resource_limits().await?),
        Command::EnsureNetwork { name, labels } => encode(runtime.ensure_network(&name, &labels).await?),
        Command::EnsureInternalNetwork { name, labels } => {
            encode(runtime.ensure_internal_network(&name, &labels).await?)
        }
        Command::NetworkIsolated { name } => encode(runtime.network_isolated(&name).await?),
        Command::EnsureImage { image, timeout } => encode(runtime.ensure_image(&image, timeout).await?),
        Command::Create { spec } => encode(runtime.create(&spec).await?),
        Command::Start { name } => encode(runtime.start(&name).await?),
        Command::Stop { name, signal, grace } => encode(runtime.stop(&name, &signal, grace).await?),
        Command::Kill { name } => encode(runtime.kill(&name).await?),
        Command::Pause { name } => encode(runtime.pause(&name).await?),
        Command::Unpause { name } => encode(runtime.unpause(&name).await?),
        Command::DisableRestart { name } => encode(runtime.disable_restart(&name).await?),
        Command::Remove { name } => encode(runtime.remove(&name).await?),
        Command::Inspect { name } => encode(runtime.inspect(&name).await?),
        Command::List { filters } => encode(runtime.list(&filters).await?),
        Command::Exec { name, command, timeout, max_output } => {
            encode(runtime.exec(&name, &command, timeout, max_output).await?)
        }
        Command::Stats { name } => encode(runtime.stats(&name).await?),
    }
}

#[derive(Clone)]
struct Dispatch {
    runtime: Arc<dyn ContainerRuntime>,
    authority: Authority,
}

async fn control(
    State(dispatch): State<Dispatch>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    bytes: Bytes,
) -> Json<Result<Value, RuntimeError>> {
    let admitted = match dispatch.authority.admit_peer(&peer).await {
        Ok(admitted) => admitted,
        Err(_) => {
            return Json(Err(RuntimeError::Unavailable(
                "Worker frontend authority is absent, different or retiring".into(),
            )));
        }
    };
    let command = match serde_json::from_slice::<Command>(&bytes) {
        Ok(command) => command,
        Err(_) => return Json(Err(RuntimeError::Other("Invalid private worker command".into()))),
    };
    // A dropped connection cannot cancel an admitted non-idempotent operation.
    Json(
        tokio::spawn(async move {
            let _admitted = admitted;
            execute(dispatch.runtime.as_ref(), command).await
        })
        .await
        .unwrap_or_else(|_| Err(RuntimeError::Unavailable("Private worker operation ended without an answer".into()))),
    )
}

fn app(runtime: Arc<dyn ContainerRuntime>, authority: Authority) -> Router {
    Router::new()
        .route(CONTROL_PATH, post(control))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_CONTROL_BYTES))
        .with_state(Dispatch { runtime, authority })
}

pub fn router(runtime: Arc<VmRuntime>, authority: Authority) -> Router {
    app(runtime, authority)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{ContainerInfo, process::ProcessRuntime};
    use std::{fs, os::unix::fs::PermissionsExt};
    use tokio::time::Instant;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn actual_runtime_answers_and_errors_keep_their_owner_types_over_http() {
        let root = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
        let socket = root.path().join("worker.sock");
        let owner = rustix::process::geteuid().as_raw();
        let endpoint = super::super::transport::Endpoint::bind(&socket, owner).await.unwrap();
        // Exercise the transport with the actual rootless backend; installed workers accept
        // only VmRuntime, whose actual KVM acceptance runs in the existing optional job.
        let runtime = Arc::new(ProcessRuntime::new(root.path(), Default::default()).unwrap());
        let (lifetime, accepted) = tokio::net::UnixStream::pair().unwrap();
        let (authority, retired) = Authority::watch(accepted, owner, async { Ok(()) }).unwrap();
        let lifetime = super::super::authority::Owner::ready(lifetime).await.unwrap();
        let cancel = CancellationToken::new();
        let server =
            tokio::spawn(super::super::http::serve(endpoint, app(runtime.clone(), authority.clone()), cancel.clone()));
        let name = uuid::Uuid::new_v4().to_string();
        let answer: Result<Option<ContainerInfo>, RuntimeError> = super::super::http::control(
            &socket,
            owner,
            &Command::Inspect { name: name.clone() },
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert!(answer.unwrap().is_none());
        let actual = runtime.ensure_image(&name, Duration::from_secs(1)).await.unwrap_err();
        let answer: Result<bool, RuntimeError> = super::super::http::control(
            &socket,
            owner,
            &Command::EnsureImage { image: name, timeout: Duration::from_secs(1) },
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap();
        let received = answer.unwrap_err();
        assert!(matches!(received, RuntimeError::Other(_)));
        assert_eq!(received.to_string(), actual.to_string());
        let command = Command::Inspect { name: uuid::Uuid::new_v4().to_string() };
        let foreign = tokio::process::Command::new("curl")
            .args(["--silent", "--show-error", "--noproxy", "*", "--unix-socket"])
            .arg(&socket)
            .args(["--header", "Content-Type: application/json", "--data-binary"])
            .arg(serde_json::to_string(&command).unwrap())
            .arg(format!("http://localhost{CONTROL_PATH}"))
            .output()
            .await
            .unwrap();
        assert!(foreign.status.success());
        let answer: Result<Option<ContainerInfo>, RuntimeError> = serde_json::from_slice(&foreign.stdout).unwrap();
        assert!(
            matches!(answer, Err(RuntimeError::Unavailable(_))),
            "Another actual HTTP client PID borrowed frontend authority"
        );
        drop(lifetime);
        authority.cancellation().cancelled().await;
        let answer: Result<Option<ContainerInfo>, RuntimeError> =
            super::super::http::control(&socket, owner, &command, Instant::now() + Duration::from_secs(2))
                .await
                .unwrap();
        assert!(matches!(answer, Err(RuntimeError::Unavailable(_))));
        retired.await.unwrap().unwrap();
        cancel.cancel();
        server.await.unwrap().unwrap();
    }
}
