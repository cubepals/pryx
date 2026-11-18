// SPDX-License-Identifier: FSL-1.1-ALv2
//! One engine start mutation under exec-generation ownership. Configured Docker backends
//! bind the native systemd invocation; direct callers and Podman retain their observation path.
use super::*;

pub(super) async fn start(runtime: &DockerRuntime, name: &str) -> Result<(), RuntimeError> {
    let id = quick("inspect start identity", runtime.docker()?.inspect_container(name, None))
        .await?
        .id
        .ok_or_else(|| RuntimeError::Other("start target has no immutable identity".into()))?;
    let _generation = runtime.starts.lock(&id).await;
    let name = id.as_str();
    if runtime.oci_runtime.is_some() || runtime.journal_context().is_some() {
        return journal_start(runtime, name).await;
    }
    if let Err(error) = quick("start", runtime.docker()?.start_container(name, None)).await {
        if matches!(error, RuntimeError::Timeout(_))
            && matches!(runtime.inspect(name).await, Ok(Some(info)) if info.status == ContainerStatus::Running)
        {
            tracing::warn!("confirmed running after Docker start timeout without repeating start");
            return Ok(());
        }
        return Err(error);
    }
    runtime.inspect(name).await?;
    Ok(())
}

async fn journal_start(runtime: &DockerRuntime, name: &str) -> Result<(), RuntimeError> {
    let before = quick("inspect Docker start owner", runtime.docker()?.inspect_container(name, None)).await?;
    if !gvisor::owns_container(runtime.oci_runtime, &before) {
        return Err(RuntimeError::NotFound(name.into()));
    }
    let before = info_from(before);
    let mut capture = None;
    if let Some(context) = runtime.journal_context()
        && context.can_capture()?
    {
        match context.begin(&before).await {
            Ok(held) => capture = Some(held),
            Err(error) => return Err(error),
        }
    }
    // Names can be replaced between inspection and mutation. Start only the immutable ID.
    let result = quick("start", runtime.docker()?.start_container(&before.id, None)).await;
    let actual = quick("confirm Docker start owner", runtime.docker()?.inspect_container(&before.id, None)).await?;
    let actual = info_from(actual);
    if actual.id != before.id {
        return Err(oom_journal::failure("Docker start owner changed"));
    }
    if let (Some(context), Some(held)) = (runtime.journal_context(), &mut capture)
        && context.finish(runtime, held, &before, &actual, &result).await.is_err()
    {
        tracing::debug!("native journal scope invocation is unknown");
    }
    drop(capture);
    match result {
        Ok(()) => (),
        Err(RuntimeError::Timeout(_)) if actual.status == ContainerStatus::Running => {
            tracing::warn!("confirmed running after Docker start timeout without repeating start");
        }
        Err(error) => return Err(error),
    }
    runtime.inspect(&before.id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        extract::Request,
        http::{Method, StatusCode},
        routing::any,
    };
    use serde_json::json;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn both_start_paths_wait_for_the_immutable_exec_generation() {
        const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        for gvisor in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let socket = directory.path().join("engine.sock");
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            let seen = Arc::new(tokio::sync::Notify::new());
            let mutations = Arc::new(AtomicUsize::new(0));
            let (lookups, starts) = (seen.clone(), mutations.clone());
            let app = Router::new().fallback(any(move |request: Request| {
                let (lookups, starts) = (lookups.clone(), starts.clone());
                async move {
                    let path = request.uri().path();
                    if request.method() == Method::POST {
                        assert!(path.ends_with(&format!("/containers/{ID}/start")), "only the gated ID mutates");
                        starts.fetch_add(1, Ordering::SeqCst);
                        return (StatusCode::NO_CONTENT, String::new());
                    }
                    assert_eq!(request.method(), Method::GET);
                    assert!(
                        path.ends_with("/containers/owned/json") || path.ends_with(&format!("/containers/{ID}/json"))
                    );
                    if path.ends_with("/containers/owned/json") {
                        lookups.notify_one();
                    }
                    let running = starts.load(Ordering::SeqCst) != 0;
                    let runtime = if gvisor { gvisor::GVISOR_RUNTIME } else { gvisor::RUNC };
                    (
                        StatusCode::OK,
                        json!({
                            "Id": ID, "Name": "/owned", "Config": {"Labels": {}},
                            "HostConfig": {"Runtime": runtime},
                            "State": {"Status": if running {"running"} else {"created"}, "Running": running,
                                "StartedAt": "2026-11-18T00:00:00Z", "OOMKilled": false}
                        })
                        .to_string(),
                    )
                }
            }));
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let runtime = DockerRuntime::new(&socket).unwrap();
            let runtime = Arc::new(if gvisor { runtime.gvisor() } else { runtime });
            let generation = runtime.starts.lock(ID).await;
            let starting = runtime.clone();
            let mut started = tokio::spawn(async move { starting.start("owned").await });
            tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, seen.notified()).await.unwrap();
            // The owned local transport is live, but a held generation must keep the
            // actual start pending rather than merely serialize another lock helper.
            assert!(tokio::time::timeout(Duration::from_millis(200), &mut started).await.is_err());
            assert_eq!(mutations.load(Ordering::SeqCst), 0);
            drop(generation);
            tokio::time::timeout(crate::runtime::ENGINE_PROBE_TIMEOUT, started).await.unwrap().unwrap().unwrap();
            assert_eq!(mutations.load(Ordering::SeqCst), 1);
            server.abort();
        }
    }
}
