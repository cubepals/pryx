// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Real websocket and TCP attachment, slot reuse and filesystem notification boundaries. These
//! tests run unprivileged and leave container, namespace and network policy proofs to CI.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use pryx_agent::auth::{Claims, Scope, SigningKey};
use pryx_agent::paths::DataDir;
use pryx_agent::ports::{PortError, Ports};
use pryx_agent::process::{Command, Processes, Start};
use pryx_agent::server::{Agent, now, router};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (url, server)
}

async fn fixture() -> (tempfile::TempDir, Arc<Agent>, String) {
    let dir = tempfile::tempdir().unwrap();
    let (key, _) = SigningKey::generate().unwrap();
    let token = key
        .sign(&Claims {
            sandbox: "one".into(),
            epoch: 1,
            issued_at: now(),
            expires_at: now() + 60,
            scope: Scope::Sandbox,
            max_ports: 1,
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
        processes: Processes::new(dir.path().into(), "PRIVATE_BOOTSTRAP".into()),
        ports: Ports::bind("one".into(), "example.test".into(), 0).await.unwrap(),
    });
    (dir, agent, token)
}

#[tokio::test]
async fn websocket_authenticates_before_attachment_and_passes_stdin() {
    let (_dir, agent, token) = fixture().await;
    let process = agent
        .processes
        .start(
            Start {
                command: Command::Shell("printf ready; read value; printf '%s' \"$value\"".into()),
                env: Default::default(),
                cwd: None,
                stdin: true,
                pty: false,
                cols: None,
                rows: None,
                timeout: Some(5),
            },
            "socket",
            Some(10),
        )
        .unwrap();
    let (url, server) = serve(router(agent)).await;
    let url = format!("{url}/_pryx/processes/{}/ws", process.id);
    assert!(tokio_tungstenite::connect_async(&url).await.is_err());
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert("sec-websocket-protocol", format!("pryx, pryx-token.{token}").parse().unwrap());
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.headers()["sec-websocket-protocol"], "pryx");
    let first = tokio::time::timeout(Duration::from_secs(2), socket.next()).await.unwrap().unwrap().unwrap();
    assert!(first.into_text().unwrap().contains("output"));
    assert!(process.view().running);
    socket
        .send(Message::Text(serde_json::json!({"type":"stdin","bytes":"aGVsbG8K"}).to_string().into()))
        .await
        .unwrap();
    let mut exited = false;
    while let Some(Ok(message)) = tokio::time::timeout(Duration::from_secs(2), socket.next()).await.unwrap() {
        if message.is_text() && message.into_text().unwrap().contains("\"exit\"") {
            exited = true;
            break;
        }
    }
    assert!(exited);
    assert_eq!(process.wait().await.code, Some(0));
    server.abort();
}

#[tokio::test]
async fn file_watch_observes_the_open_directory() {
    let (dir, agent, token) = fixture().await;
    let (url, server) = serve(router(agent)).await;
    let mut request = format!("{url}/_pryx/files/watch?path=.").into_client_request().unwrap();
    request.headers_mut().insert("authorization", format!("Bearer {token}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let ready = socket.next().await.unwrap().unwrap().into_text().unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&ready).unwrap()["type"], "ready");
    std::fs::write(dir.path().join("changed"), b"hello").unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), socket.next()).await.unwrap().unwrap().unwrap();
    let event: serde_json::Value = serde_json::from_str(&event.into_text().unwrap()).unwrap();
    assert_eq!(event["type"], "change");
    assert!(event["paths"].as_array().unwrap().iter().any(|p| p == "changed"));
    server.abort();
}

#[tokio::test]
async fn spare_ports_forward_locally_and_unexpose_closes_connections() {
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_port = target.local_addr().unwrap().port();
    let reserve = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let spare = reserve.local_addr().unwrap().port();
    let ports = Ports::bind_listeners(
        "one".into(),
        "example.test".into(),
        5280,
        vec![(spare, vec![reserve])],
        None,
        Arc::default(),
        None,
    )
    .await
    .unwrap();
    assert!(matches!(ports.expose(target_port, 0).await, Err(PortError::NoSparePort)));
    let exposed = ports.expose(target_port, 1).await.unwrap();
    assert_eq!(exposed.url, format!("https://one-{target_port}.example.test"));
    assert_eq!(exposed.slot, spare);
    assert!(matches!(ports.expose(target_port - 1, 1).await, Err(PortError::NoSparePort)));
    assert!(matches!(ports.expose(spare, 1).await, Err(PortError::Invalid)));
    let echo = tokio::spawn(async move {
        let (mut socket, _) = target.accept().await.unwrap();
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(socket.read_u8().await.unwrap());
        }
        assert!(!String::from_utf8_lossy(&bytes).contains(pryx_agent::contract::FORWARD_HEADER));
        socket.read_exact(&mut [0; 5]).await.unwrap();
        socket.write_all(b"hello").await.unwrap();
        std::future::pending::<()>().await;
    });
    let mut connection = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, spare)).await.unwrap();
    connection
        .write_all(
            format!(
                "POST / HTTP/1.1\r\nHost: example.test\r\n{}: {target_port}\r\n\r\nhello",
                pryx_agent::contract::FORWARD_HEADER
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut bytes = [0; 5];
    connection.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"hello");
    ports.unexpose(target_port).await.unwrap();
    assert_eq!(tokio::time::timeout(Duration::from_secs(2), connection.read(&mut bytes)).await.unwrap().unwrap(), 0);
    assert!(ports.list().await.is_empty());
    ports.expose(target_port - 1, 1).await.unwrap();
    let mut stale = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, spare)).await.unwrap();
    stale
        .write_all(
            format!("GET / HTTP/1.1\r\n{}: {target_port}\r\n\r\n", pryx_agent::contract::FORWARD_HEADER).as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(tokio::time::timeout(Duration::from_secs(2), stale.read(&mut bytes)).await.unwrap().unwrap(), 0);
    echo.abort();
}

#[tokio::test]
async fn port_assignments_restore_from_an_atomic_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::open(dir.path()).unwrap();
    pryx_agent::state::write(
        &data,
        pryx_agent::state::PORTS_PATH,
        &pryx_agent::state::PortState { revision: 7, targets: vec![Some(3000)] },
        true,
    )
    .unwrap();
    let reserve = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let spare = reserve.local_addr().unwrap().port();
    let ports = Ports::bind_listeners(
        "one".into(),
        "example.test".into(),
        5280,
        vec![(spare, vec![reserve])],
        Some(data),
        Arc::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(ports.revision(), 7);
    assert_eq!(ports.list().await[0].port, 3000);
    ports.unexpose(3000).await.unwrap();
    let state: pryx_agent::state::PortState =
        pryx_agent::state::read(&DataDir::open(dir.path()).unwrap(), pryx_agent::state::PORTS_PATH).unwrap();
    assert_eq!(state.revision, 8);
    assert_eq!(state.targets, vec![None]);
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("unsafe")).unwrap();
    assert!(DataDir::open(dir.path()).unwrap().write_atomic("unsafe/file", b"denied", true).is_err());
    assert!(!outside.path().join("file").exists());
}
