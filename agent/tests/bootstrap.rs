// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The real bootstrap executable preserves listener addresses and retires an intentional stop.
//! Kernel process-tier restrictions have separate fail-closed tests; these use local owned agents.

use pryx_agent::auth::SigningKey;
use std::{
    net::{Ipv4Addr, TcpListener, UdpSocket},
    process::{Child, Command, Stdio},
    time::Duration,
};

struct Supervisor(Child);
impl Drop for Supervisor {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn failed_request(child: &mut Child, problem: impl std::fmt::Display, secret: &str) -> ! {
    use std::io::Read;
    let status = child.try_wait();
    let _ = child.kill();
    let _ = child.wait();
    let mut bytes = Vec::new();
    if let Some(stderr) = child.stderr.take() {
        let _ = stderr.take(8192).read_to_end(&mut bytes);
    }
    let diagnostics = String::from_utf8_lossy(&bytes).replace(secret, "[redacted]");
    panic!("Bootstrap request failed: {problem}; child before cleanup: {status:?}; stderr: {diagnostics}");
}

#[tokio::test]
async fn agent_and_spare_listeners_are_unreachable_on_non_configured_interfaces() {
    // RFC 5737 TEST-NET-1 chooses an outbound route without sending a UDP datagram.
    let route = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    route.connect("192.0.2.1:9").unwrap();
    let outside = route.local_addr().unwrap().ip();
    assert!(!outside.is_loopback() && !outside.is_unspecified());
    let first = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let second = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let agent_port = first.local_addr().unwrap().port();
    let spare_port = second.local_addr().unwrap().port();
    let addresses =
        serde_json::json!({agent_port.to_string(): ["127.0.0.1"], spare_port.to_string(): ["127.0.0.2"]}).to_string();
    let data = tempfile::tempdir().unwrap();
    let (_, encoded) = SigningKey::generate().unwrap();
    drop((first, second));
    let mut child = Supervisor(
        Command::new(env!("CARGO_BIN_EXE_pryx-agent"))
            .args([
                "--sandbox",
                "one",
                "--epoch",
                "1",
                "--data",
                data.path().to_str().unwrap(),
                "--domain",
                "pryx.test",
                "--port",
                &agent_port.to_string(),
                "--spare-ports",
                &spare_port.to_string(),
                "--secret-env",
                "TEST_BOOTSTRAP_SECRET",
                "--host-bindings",
                &addresses,
                "--keep-alive",
            ])
            .env("TEST_BOOTSTRAP_SECRET", &encoded)
            .env_remove(pryx_agent::process_policy::POLICY_ENV)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, agent_port)).await.is_ok() {
            break;
        }
        assert!(child.0.try_wait().unwrap().is_none(), "bootstrap exited before listening");
        assert!(tokio::time::Instant::now() < deadline, "bootstrap did not listen");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(tokio::net::TcpStream::connect((outside, agent_port)).await.is_err());
    assert!(tokio::net::TcpStream::connect(("127.0.0.2", agent_port)).await.is_err());
    tokio::net::TcpStream::connect(("127.0.0.2", spare_port))
        .await
        .unwrap_or_else(|problem| failed_request(&mut child.0, problem, &encoded));
    assert!(tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, spare_port)).await.is_err());
    assert!(tokio::net::TcpStream::connect((outside, spare_port)).await.is_err());
}

#[tokio::test]
async fn process_spare_cannot_forward_a_service_outside_its_supervisor_tree() {
    use pryx_agent::auth::{Claims, Scope, TOKEN_LIFETIME_SECONDS};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let _ = rustls::crypto::ring::default_provider().install_default();
    let first = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let second = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let agent_port = first.local_addr().unwrap().port();
    let spare_port = second.local_addr().unwrap().port();
    let bindings =
        serde_json::json!({agent_port.to_string(): ["127.0.0.1"], spare_port.to_string(): ["127.0.0.1"]}).to_string();
    let data = tempfile::tempdir().unwrap();
    let (key, encoded) = SigningKey::generate().unwrap();
    drop((first, second));
    let mut child = Supervisor(
        Command::new(env!("CARGO_BIN_EXE_pryx-agent"))
            .args([
                "--sandbox",
                "one",
                "--epoch",
                "1",
                "--data",
                data.path().to_str().unwrap(),
                "--domain",
                "pryx.test",
                "--port",
                &agent_port.to_string(),
                "--spare-ports",
                &spare_port.to_string(),
                "--secret-env",
                "TEST_BOOTSTRAP_SECRET",
                "--host-bindings",
                &bindings,
                "--keep-alive",
            ])
            .env("TEST_BOOTSTRAP_SECRET", &encoded)
            .env_remove(pryx_agent::process_policy::POLICY_ENV)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, agent_port)).await.is_err() {
        assert!(child.0.try_wait().unwrap().is_none());
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let victim = listener.local_addr().unwrap().port();
    let app = axum::Router::new().route(
        "/",
        axum::routing::get(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                "victim data"
            }
        }),
    );
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    let now = pryx_agent::telemetry::now();
    let token = key
        .sign(&Claims {
            sandbox: "one".into(),
            epoch: 1,
            issued_at: now,
            expires_at: now + TOKEN_LIFETIME_SECONDS,
            scope: Scope::Sandbox,
            max_ports: 1,
            max_exec_seconds: Some(10),
            file: None,
        })
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(3)).build().unwrap();
    let assigned = client
        .post(format!("http://127.0.0.1:{agent_port}/_pryx/ports"))
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::json!({"port":victim}).to_string())
        .send()
        .await
        .unwrap_or_else(|problem| failed_request(&mut child.0, problem, &encoded));
    assert!(assigned.status().is_success());
    let response = client
        .get(format!("http://127.0.0.1:{spare_port}/"))
        .header(pryx_agent::contract::FORWARD_HEADER, victim.to_string())
        .send()
        .await;
    server.abort();
    assert!(response.is_err(), "the process proxy reached a co-resident service");
    assert_eq!(hits.load(Ordering::SeqCst), 0, "the victim received an application request");
}

struct OwnedPty(std::os::fd::OwnedFd);

impl Drop for OwnedPty {
    fn drop(&mut self) {
        let _ = rustix::process::pidfd_send_signal(&self.0, rustix::process::Signal::KILL);
    }
}

#[tokio::test]
async fn intentional_stop_retires_a_detached_term_resistant_pty() {
    use pryx_agent::auth::{Claims, Scope, TOKEN_LIFETIME_SECONDS};
    use pryx_agent::process::{Command as ProcessCommand, Start};

    let _ = rustls::crypto::ring::default_provider().install_default();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let bindings = serde_json::json!({port.to_string(): ["127.0.0.1"]}).to_string();
    let data = tempfile::tempdir().unwrap();
    let (key, encoded) = SigningKey::generate().unwrap();
    drop(listener);
    let mut child = Supervisor(
        Command::new(env!("CARGO_BIN_EXE_pryx-agent"))
            .args([
                "--sandbox",
                "one",
                "--epoch",
                "1",
                "--data",
                data.path().to_str().unwrap(),
                "--domain",
                "pryx.test",
                "--port",
                &port.to_string(),
                "--secret-env",
                "TEST_BOOTSTRAP_SECRET",
                "--host-bindings",
                &bindings,
                "--keep-alive",
            ])
            .env("TEST_BOOTSTRAP_SECRET", &encoded)
            .env_remove(pryx_agent::process_policy::POLICY_ENV)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.is_err() {
        assert!(child.0.try_wait().unwrap().is_none(), "bootstrap exited before listening");
        assert!(tokio::time::Instant::now() < until, "bootstrap did not listen");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let now = pryx_agent::telemetry::now();
    let token = key
        .sign(&Claims {
            sandbox: "one".into(),
            epoch: 1,
            issued_at: now,
            expires_at: now + TOKEN_LIFETIME_SECONDS,
            scope: Scope::Sandbox,
            max_ports: 0,
            max_exec_seconds: None,
            file: None,
        })
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(3)).build().unwrap();
    let request = Start {
        command: ProcessCommand::Shell(
            "trap '' TERM; printf 'term-resistant-ready\\n'; \
             while read line; do printf 'received:%s\\n' \"$line\"; done"
                .into(),
        ),
        env: Default::default(),
        cwd: None,
        stdin: true,
        pty: true,
        cols: Some(80),
        rows: Some(24),
        timeout: None,
    };
    let response = client
        .post(format!("http://127.0.0.1:{port}/_pryx/processes"))
        .bearer_auth(&token)
        .header(pryx_agent::contract::IDEMPOTENCY_HEADER, "shutdown-pty")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(&request).unwrap())
        .send()
        .await
        .unwrap_or_else(|problem| failed_request(&mut child.0, problem, &encoded));
    assert!(response.status().is_success());
    let process: serde_json::Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    let id = process["id"].as_str().unwrap();
    let pid = i32::try_from(process["pid"].as_u64().unwrap()).unwrap();
    let owned_pty = OwnedPty(
        rustix::process::pidfd_open(rustix::process::Pid::from_raw(pid).unwrap(), rustix::process::PidfdFlags::empty())
            .unwrap(),
    );
    let socket = format!("ws://127.0.0.1:{port}/_pryx/processes/{id}/ws");
    detached_terminal_output(&socket, &token, "term-resistant-ready").await;
    let written = client
        .post(format!("http://127.0.0.1:{port}/_pryx/processes/{id}/stdin"))
        .bearer_auth(&token)
        .body("live\n")
        .send()
        .await
        .unwrap();
    assert!(written.status().is_success());
    detached_terminal_output(&socket, &token, "received:live").await;
    let observed =
        client.get(format!("http://127.0.0.1:{port}/_pryx/processes/{id}")).bearer_auth(&token).send().await.unwrap();
    let observed: serde_json::Value = serde_json::from_slice(&observed.bytes().await.unwrap()).unwrap();
    assert_eq!(observed["running"], true, "detaching does not retire a live terminal");
    println!("real bootstrap holds a detached TERM-resistant PTY");
    rustix::process::kill_process(rustix::process::Pid::from_child(&child.0), rustix::process::Signal::TERM).unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(tokio::time::Instant::now() < until, "intentional stop retained live PTY blocking I/O");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(status.code(), Some(0), "bootstrap handles TERM without being killed by its owner");
    assert!(
        std::fs::symlink_metadata(format!("/proc/{pid}"))
            .is_err_and(|problem| problem.kind() == std::io::ErrorKind::NotFound),
        "the owned TERM-resistant PTY is retired before bootstrap exit"
    );
    drop(owned_pty);
}

async fn detached_terminal_output(url: &str, token: &str, expected: &str) {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    tokio::time::timeout(Duration::from_secs(3), async {
        let mut request = url.into_client_request().unwrap();
        request.headers_mut().insert("authorization", format!("Bearer {token}").parse().unwrap());
        request
            .headers_mut()
            .insert("sec-websocket-protocol", pryx_agent::contract::WEBSOCKET_PROTOCOL.parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let mut output = String::new();
        loop {
            let frame = socket.next().await.unwrap().unwrap();
            if !frame.is_text() {
                continue;
            }
            let event: serde_json::Value = serde_json::from_str(&frame.into_text().unwrap()).unwrap();
            if event["type"] == "output" {
                let bytes = STANDARD.decode(event["bytes"].as_str().unwrap()).unwrap();
                output.push_str(&String::from_utf8(bytes).unwrap());
                if output.contains(expected) {
                    socket.close(None).await.unwrap();
                    break;
                }
            }
        }
    })
    .await
    .expect("the owned live terminal reaches its expected output before detachment");
}
