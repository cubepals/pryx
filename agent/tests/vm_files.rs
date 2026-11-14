// SPDX-License-Identifier: FSL-1.1-ALv2
//! File-only guest routes reuse real descriptor walks and token verification. These local
//! tests do not claim a guest boot; the VM workflow proves that once host integration is wired.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use pryx_agent::{
    auth::{Claims, Scope, SigningKey, TOKEN_LIFETIME_SECONDS},
    paths::DataDir,
    ports::Ports,
    process::Processes,
    server::{Agent, files_router, now},
    state,
};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn file_service_streams_data_without_executing_or_rewriting_workload_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let data = DataDir::open(directory.path()).unwrap();
    let ports = state::PortState { revision: 7, targets: vec![Some(3000)] };
    state::write(&data, state::PORTS_PATH, &ports, true).unwrap();
    let before = std::fs::read(directory.path().join(state::PORTS_PATH)).unwrap();
    let (key, _) = SigningKey::generate().unwrap();
    let token = key
        .sign(&Claims {
            sandbox: "one".into(),
            epoch: 1,
            issued_at: now(),
            expires_at: now() + TOKEN_LIFETIME_SECONDS,
            scope: Scope::Sandbox,
            max_ports: 0,
            max_exec_seconds: None,
            file: None,
        })
        .unwrap();
    let agent = Arc::new(Agent {
        vm_resources: false,
        file_tasks: Default::default(),
        shells: Default::default(),
        telemetry: Arc::default(),
        sandbox: "one".into(),
        epoch: 1,
        public_key: key.public_key(),
        key,
        logs: Arc::new(pryx_agent::logbook::Logbook::open(directory.path()).unwrap()),
        data,
        processes: Processes::new(directory.path().into(), "private".into()),
        ports: Ports::bind("one".into(), "invalid".into(), 0).await.unwrap(),
    });
    let app = files_router(agent.clone());
    let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(pryx_agent::contract::MAX_REQUEST_BYTES + 1).collect();
    let request = |method, path, body| {
        Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .body(body)
            .unwrap()
    };
    let response = app
        .clone()
        .oneshot(request("PUT", "/_pryx/files/content?path=roundtrip", Body::from(bytes.clone())))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response =
        app.clone().oneshot(request("GET", "/_pryx/files/content?path=roundtrip", Body::empty())).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.into_body().collect().await.unwrap().to_bytes(), bytes);
    for path in ["/_pryx/files/list?path=.", "/_pryx/files/stat?path=roundtrip", "/_pryx/health"] {
        assert_eq!(app.clone().oneshot(request("GET", path, Body::empty())).await.unwrap().status(), StatusCode::OK);
    }
    for path in ["/_pryx/processes", "/_pryx/shells", "/_pryx/kernels", "/_pryx/browser", "/_pryx/ports"] {
        assert_eq!(
            app.clone().oneshot(request("POST", path, Body::from("{}"))).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }
    let response =
        app.clone().oneshot(request("GET", "/_pryx/files/content?path=../outside", Body::empty())).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let unauthenticated = Request::builder().uri("/_pryx/files/stat?path=roundtrip").body(Body::empty()).unwrap();
    assert_eq!(app.oneshot(unauthenticated).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert!(agent.processes.list().is_empty());
    assert_eq!(std::fs::read(directory.path().join(state::PORTS_PATH)).unwrap(), before);
    assert!(!directory.path().join(state::ACTIVITY_PATH).exists());
}
