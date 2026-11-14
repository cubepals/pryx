// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! A compatible HTTP peer verifies refusal before mutations. This is transport/policy proof;
//! actual Podman confinement and lifecycle acceptance still require that engine in CI.
use super::*;
use axum::{Json, Router, extract::Request, http::StatusCode, routing::any};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn podman_restart_fencing_requires_observed_no_restart_without_unsupported_updates() {
    use bollard::models::{ContainerInspectResponse, HostConfig};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("restart.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let updates = Arc::new(AtomicUsize::new(0));
    let policy = Arc::new(std::sync::Mutex::new(Some(RestartPolicyNameEnum::NO)));
    let observed = policy.clone();
    let calls = updates.clone();
    let app = Router::new().fallback(any(move |request: Request| {
        let name = *observed.lock().unwrap();
        let calls = calls.clone();
        async move {
            if request.method() != axum::http::Method::GET {
                calls.fetch_add(1, Ordering::SeqCst);
                return (StatusCode::NOT_IMPLEMENTED, Json(serde_json::json!({})));
            }
            let inspect = ContainerInspectResponse {
                host_config: Some(HostConfig {
                    restart_policy: name
                        .map(|name| DockerRestartPolicy { name: Some(name), maximum_retry_count: None }),
                    ..Default::default()
                }),
                ..Default::default()
            };
            (StatusCode::OK, Json(serde_json::to_value(inspect).unwrap()))
        }
    }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let runtime = DockerRuntime::new(&path).unwrap();
    *runtime.info.lock().unwrap() = Some(RuntimeInfo { podman: true, ..Default::default() });
    runtime.disable_restart("owned").await.unwrap();
    *policy.lock().unwrap() = Some(RestartPolicyNameEnum::EMPTY);
    runtime.disable_restart("owned").await.unwrap();
    for changed in [Some(RestartPolicyNameEnum::ALWAYS), None] {
        *policy.lock().unwrap() = changed;
        let error = runtime.disable_restart("owned").await.unwrap_err();
        assert!(error.to_string().contains("restart-policy"), "{error:?}");
    }
    assert_eq!(updates.load(Ordering::SeqCst), 0, "no unsupported update or ignored policy change");
    runtime.ensure_internal_network("unused-placeholder", &BTreeMap::new()).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn an_unsupported_bridge_policy_is_refused_before_any_compute_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("engine.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let mutations = Arc::new(AtomicUsize::new(0));
    let seen = mutations.clone();
    let app = Router::new().fallback(any(move |request: Request| {
        let seen = seen.clone();
        async move {
            if request.method() != axum::http::Method::GET {
                seen.fetch_add(1, Ordering::SeqCst);
                return (StatusCode::NOT_IMPLEMENTED, Json(serde_json::json!({})));
            }
            let value = if request.uri().path().ends_with("/version") {
                serde_json::to_value(bollard::models::SystemVersion {
                    api_version: Some(bollard::API_DEFAULT_VERSION.to_string()),
                    components: Some(vec![bollard::models::SystemVersionComponents {
                        name: PODMAN_ENGINE_COMPONENT.into(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                })
                .unwrap()
            } else {
                serde_json::to_value(bollard::models::SystemInfo {
                    memory_limit: Some(true),
                    cpu_cfs_quota: Some(true),
                    cpu_shares: Some(true),
                    pids_limit: Some(true),
                    cgroup_driver: Some(bollard::models::SystemInfoCgroupDriverEnum::SYSTEMD),
                    ..Default::default()
                })
                .unwrap()
            };
            (StatusCode::OK, Json(value))
        }
    }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let runtime = DockerRuntime::new(&path).unwrap();
    assert_eq!(runtime.resource_limits().await.unwrap_err().to_string(), PODMAN_POLICY_PROBLEM);
    let error = runtime.create(&super::tests::spec()).await.unwrap_err();
    assert_eq!(error.to_string(), PODMAN_POLICY_PROBLEM);
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn losing_cpu_or_pids_enforcement_refuses_before_native_or_compatible_create() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("controllers.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let mutations = Arc::new(AtomicUsize::new(0));
    let seen = mutations.clone();
    let app = Router::new().fallback(any(move |request: Request| {
        let seen = seen.clone();
        async move {
            if request.method() != axum::http::Method::GET {
                seen.fetch_add(1, Ordering::SeqCst);
                return (StatusCode::NOT_IMPLEMENTED, Json(serde_json::json!({})));
            }
            let value = if request.uri().path().ends_with("/version") {
                serde_json::to_value(bollard::models::SystemVersion {
                    api_version: Some(bollard::API_DEFAULT_VERSION.to_string()),
                    ..Default::default()
                })
                .unwrap()
            } else {
                serde_json::to_value(bollard::models::SystemInfo {
                    memory_limit: Some(true),
                    cpu_cfs_quota: Some(false),
                    cpu_shares: Some(false),
                    pids_limit: Some(false),
                    cgroup_driver: Some(bollard::models::SystemInfoCgroupDriverEnum::SYSTEMD),
                    ..Default::default()
                })
                .unwrap()
            };
            (StatusCode::OK, Json(value))
        }
    }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let runtime = DockerRuntime::new(&path).unwrap();
    let mut spec = super::tests::spec();
    spec.require_cpu_limit = true;
    assert!(
        runtime.create(&spec).await.unwrap_err().to_string().contains(&crate::protocol::Guarantee::CpuLimit.feature())
    );
    spec.require_cpu_limit = false;
    spec.require_pids_limit = true;
    assert!(runtime.create(&spec).await.unwrap_err().to_string().contains("pids-limit"));
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn native_podman_metadata_uses_the_selected_socket_and_native_route() {
    use axum::routing::get;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("podman.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let versions: serde_json::Value =
        serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/.github/engine-versions.json")))
            .unwrap();
    let native_version = versions["podman"]["version"].as_str().unwrap().split('+').next().unwrap().to_owned();
    let endpoint = format!("/v{native_version}/libpod/info");
    let native = serde_json::json!({"host": {"networkBackend": "netavark", "cgroupManager": "systemd",
        "cgroupControllers": [], "security": {"rootless": true}}});
    let native: podman_api_stubs::models::Info = serde_json::from_value(native).unwrap();
    let native = Arc::new(std::sync::Mutex::new(native));
    let returned = native.clone();
    let server_version = Arc::new(std::sync::Mutex::new(native_version.clone()));
    let changing = server_version.clone();
    let paths = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = paths.clone();
    let app = Router::new()
        .route(
            &endpoint,
            get(move || {
                let native = returned.lock().unwrap().clone();
                async move { Json(native) }
            }),
        )
        .fallback(any(move |request: Request| {
            let version = native_version.clone();
            let server_version = changing.lock().unwrap().clone();
            seen.lock().unwrap().push(request.uri().path().to_owned());
            async move {
                let value = if request.uri().path().ends_with("/version") {
                    serde_json::to_value(bollard::models::SystemVersion {
                        version: Some(version.clone()),
                        api_version: Some(bollard::API_DEFAULT_VERSION.to_string()),
                        components: Some(vec![bollard::models::SystemVersionComponents {
                            name: PODMAN_ENGINE_COMPONENT.into(),
                            version,
                            ..Default::default()
                        }]),
                        ..Default::default()
                    })
                    .unwrap()
                } else {
                    serde_json::to_value(bollard::models::SystemInfo {
                        server_version: Some(server_version),
                        security_options: Some(vec![ROOTLESS_SECURITY_OPTION.into()]),
                        memory_limit: Some(true),
                        cpu_cfs_quota: Some(true),
                        cpu_shares: Some(true),
                        pids_limit: Some(true),
                        cgroup_driver: Some(bollard::models::SystemInfoCgroupDriverEnum::SYSTEMD),
                        ..Default::default()
                    })
                    .unwrap()
                };
                Json(value)
            }
        }));
    // Actual Podman 4.9.3 metadata reads in CI job 130448281871 take about 560 ms each.
    // The two-second discovery deadline must include every required read, without serial delays.
    let app = app.layer(axum::middleware::from_fn(|request, next: axum::middleware::Next| async move {
        tokio::time::sleep(Duration::from_millis(560)).await;
        next.run(request).await
    }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let runtime = DockerRuntime::new(&path).unwrap();
    let info = tokio::time::timeout(super::super::ENGINE_PROBE_TIMEOUT, runtime.info()).await.unwrap().unwrap();
    assert!(info.container_policy_problem.is_none(), "native route was not reached: {:?}", paths.lock().unwrap());
    assert!(info.podman && info.rootless);
    assert!(!info.memory_limit && !info.cpu_limit && !info.pids_limit);
    verify_metadata_refresh(&runtime, &paths, &native, &server_version).await;
    server.abort();
}

async fn verify_metadata_refresh(
    runtime: &DockerRuntime,
    paths: &Arc<std::sync::Mutex<Vec<String>>>,
    native: &Arc<std::sync::Mutex<podman_api_stubs::models::Info>>,
    server_version: &Arc<std::sync::Mutex<String>>,
) {
    let versions = || paths.lock().unwrap().iter().filter(|path| path.ends_with("/version")).count();
    assert_eq!(versions(), 2);
    native.lock().unwrap().host.as_mut().unwrap().cgroup_controllers = Some(vec!["memory".into()]);
    let info = tokio::time::timeout(super::super::ENGINE_PROBE_TIMEOUT, runtime.info()).await.unwrap().unwrap();
    assert!(info.memory_limit, "controllers remain live observations");
    assert_eq!(versions(), 2, "stable Podman version reads must not repeat during health/probe contention");
    let original = server_version.lock().unwrap().clone();
    *server_version.lock().unwrap() = format!("{original}-changed");
    assert!(matches!(runtime.info().await, Err(RuntimeError::Unavailable(_))));
    *server_version.lock().unwrap() = original;
    assert!(runtime.info().await.unwrap().memory_limit);
    assert_eq!(versions(), 4, "a changed engine must negotiate and revalidate metadata again");
}
