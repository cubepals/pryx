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
