// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Ordered discovery of local Docker-compatible services. A socket's existence is not proof:
//! selection requires both version and info. A selected endpoint never changes during service.
use super::{ContainerRuntime, RuntimeError, RuntimeInfo, docker::DockerRuntime};
use crate::config::DockerConfig;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

pub fn candidates() -> Vec<(&'static str, PathBuf)> {
    let uid = rustix::process::geteuid().as_raw();
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")));
    ordered(std::env::var("DOCKER_HOST").ok().as_deref(), &directory)
}

fn ordered(host: Option<&str>, directory: &Path) -> Vec<(&'static str, PathBuf)> {
    let mut paths = Vec::new();
    if let Some(path) = host.and_then(|host| host.strip_prefix("unix://")) {
        paths.push(("engine-env", PathBuf::from(path)));
    }
    paths.extend([
        ("docker", DockerConfig::default().socket),
        ("docker-user", directory.join("docker.sock")),
        ("podman-user", directory.join("podman/podman.sock")),
        ("podman", PathBuf::from("/run/podman/podman.sock")),
    ]);
    paths
}

pub async fn inspect(path: &Path) -> Result<RuntimeInfo, RuntimeError> {
    tokio::time::timeout(PROBE_TIMEOUT, async { DockerRuntime::new(path)?.info().await })
        .await
        .map_err(|_| RuntimeError::Timeout("engine version/info exceeded two seconds".into()))?
}

pub async fn socket(config: &DockerConfig) -> Result<PathBuf, RuntimeError> {
    if !config.discover {
        return Ok(config.socket.clone());
    }
    if std::env::var("DOCKER_HOST")
        .is_ok_and(|host| host.strip_prefix("unix://").is_none_or(|path| !Path::new(path).is_absolute()))
    {
        return Err(RuntimeError::Other(
            "DOCKER_HOST must name a local unix:// socket; remote engines cannot bind this host's data".into(),
        ));
    }
    // Explicit container mode may start while its engine is down. Auto separately probes it.
    Ok(first(candidates().into_iter().map(|(_, path)| path)).await.unwrap_or_else(|| config.socket.clone()))
}

async fn first(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    for path in paths {
        if inspect(&path).await.is_ok() {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn environment_precedes_system_then_user_engines() {
        let directory = Path::new("/run/user/1234");
        let paths = ordered(Some("unix:///run/user/1234/docker.sock"), directory);
        assert_eq!(
            paths.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            ["engine-env", "docker", "docker-user", "podman-user", "podman"]
        );
        assert_eq!(paths[0].1, directory.join("docker.sock"));
        let paths = ordered(None, directory);
        assert_eq!(
            paths.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            ["docker", "docker-user", "podman-user", "podman"]
        );
    }
    #[tokio::test]
    async fn an_explicit_endpoint_never_falls_back() {
        let config =
            DockerConfig { socket: "/missing/owned-engine.sock".into(), discover: false, ..Default::default() };
        assert_eq!(socket(&config).await.unwrap(), config.socket);
        assert!(inspect(&config.socket).await.is_err());
    }
    #[tokio::test]
    async fn selection_needs_version_and_info_and_pins_the_first_complete_answer() {
        use axum::{
            Json, Router,
            http::{StatusCode, Uri},
            routing::get,
        };
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let directory = tempfile::tempdir().unwrap();
        let mut servers = Vec::new();
        let mut paths = Vec::new();
        let calls = Arc::new(AtomicUsize::new(0));
        for (index, answers_info) in [false, true, true].into_iter().enumerate() {
            let path = directory.path().join(index.to_string());
            let listener = tokio::net::UnixListener::bind(&path).unwrap();
            paths.push(path);
            let calls = calls.clone();
            let app = Router::new().fallback(get(move |uri: Uri| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    if uri.path().ends_with("/version") {
                        Ok(Json(
                            serde_json::to_value(bollard::models::SystemVersion {
                                api_version: Some(bollard::API_DEFAULT_VERSION.to_string()),
                                ..Default::default()
                            })
                            .unwrap(),
                        ))
                    } else if answers_info && uri.path().ends_with("/info") {
                        Ok(Json(serde_json::to_value(bollard::models::SystemInfo::default()).unwrap()))
                    } else {
                        Err(StatusCode::SERVICE_UNAVAILABLE)
                    }
                }
            }));
            servers.push(tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }));
        }
        let chosen = first(paths.clone()).await.unwrap();
        assert_eq!(chosen, paths[1], "an existing socket and successful version alone are insufficient");
        assert_eq!(calls.load(Ordering::SeqCst), 6, "do not ask the third service after selecting an engine");
        for server in servers {
            server.abort();
        }
    }
}
