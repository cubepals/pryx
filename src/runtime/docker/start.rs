// SPDX-License-Identifier: FSL-1.1-ALv2
//! One engine start mutation under exec-generation ownership. Configured Docker backends
//! bind the native systemd invocation; direct callers and Podman retain their observation path.
use super::*;

pub(super) async fn start(runtime: &DockerRuntime, name: &str) -> Result<(), RuntimeError> {
    let id = quick("inspect start identity", runtime.docker()?.inspect_container(name, None))
        .await?
        .id
        .ok_or_else(|| RuntimeError::Other("start target has no immutable identity".into()))?;
    let generation = runtime.starts.lock(&id).await;
    let name = id.as_str();
    if runtime.oci_runtime.is_some() || runtime.journal_context().is_some() {
        journal_start(runtime, name).await?;
    } else if let Err(error) = quick("start", runtime.docker()?.start_container(name, None)).await {
        // A lost response does not undo Docker's mutation. Confirm its current state once;
        // never issue a second start, and preserve the timeout unless it is already running.
        if !(matches!(error, RuntimeError::Timeout(_)) && running(runtime, name).await) {
            return Err(error);
        }
        tracing::warn!("confirmed running after Docker start timeout without repeating start");
    }
    // Confirmation above is read-only while mutation authority is held. Relay adoption can
    // retire an orphan from this start, so it acquires that authority separately.
    drop(generation);
    runtime.inspect(name).await?;
    Ok(())
}

/// A read-only look at whether this backend's container is running: no relay is touched.
async fn running(runtime: &DockerRuntime, name: &str) -> bool {
    let Ok(docker) = runtime.docker() else { return false };
    matches!(
        quick("confirm start", docker.inspect_container(name, None)).await,
        Ok(info) if gvisor::owns_container(runtime.oci_runtime, &info)
            && info.state.as_ref().and_then(|state| state.running) == Some(true)
    )
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
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

    #[tokio::test]
    async fn idempotent_start_can_retire_an_abandoned_ingress_owner() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let kills = Arc::new(AtomicUsize::new(0));
        let starts = Arc::new(AtomicUsize::new(0));
        let probes = Arc::new(AtomicUsize::new(0));
        let observed = (kills.clone(), starts.clone(), probes.clone());
        let timestamp = OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap();
        let bindings = serde_json::to_string(&vec![crate::runtime::PortBinding {
            container_port: 8080,
            host_port: 0,
            host_ips: vec![std::net::Ipv4Addr::LOCALHOST.into()],
            protocol: crate::protocol::Proto::Tcp,
        }])
        .unwrap();
        let record = crate::labels::private_record(
            "alpine:3.22",
            &serde_json::from_str::<Vec<crate::runtime::PortBinding>>(&bindings).unwrap(),
        );
        let app = Router::new().fallback(any(move |request: Request| {
            let (kills, starts, probes) = observed.clone();
            let timestamp = timestamp.clone();
            let bindings = bindings.clone();
            let record = record.clone();
            async move {
                let route = request.uri().path();
                if route.ends_with("/start") {
                    starts.fetch_add(1, Ordering::SeqCst);
                    // A real Engine returns this for an already-running immutable ID.
                    return (StatusCode::NOT_MODIFIED, Json(serde_json::json!({})));
                }
                if route.ends_with("/kill") {
                    kills.fetch_add(1, Ordering::SeqCst);
                    return (StatusCode::NO_CONTENT, Json(serde_json::json!({})));
                }
                if route.contains("/exec/") {
                    if probes.fetch_add(1, Ordering::SeqCst) > 0 {
                        // Losing the read-only completion observation cannot establish EOF.
                        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"message": "engine unavailable"})));
                    }
                    return (StatusCode::OK, Json(serde_json::json!({"ContainerID": "owned", "Running": true,
                        "ProcessConfig": {"entrypoint": pryx_agent::contract::BINARY_PATH,
                            "arguments": [pryx_agent::exec_ingress::MODE, "tcp", "8080"]}})));
                }
                let running = kills.load(Ordering::SeqCst) == 0;
                (StatusCode::OK, Json(serde_json::json!({"Id": "owned", "Name": "/owned",
                    "State": {"Running": running, "Status": if running {"running"} else {"exited"}, "StartedAt": timestamp},
                    "Config": {"Labels": {crate::labels::LABEL_INTERNAL_BINDINGS: bindings, crate::labels::LABEL_RECORD: record}}, "ExecIDs": ["abandoned"]})))
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let runtime = DockerRuntime::new(&path).unwrap();
        *runtime.info.lock().unwrap() = Some(RuntimeInfo { rootless: true, ..Default::default() });
        let error = tokio::time::timeout(Duration::from_secs(2), runtime.start("owned"))
            .await
            .expect("adoption must not wait on its own start lock")
            .unwrap_err();
        assert!(error.to_string().contains("old container start was retired"), "{error}");
        assert_eq!(starts.load(Ordering::SeqCst), 1, "never repeat a start mutation");
        assert_eq!(kills.load(Ordering::SeqCst), 1, "retire the abandoned generation exactly once");
        server.abort();
    }
}
