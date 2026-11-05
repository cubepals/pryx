// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Agent contracts against real local processes and descriptors. No container engine, control
//! process or database is involved; those boundaries are exercised by the CI end-to-end suites.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pryx_agent::auth::{Claims, Scope, SigningKey, TOKEN_LIFETIME_SECONDS};
use pryx_agent::paths::DataDir;
use pryx_agent::ports::Ports;
use pryx_agent::process::{Command, Event, Processes, Start};
use pryx_agent::server::{Agent, now, router};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn fixture() -> (tempfile::TempDir, Arc<Agent>, String) {
    let dir = tempfile::tempdir().unwrap();
    let (key, _) = SigningKey::generate().unwrap();
    let token = key
        .sign(&Claims {
            sandbox: "one".into(),
            epoch: 1,
            issued_at: now(),
            expires_at: now() + TOKEN_LIFETIME_SECONDS,
            scope: Scope::Sandbox,
            max_ports: 3,
            max_exec_seconds: Some(10),
            file: None,
        })
        .unwrap();
    let agent = Arc::new(Agent {
        logs: Arc::new(pryx_agent::logbook::Logbook::open(dir.path()).unwrap()),
        telemetry: Arc::default(),
        sandbox: "one".into(),
        epoch: 1,
        public_key: key.public_key(),
        key,
        data: DataDir::open(dir.path()).unwrap(),
        processes: Processes::new(dir.path().into(), "PRYX_AGENT_SECRET_fixture".into()),
        ports: Ports::bind("one".into(), "example.test".into(), 0).await.unwrap(),
    });
    (dir, agent, token)
}

#[tokio::test]
async fn file_urls_are_bound_to_one_path_method_and_short_expiry() {
    let (dir, agent, token) = fixture().await;
    std::fs::write(dir.path().join("message"), b"hello").unwrap();
    let (status, grant) =
        call(&agent, &token, "POST", "/_pryx/files/token", json!({"path":"message","method":"GET"})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(grant["expiresAt"].as_i64().unwrap() <= now() + pryx_agent::auth::FILE_TOKEN_LIFETIME_SECONDS);
    let url = grant["url"].as_str().unwrap();
    let response =
        router(agent.clone()).oneshot(Request::builder().uri(url).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.into_body().collect().await.unwrap().to_bytes(), "hello");
    for (method, url) in [("PUT", url.to_owned()), ("GET", url.replace("path=message", "path=other"))] {
        let response = router(agent.clone())
            .oneshot(Request::builder().method(method).uri(url).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}

async fn call(agent: &Arc<Agent>, token: &str, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = router(agent.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("idempotency-key", "request-one")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn request(command: &str) -> Start {
    Start {
        command: Command::Shell(command.into()),
        env: Default::default(),
        cwd: None,
        stdin: false,
        pty: false,
        cols: None,
        rows: None,
        timeout: None,
    }
}

#[tokio::test]
async fn authentication_precedes_process_creation_and_keys_bind_requests() {
    let (_dir, agent, token) = fixture().await;
    let (status, _) = call(&agent, "bad", "POST", "/_pryx/processes", json!({"command":"true"})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(agent.processes.list().is_empty());
    let (status, made) = call(&agent, &token, "POST", "/_pryx/processes", json!({"command":"true"})).await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, again) = call(&agent, &token, "POST", "/_pryx/processes", json!({"command":"true"})).await;
    assert_eq!(made["id"], again["id"]);
    let (status, _) = call(&agent, &token, "POST", "/_pryx/processes", json!({"command":"echo different"})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(&agent, &token, "GET", "/_pryx/processes", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let id = made["id"].as_str().unwrap();
    let (_, result) = call(&agent, &token, "GET", &format!("/_pryx/processes/{id}/wait"), Value::Null).await;
    assert_eq!(result["code"], 0);
}

#[tokio::test]
async fn output_is_live_and_processes_outlive_their_first_observer() {
    let (_dir, agent, _) = fixture().await;
    let mut command = request("printf first; read gate; printf last");
    command.stdin = true;
    let process = agent.processes.start(command, "live", Some(5)).unwrap();
    let (replay, mut events) = process.subscribe().await;
    let first = if let Some(first) = replay.first() {
        first.clone()
    } else {
        tokio::time::timeout(Duration::from_secs(2), events.recv()).await.unwrap().unwrap()
    };
    assert!(matches!(first, Event::Output { .. }));
    assert!(process.view().running);
    drop(events);
    process.write(b"continue\n").await.unwrap();
    assert_eq!(process.wait().await.code, Some(0));
    let (replay, _) = process.subscribe().await;
    assert!(replay.iter().any(|e| matches!(e, Event::Exit { .. })));
    assert!(process.view().result.is_some());
}

#[tokio::test]
async fn stdin_environment_cwd_and_deadlines_use_real_processes() {
    let (dir, agent, _) = fixture().await;
    let mut input = request("read value; printf '%s:%s:%s' \"$value\" \"$CI\" \"$PWD\"");
    input.stdin = true;
    input.env.insert("CI".into(), "yes".into());
    let process = agent.processes.start(input, "stdin", Some(2)).unwrap();
    process.write(b"hello\n").await.unwrap();
    process.close_input().await;
    assert_eq!(process.wait().await.code, Some(0));
    let (events, _) = process.subscribe().await;
    use base64::{Engine, engine::general_purpose::STANDARD};
    let bytes = events
        .iter()
        .filter_map(|e| match e {
            Event::Output { bytes, .. } => Some(STANDARD.decode(bytes).unwrap()),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(String::from_utf8(bytes).unwrap(), format!("hello:yes:{}", dir.path().display()));
    let mut closed = request("exec 1>&- 2>&-; sleep 30");
    closed.timeout = Some(1);
    let process = agent.processes.start(closed, "deadline", Some(2)).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), process.wait()).await.unwrap();
    assert!(result.timed_out && result.killed);
    assert_eq!(result.code, Some(128 + libc::SIGKILL));
    assert!(agent.processes.start(request("true"), "policy", Some(0)).is_err());
}

#[tokio::test]
async fn streamed_files_have_no_sdk_chunk_limit_and_cannot_follow_links() {
    let (dir, agent, token) = fixture().await;
    let bytes = vec![0x42; 3 * 1024 * 1024];
    let response = router(agent.clone())
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/_pryx/files/content?path=src/data.bin")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(bytes.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = router(agent.clone())
        .oneshot(
            Request::builder()
                .uri("/_pryx/files/content?path=src/data.bin")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.into_body().collect().await.unwrap().to_bytes(), bytes);
    std::os::unix::fs::symlink("/etc", dir.path().join("outside")).unwrap();
    let (status, _) = call(&agent, &token, "GET", "/_pryx/files/content?path=outside/passwd", Value::Null).await;
    assert_ne!(status, StatusCode::OK);
    let (_, listed) = call(&agent, &token, "GET", "/_pryx/files/list?path=src", Value::Null).await;
    assert_eq!(listed["files"][0]["size"], bytes.len());
    let (status, _) =
        call(&agent, &token, "POST", "/_pryx/files/move", json!({"from":"src/data.bin","to":"renamed"})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(agent.data.stat("renamed").unwrap().size, bytes.len() as u64);
}

#[tokio::test]
async fn terminal_has_a_controlling_pty_and_resizes() {
    let (_dir, agent, _) = fixture().await;
    let mut command = request("stty size; read line; stty size");
    command.pty = true;
    command.stdin = true;
    command.cols = Some(120);
    command.rows = Some(40);
    let process = agent.processes.start(command, "terminal", Some(5)).unwrap();
    let (replay, mut events) = process.subscribe().await;
    let first = if let Some(first) = replay.first() {
        first.clone()
    } else {
        tokio::time::timeout(Duration::from_secs(2), events.recv()).await.unwrap().unwrap()
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    let Event::Output { bytes, .. } = first else { panic!("expected terminal output") };
    assert!(String::from_utf8(STANDARD.decode(bytes).unwrap()).unwrap().contains("40 120"));
    // Resize uses the public HTTP route because terminal ownership stays internal.
    let token = agent
        .key
        .sign(&Claims {
            sandbox: "one".into(),
            epoch: 1,
            issued_at: now(),
            expires_at: now() + 60,
            scope: Scope::Sandbox,
            max_ports: 0,
            max_exec_seconds: Some(5),
            file: None,
        })
        .unwrap();
    let (status, _) =
        call(&agent, &token, "POST", &format!("/_pryx/processes/{}/resize", process.id), json!({"cols":80,"rows":24}))
            .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    process.write(b"next\n").await.unwrap();
    drop(events);
    assert_eq!(process.wait().await.code, Some(0));
    let (events, _) = process.subscribe().await;
    let output = events
        .into_iter()
        .filter_map(|e| match e {
            Event::Output { bytes, .. } => Some(STANDARD.decode(bytes).unwrap()),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    assert!(String::from_utf8(output).unwrap().contains("24 80"));
}
