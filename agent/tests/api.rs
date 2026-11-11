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
        file_tasks: Default::default(),
        shells: Default::default(),
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
async fn route_capabilities_allow_only_readiness_and_port_reads() {
    let (_dir, agent, _) = fixture().await;
    let token = agent
        .key
        .sign(&Claims {
            sandbox: "one".into(),
            epoch: 1,
            issued_at: now(),
            expires_at: now() + 60,
            scope: Scope::Routes,
            max_ports: 0,
            max_exec_seconds: None,
            file: None,
        })
        .unwrap();
    assert_eq!(call(&agent, &token, "GET", "/_pryx/health", Value::Null).await.0, StatusCode::OK);
    assert_eq!(call(&agent, &token, "GET", "/_pryx/ports", Value::Null).await.0, StatusCode::OK);
    assert_eq!(
        call(&agent, &token, "POST", "/_pryx/processes", json!({"command":"true"})).await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn process_scope_proofs_are_optional_authenticated_and_bound_to_actual_state() {
    let (_dir, agent, token) = fixture().await;
    let (_, legacy) = call(&agent, &token, "GET", "/_pryx/health", Value::Null).await;
    assert!(legacy.get("processObservation").is_none());
    for nonce in ["fresh-node-challenge".to_string(), "invalid challenge".into(), "oversized".repeat(128)] {
        let response = router(agent.clone())
            .oneshot(
                Request::builder()
                    .uri("/_pryx/health")
                    .header("authorization", format!("Bearer {token}"))
                    .header(pryx_agent::process_observation::NONCE_HEADER, &nonce)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        if pryx_agent::process_observation::nonce_valid(&nonce) {
            let proof: pryx_agent::process_observation::Proof =
                serde_json::from_value(value["processObservation"].clone()).unwrap();
            assert_eq!(proof.observation.nonce, nonce);
            assert_eq!(proof.observation.sandbox, agent.sandbox);
            assert_eq!(proof.observation.epoch, agent.epoch);
            assert_eq!(proof.observation.pid, std::process::id());
            assert_eq!(proof.observation.ipc_scoped, pryx_agent::process_policy::ipc_scoped());
            assert!(!proof.signature.is_empty());
        } else {
            assert!(value.get("processObservation").is_none());
        }
    }
    let response = router(agent)
        .oneshot(
            Request::builder()
                .uri("/_pryx/health")
                .header(pryx_agent::process_observation::NONCE_HEADER, "fresh-node-challenge")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[cfg(feature = "mcp")]
#[tokio::test]
async fn mcp_runs_real_processes_and_enforces_file_and_timeout_authority() {
    let (_dir, agent, token) = fixture().await;
    let rpc = |name: &str, args: Value| json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}});
    let (status, ran) =
        call(&agent, &token, "POST", "/_pryx/mcp", rpc("run", json!({"command":"printf mcp-local"}))).await;
    assert_eq!(status, StatusCode::OK);
    let run: Value = serde_json::from_str(ran["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(run["stdout"], "mcp-local");
    assert_eq!(run["code"], 0);
    let denied = call(&agent, &token, "POST", "/_pryx/mcp", rpc("read", json!({"path":"../outside"}))).await.1;
    assert_eq!(denied["result"]["isError"], true);
    let timeout =
        call(&agent, &token, "POST", "/_pryx/mcp", rpc("run", json!({"command":"true","timeout":11}))).await.1;
    assert_eq!(timeout["result"]["isError"], true);
    let notification =
        call(&agent, &token, "POST", "/_pryx/mcp", json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await;
    assert_eq!(notification.0, StatusCode::ACCEPTED);
    let initialized = call(
        &agent,
        &token,
        "POST",
        "/_pryx/mcp",
        json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
    )
    .await
    .1;
    assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
    let future = router(agent.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_pryx/mcp")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("mcp-protocol-version", "2026-07-28")
                .body(Body::from(rpc("run", json!({"command":"touch future"})).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(future.status(), StatusCode::BAD_REQUEST);
    assert!(agent.data.read("future").is_err());

    assert_eq!(
        call(&agent, "invalid", "POST", "/_pryx/mcp", rpc("run", json!({"command":"true"}))).await.0,
        StatusCode::UNAUTHORIZED
    );
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

#[tokio::test]
async fn remote_transfers_cannot_contact_an_owned_loopback_server_or_replace_files() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (dir, agent, token) = fixture().await;
    std::fs::write(dir.path().join("payload"), b"original").unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let app = axum::Router::new().route(
        "/transfer",
        axum::routing::any(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                "external content"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/transfer", listener.local_addr().unwrap());
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    for path in ["/_pryx/files/upload", "/_pryx/files/download"] {
        let (status, _) = call(&agent, &token, "POST", path, json!({"path":"payload","url":url})).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read(dir.path().join("payload")).unwrap(), b"original");
    server.abort();
}

#[tokio::test]
async fn persistent_shell_keeps_state_and_separates_streams_and_exact_status() {
    let (dir, agent, token) = fixture().await;
    let (_, health) = call(&agent, &token, "GET", "/_pryx/health", Value::Null).await;
    assert_eq!(health["apiVersion"], 2);
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    let (status, shell) = call(&agent, &token, "POST", "/_pryx/shells", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let id = shell["id"].as_str().unwrap();
    let path = format!("/_pryx/shells/{id}/run");
    let (status, result) =
        call(&agent, &token, "POST", &path, json!({"command":"printf out; printf err >&2; false","timeout":2})).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["code"], 1);
    assert_eq!(result["stdout"], "out");
    assert_eq!(result["stderr"], "err");
    let (_, result) = call(
        &agent,
        &token,
        "POST",
        &path,
        json!({"command":"cd nested; export KEPT=yes; n=3; function_kept() { printf function; }","timeout":2}),
    )
    .await;
    assert_eq!(result["code"], 0);
    let (_, result) = call(
        &agent,
        &token,
        "POST",
        &path,
        json!({"command":"if [ \"$KEPT\" = yes ]; then\n printf '%s:' \"$n\"; function_kept; pwd\nfi","timeout":2}),
    )
    .await;
    assert_eq!(result["stdout"], format!("3:function{}\n", dir.path().join("nested").display()));
    let (status, _) = call(&agent, &token, "DELETE", &format!("/_pryx/shells/{id}"), Value::Null).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(agent.processes.get(id).unwrap().wait().await.code, Some(128 + libc::SIGKILL));
}

#[tokio::test]
async fn queued_shell_timeout_signals_without_waiting_for_the_active_command() {
    let (dir, agent, token) = fixture().await;
    let (id, active) = busy_shell(&dir, &agent, &token).await;
    let process = agent.processes.get(&id).unwrap();
    let answer = tokio::time::timeout(
        Duration::from_secs(2),
        call(
            &agent,
            &token,
            "POST",
            &format!("/_pryx/shells/{id}/run"),
            json!({"command":"printf should-not-run","timeout":0.05}),
        ),
    )
    .await;
    let exited =
        if answer.is_ok() { tokio::time::timeout(Duration::from_secs(2), process.wait()).await.ok() } else { None };
    // Also clean up when the response exceeded its guard on the unfixed implementation.
    if process.view().running {
        let _ = process.signal_awaited(libc::SIGKILL).await;
    }
    tokio::time::timeout(Duration::from_secs(2), active).await.unwrap().unwrap();
    let (status, response) = answer.expect("a queued command's timeout must not wait for the active command");
    assert_eq!(status, StatusCode::CONFLICT, "{response}");
    assert_eq!(response["error"]["code"], "wait_timeout");
    assert_eq!(exited.expect("timeout must kill the session").code, Some(128 + libc::SIGKILL));
    assert_eq!(
        call(&agent, &token, "POST", &format!("/_pryx/shells/{id}/run"), json!({"command":"true","timeout":1})).await.1
            ["error"]["code"],
        "not_found"
    );
}

#[tokio::test]
async fn closing_a_busy_shell_signals_without_waiting_for_its_command() {
    let (dir, agent, token) = fixture().await;
    let (id, active) = busy_shell(&dir, &agent, &token).await;
    let process = agent.processes.get(&id).unwrap();
    let answer = tokio::time::timeout(
        Duration::from_secs(2),
        call(&agent, &token, "DELETE", &format!("/_pryx/shells/{id}"), Value::Null),
    )
    .await;
    let exited =
        if answer.is_ok() { tokio::time::timeout(Duration::from_secs(2), process.wait()).await.ok() } else { None };
    if process.view().running {
        let _ = process.signal_awaited(libc::SIGKILL).await;
    }
    tokio::time::timeout(Duration::from_secs(2), active).await.unwrap().unwrap();
    assert_eq!(answer.expect("close must not wait for the active command").0, StatusCode::NO_CONTENT);
    assert_eq!(exited.expect("close must kill the session").code, Some(128 + libc::SIGKILL));
    assert_eq!(
        call(&agent, &token, "DELETE", &format!("/_pryx/shells/{id}"), Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
}

/// Wait for the real command to enter its critical section, rather than racing a timed sleep.
async fn busy_shell(
    dir: &tempfile::TempDir,
    agent: &Arc<Agent>,
    token: &str,
) -> (String, tokio::task::JoinHandle<(StatusCode, Value)>) {
    let (status, shell) = call(agent, token, "POST", "/_pryx/shells", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let id = shell["id"].as_str().unwrap().to_owned();
    let (own, credential, path) = (agent.clone(), token.to_owned(), format!("/_pryx/shells/{id}/run"));
    let active = tokio::spawn(async move {
        call(&own, &credential, "POST", &path, json!({"command":"printf busy > shell-started; sleep 30","timeout":8}))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !dir.path().join("shell-started").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the active command must start");
    (id, active)
}

#[tokio::test]
async fn readiness_distinguishes_exit_and_timeout_and_matches_retained_logs() {
    let (_dir, agent, token) = fixture().await;
    let (_, view) =
        call(&agent, &token, "POST", "/_pryx/processes", json!({"command":"printf 'réady\\n'; sleep 30"})).await;
    let id = view["id"].as_str().unwrap();
    let ready = format!("/_pryx/processes/{id}/ready");
    let (status, result) =
        call(&agent, &token, "POST", &ready, json!({"kind":"log","pattern":"réady","timeout":2})).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let (status, result) =
        call(&agent, &token, "POST", &ready, json!({"kind":"log","pattern":"never","timeout":0.01})).await;
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(result["error"]["code"], "wait_timeout");
    let (_, killed) = call(&agent, &token, "POST", "/_pryx/processes/kill-all", json!({})).await;
    assert!(killed["killed"].as_array().unwrap().iter().any(|p| p == id));
    agent.processes.get(id).unwrap().wait().await;
    let (_, result) = call(&agent, &token, "POST", &ready, json!({"kind":"port","port":12345,"timeout":1})).await;
    assert_eq!(result["error"]["code"], "process_exited");
    let (status, result) = call(&agent, &token, "POST", &ready, json!({"kind":"exit","timeout":1})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["code"], 128 + libc::SIGKILL);
}

#[tokio::test]
async fn search_replace_and_modes_are_confined_and_bounded() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (dir, agent, token) = fixture().await;
    std::fs::write(dir.path().join("known.txt"), "first\nneedle here\nlast\n").unwrap();
    let (_, found) = call(&agent, &token, "POST", "/_pryx/files/find", json!({"pattern":"**/*.txt"})).await;
    assert_eq!(found["paths"], json!(["known.txt"]));
    let (_, found) =
        call(&agent, &token, "POST", "/_pryx/files/grep", json!({"pattern":"needle","paths":["known.txt"]})).await;
    assert_eq!(found["matches"][0]["line"], 2);
    let (_, changed) = call(
        &agent,
        &token,
        "POST",
        "/_pryx/files/replace",
        json!({"pattern":"needle","text":"replacement","paths":["known.txt"]}),
    )
    .await;
    assert_eq!(changed["replacements"], 1);
    assert!(std::fs::read_to_string(dir.path().join("known.txt")).unwrap().contains("replacement here"));
    let (status, _) =
        call(&agent, &token, "POST", "/_pryx/files/chmod", json!({"path":"known.txt","mode":0o600})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(std::fs::metadata(dir.path().join("known.txt")).unwrap().permissions().mode() & 0o777, 0o600);
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("private");
    std::fs::write(&path, "untouched").unwrap();
    symlink(&path, dir.path().join("link")).unwrap();
    for route in ["grep", "replace"] {
        let (status, _) = call(
            &agent,
            &token,
            "POST",
            &format!("/_pryx/files/{route}"),
            json!({"pattern":"untouched","text":"bad","paths":["link"]}),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), "untouched");
    let (status, _) =
        call(&agent, &token, "POST", "/_pryx/files/chmod", json!({"path":"known.txt","mode":0o4755})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn directory_archives_preserve_empty_dirs_and_refuse_links_and_traversal() {
    use std::io::Read;
    use std::os::unix::fs::symlink;
    let (dir, agent, token) = fixture().await;
    std::fs::create_dir_all(dir.path().join("tree/empty")).unwrap();
    std::fs::write(dir.path().join("tree/content"), b"kept").unwrap();
    let response = router(agent.clone())
        .oneshot(
            Request::builder()
                .uri("/_pryx/files/archive?path=tree")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let mut archive = tar::Archive::new(bytes.as_ref());
    let mut names = Vec::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        names.push(path.clone());
        if path == "content" {
            let mut text = String::new();
            entry.read_to_string(&mut text).unwrap();
            assert_eq!(text, "kept");
        }
    }
    assert!(names.iter().any(|name| name.trim_end_matches('/') == "empty"));
    let send = |data: Vec<u8>| {
        router(agent.clone()).oneshot(
            Request::builder()
                .method("PUT")
                .uri("/_pryx/files/archive?path=target")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(data))
                .unwrap(),
        )
    };
    assert_eq!(send(bytes.to_vec()).await.unwrap().status(), StatusCode::NO_CONTENT);
    assert_eq!(std::fs::read(dir.path().join("target/content")).unwrap(), b"kept");
    assert!(dir.path().join("target/empty").is_dir());
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("private"), b"untouched").unwrap();
    symlink(outside.path(), dir.path().join("target/link")).unwrap();
    for (path, kind) in [("link/private", tar::EntryType::Regular), ("escape", tar::EntryType::Symlink)] {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_mode(0o600);
        if kind.is_symlink() {
            header.set_link_name("../../outside").unwrap();
            header.set_size(0);
        } else {
            header.set_size(3);
        }
        header.set_cksum();
        builder.append_data(&mut header, path, if kind.is_symlink() { &b""[..] } else { &b"bad"[..] }).unwrap();
        assert_eq!(send(builder.into_inner().unwrap()).await.unwrap().status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let mut malicious = bytes.to_vec();
    malicious[..100].fill(0);
    malicious[..10].copy_from_slice(b"../outside");
    malicious[148..156].fill(b' ');
    let checksum: usize = malicious[..512].iter().map(|b| *b as usize).sum();
    malicious[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
    assert_eq!(send(malicious).await.unwrap().status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(std::fs::read(outside.path().join("private")).unwrap(), b"untouched");
}

#[tokio::test]
async fn replacement_refuses_expansion_before_replacing_the_file() {
    let (dir, agent, token) = fixture().await;
    let original = "a".repeat(4096);
    std::fs::write(dir.path().join("bounded"), &original).unwrap();
    let (status, _) = call(
        &agent,
        &token,
        "POST",
        "/_pryx/files/replace",
        json!({"pattern":"a","text":"b".repeat(4096),"paths":["bounded"]}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(std::fs::read_to_string(dir.path().join("bounded")).unwrap(), original);
}

#[tokio::test]
async fn released_tool_commands_leave_the_table_while_running_ones_stay() {
    let (_dir, agent, _) = fixture().await;
    let done = agent.processes.start(request("true"), "released", Some(5)).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while done.view().running && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!done.view().running);
    agent.processes.release(&done.id);
    assert!(agent.processes.get(&done.id).is_err());
    let kept = agent.processes.start(request("sleep 5"), "kept", Some(5)).unwrap();
    agent.processes.release(&kept.id);
    assert!(agent.processes.get(&kept.id).is_ok());
    kept.signal(libc::SIGKILL).unwrap();
}

#[tokio::test]
async fn kernels_that_fail_their_first_start_free_their_context() {
    // No image here installs the JavaScript kernel, so its first start fails at once.
    if std::path::Path::new("/opt/pryx/kernel.cjs").exists() {
        return;
    }
    let (_dir, agent, token) = fixture().await;
    for n in 0..17 {
        let (status, body) =
            call(&agent, &token, "POST", "/_pryx/kernels", json!({"language":"javascript","context":format!("c{n}")}))
                .await;
        assert!(!status.is_success(), "{body}");
        assert_ne!(body["error"]["code"], "kernel_capacity", "{body}");
    }
    let (status, body) = call(&agent, &token, "GET", "/_pryx/kernels", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["kernels"], json!([]));
}
