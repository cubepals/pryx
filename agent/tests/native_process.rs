// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real native tools through independent, fully confined agent executables. Explicit CI
//! supplies the verified archive; ordinary unit runs never acquire these large payloads.
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use pryx_agent::{
    auth::{Claims, Scope, SigningKey, TOKEN_LIFETIME_SECONDS},
    process_policy::{POLICY_ENV, Policy},
    process_tool_policy,
};
use serde_json::{Value, json};
use std::{
    fs,
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Agent {
    child: Child,
    directory: tempfile::TempDir,
    url: String,
    token: String,
    cache: PathBuf,
    scratch: PathBuf,
    spare: u16,
}
impl Drop for Agent {
    fn drop(&mut self) {
        if std::thread::panicking() {
            diagnostic(self.directory.path());
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn diagnostic(directory: &Path) {
    for entry in fs::read_dir(directory).unwrap().flatten() {
        if entry.file_type().unwrap().is_dir() {
            diagnostic(&entry.path());
        } else if matches!(entry.file_name().to_str(), Some("agent.stderr" | "browser.stderr")) {
            let bytes = fs::read(entry.path()).unwrap();
            eprintln!(
                "Native startup diagnostic: {}",
                String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(16 * 1024)..])
            );
        }
    }
}
fn start(artifact: process_tool_policy::Artifact, browser_on: bool, unavailable: Option<String>) -> Agent {
    let directory = tempfile::tempdir().unwrap();
    for name in ["data", "cache", "downloads", "scratch"] {
        fs::create_dir(directory.path().join(name)).unwrap();
    }
    let cache = directory.path().join("cache");
    let scratch = directory.path().join("scratch");
    let (key, encoded) = SigningKey::generate().unwrap();
    let now = pryx_agent::server::now();
    let token = key
        .sign(&Claims {
            sandbox: "native-fixture".into(),
            epoch: 1,
            issued_at: now,
            expires_at: now + TOKEN_LIFETIME_SECONDS,
            scope: Scope::Sandbox,
            max_ports: 1,
            max_exec_seconds: Some(30),
            file: None,
        })
        .unwrap();
    let first = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let second = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = first.local_addr().unwrap().port();
    let spare = second.local_addr().unwrap().port();
    drop((first, second));
    let policy = Policy {
        data: directory.path().join("data"),
        memory_bytes: 2 * 1024 * 1024 * 1024,
        address_space_bytes: 128 * 1024 * 1024 * 1024,
        open_files: 4096,
        processes: 1024,
        cpu_seconds: 600,
        nice: 19,
        network_off: false,
        require_ipc_scope: Some(true),
        tools: Some(process_tool_policy::Policy {
            cache: cache.clone(),
            downloads: directory.path().join("downloads"),
            scratch: scratch.clone(),
            artifact: Some(artifact),
            unavailable,
            browser_on,
        }),
    };
    let child = Command::new(env!("CARGO_BIN_EXE_pryx-agent"))
        .args([
            "--sandbox",
            "native-fixture",
            "--epoch",
            "1",
            "--domain",
            "example.test",
            "--secret-env",
            "NATIVE_FIXTURE_KEY",
            "--keep-alive",
        ])
        .arg("--data")
        .arg(&policy.data)
        .args([
            "--port",
            &port.to_string(),
            "--spare-ports",
            &spare.to_string(),
            "--host-bindings",
            &json!({port.to_string():["127.0.0.1"],spare.to_string():["127.0.0.1"]}).to_string(),
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &policy.data)
        .env("TMPDIR", &policy.data)
        .env("NATIVE_FIXTURE_KEY", encoded)
        .env(POLICY_ENV, serde_json::to_string(&policy).unwrap())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(directory.path().join("agent.stderr")).unwrap())
        .spawn()
        .unwrap();
    Agent { child, directory, url: format!("http://127.0.0.1:{port}"), token, cache, scratch, spare }
}
impl Agent {
    async fn healthy(&self) {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(1)).build().unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if client
                .get(format!("{}/_pryx/health", self.url))
                .bearer_auth(&self.token)
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                fs::read_to_string(self.directory.path().join("agent.stderr")).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    async fn call(&self, method: reqwest::Method, path: &str, body: Value) -> reqwest::Response {
        reqwest::Client::new()
            .request(method, format!("{}{path}", self.url))
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .body(body.to_string())
            .timeout(Duration::from_secs(180))
            .send()
            .await
            .unwrap()
    }
    async fn json(&self, method: reqwest::Method, path: &str, body: Value) -> Value {
        let response = self.call(method, path, body).await;
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(status.is_success(), "{path}: {status} {text}");
        serde_json::from_str(&text).unwrap()
    }
    async fn execute(&self, id: &str, code: &str) -> Vec<Value> {
        let response = self
            .call(reqwest::Method::POST, &format!("/_pryx/kernels/{id}/execute"), json!({"code":code,"timeout":30}))
            .await;
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(status.is_success(), "{text}");
        let events: Vec<Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert!(events.iter().any(|e| e["type"] == "done" && e["status"] == "ok"), "{text}");
        assert!(!events.iter().any(|e| e["type"] == "error"), "{text}");
        events
    }
    async fn probe_sockets(&self, addresses: &[String], allowed: bool) {
        let code = r#"import json,socket,sys
for address in json.loads(sys.argv[1]):
    client=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);client.settimeout(2)
    try: client.connect('\0'+address)
    except PermissionError: assert sys.argv[2]=='false'
    else: assert sys.argv[2]=='true'
    finally: client.close()
"#;
        let started=self.json(reqwest::Method::POST,"/_pryx/processes",json!({"command":["/usr/bin/python3","-c",code,serde_json::to_string(addresses).unwrap(),allowed.to_string()],"timeout":10})).await;
        let result = self
            .json(
                reqwest::Method::GET,
                &format!("/_pryx/processes/{}/wait", started["id"].as_str().unwrap()),
                Value::Null,
            )
            .await;
        assert_eq!(result["code"], 0, "{result}");
    }
}
fn artifact() -> process_tool_policy::Artifact {
    serde_json::from_slice(
        &fs::read(std::env::var_os("PRYX_NATIVE_TOOL_DESCRIPTOR").expect("CI supplies a pinned native artifact"))
            .unwrap(),
    )
    .unwrap()
}
fn require_host() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    assert!(!rustix::process::geteuid().is_root());
    assert!(pryx_agent::process_capabilities::probe().filesystem);
    assert!(pryx_agent::process_capabilities::private_ipc_probe().supported());
}
fn kernel_addresses(agent: &Agent) -> Vec<String> {
    let mut addresses = Vec::new();
    let mut count = 0;
    for entry in fs::read_dir(&agent.scratch).unwrap().flatten() {
        let path = entry.path().join("connection.json");
        if !path.is_file() {
            continue;
        }
        let info: jupyter_protocol::ConnectionInfo = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert!(matches!(info.transport, jupyter_protocol::connection_info::Transport::IPC));
        let ip = info.ip.strip_prefix('@').expect("libzmq abstract IPC prefix");
        count += 1;
        // jupyter_client's IPC connection address is the connection prefix plus channel number.
        for port in [info.shell_port, info.iopub_port, info.stdin_port, info.control_port, info.hb_port] {
            addresses.push(format!("{ip}-{port}"));
        }
    }
    assert_eq!(count, 2);
    addresses
}

#[tokio::test]
#[ignore = "explicit hosted artifact and process-policy acceptance"]
async fn confined_tools_keep_kernel_state_images_private_channels_and_browser_proxy() {
    require_host();
    let artifact = artifact();
    let agent = start(artifact.clone(), true, None);
    agent.healthy().await;
    assert_eq!(fs::read_dir(&agent.cache).unwrap().count(), 0, "health must not acquire tools");
    for language in ["python", "javascript"] {
        let opened =
            agent.json(reqwest::Method::POST, "/_pryx/kernels", json!({"language":language,"context":"native"})).await;
        let id = opened["id"].as_str().unwrap();
        agent.execute(id, if language == "python" { "value=40" } else { "var value=40" }).await;
        assert!(agent.execute(id, "value + 2").await.iter().any(|e| e["data"]["text/plain"] == "42"));
        let chart = if language == "python" {
            "import matplotlib.pyplot as plt; from IPython.display import display; plt.plot([1,2,3]); display(plt.gcf()); plt.close()"
        } else {
            "var c=require('@napi-rs/canvas').createCanvas(32,32); c.getContext('2d').fillRect(0,0,16,16); $$.mime({'image/png':c.toBuffer('image/png').toString('base64')})"
        };
        assert!(
            agent
                .execute(id, chart)
                .await
                .iter()
                .filter_map(|e| e["data"]["image/png"].as_str())
                .any(|image| STANDARD.decode(image).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"))
        );
        println!(
            "{}",
            json!({"language":language,"scope":"full-process-policy","stateRetained":true,"png":true,"uid":rustix::process::geteuid().as_raw()})
        );
    }
    let addresses = kernel_addresses(&agent);
    agent.probe_sockets(&addresses, true).await;
    let neighbour = start(artifact, false, Some("Neighbour fixture has no native tools".into()));
    neighbour.healthy().await;
    neighbour.probe_sockets(&addresses, false).await;
    let page = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let page_url = format!("http://{}/", page.local_addr().unwrap());
    let page = tokio::spawn(async move {
        axum::serve(
            page,
            axum::Router::new()
                .route("/", axum::routing::get(|| async { axum::response::Html("<h1>Native browser 漢字 😀</h1>") })),
        )
        .await
        .unwrap();
    });
    let response = agent.call(reqwest::Method::POST, "/_pryx/browser/screenshot", json!({"url":page_url})).await;
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert!(status.is_success(), "browser {status}: {}", String::from_utf8_lossy(&bytes));
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    let access = agent.json(reqwest::Method::POST, "/_pryx/browser", Value::Null).await;
    let external = reqwest::Url::parse(access["url"].as_str().unwrap()).unwrap();
    let query = external.query().unwrap();
    let client = reqwest::Client::new();
    let forwarded = format!("http://127.0.0.1:{}/json/version", agent.spare);
    let port = pryx_agent::contract::BROWSER_PORT.to_string();
    assert_eq!(
        client.get(&forwarded).header(pryx_agent::contract::FORWARD_HEADER, &port).send().await.unwrap().status(),
        axum::http::StatusCode::FORBIDDEN
    );
    let response = client
        .get(format!("{forwarded}?{query}"))
        .header(pryx_agent::contract::FORWARD_HEADER, &port)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let discovered: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert!(discovered["webSocketDebuggerUrl"].as_str().unwrap().contains(external.host_str().unwrap()));
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    let mut request =
        format!("ws://127.0.0.1:{}/devtools/browser/pryx?{query}", agent.spare).into_client_request().unwrap();
    request.headers_mut().insert(pryx_agent::contract::FORWARD_HEADER, port.parse().unwrap());
    let (mut websocket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    websocket.send(Message::Text(json!({"id":1,"method":"Browser.getVersion"}).to_string().into())).await.unwrap();
    let version = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let value: Value =
                serde_json::from_str(websocket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            if value["id"] == 1 {
                break value;
            }
        }
    })
    .await
    .unwrap();
    assert!(version["result"]["product"].as_str().unwrap().contains("Chrome"));
    websocket.close(None).await.unwrap();
    page.abort();
    println!(
        "{}",
        json!({"browser":true,"scope":"full-process-policy","screenshot":true,"forwardedCdp":true,"sameUidNeighbourRefused":true,"uid":rustix::process::geteuid().as_raw()})
    );
}

#[tokio::test]
#[ignore = "explicit hosted artifact and process-policy acceptance"]
async fn unavailable_browser_names_namespace_policy_without_acquiring_tools() {
    require_host();
    let agent = start(artifact(), false, None);
    agent.healthy().await;
    let response = agent.call(reqwest::Method::POST, "/_pryx/browser", Value::Null).await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_IMPLEMENTED);
    let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("user namespaces"));
    assert_eq!(fs::read_dir(&agent.cache).unwrap().count(), 0);
}
