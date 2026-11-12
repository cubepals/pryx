// SPDX-License-Identifier: FSL-1.1-ALv2
//! Explicit hosted acceptance of the real closed desktop worker, headful application input,
//! screenshots, noVNC/VNC and independently confined same-UID neighbours.
use futures_util::StreamExt;
use pryx_agent::{process_policy, process_tool_policy, process_tools::desktop};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    os::{
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const INPUT: &str = "PRYX_DESKTOP_APPLICATION_WORKER";
#[derive(Clone, Deserialize, Serialize)]
struct Input {
    root: PathBuf,
    bundle: PathBuf,
    address: String,
    application_url: String,
    neighbour: bool,
    peers: Vec<u32>,
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn policy(input: &Input) -> process_policy::Policy {
    process_policy::Policy {
        data: input.root.join("data"),
        memory_bytes: 2 * 1024 * 1024 * 1024,
        address_space_bytes: process_policy::DEFAULT_ADDRESS_SPACE_MB * 1024 * 1024,
        open_files: 4096,
        processes: 4096,
        cpu_seconds: 120,
        nice: 1,
        network_off: false,
        require_ipc_scope: Some(true),
        tools: Some(process_tool_policy::Policy {
            cache: input.bundle.clone(),
            downloads: input.root.join("downloads"),
            scratch: input.root.join("scratch"),
            artifact: None,
            unavailable: None,
            browser_on: true,
            desktop_on: None,
            desktop_unavailable: None,
        }),
    }
}
fn worker(input: &Input) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "desktop_application_worker", "--ignored", "--nocapture"])
        .env_clear()
        .env(INPUT, serde_json::to_string(input).unwrap());
    command
}
async fn desktop_command(client: &reqwest::Client, token: &str, name: &str, args: Value, input: Option<&str>) {
    client
        .post("http://localhost/command")
        .header("pryx-desktop-private", token)
        .body(json!({"name":name,"args":args,"input":input}).to_string())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
}
fn owner(input: &Input) {
    process_policy::scope().unwrap();
    let token = uuid::Uuid::new_v4().to_string();
    let scratch = input.root.join("scratch");
    let launch = desktop::Launch {
        scratch: scratch.clone(),
        address: input.address.clone(),
        private_token: token.clone(),
        application_url: Some(input.application_url.clone()),
    };
    let mut child = Running(
        desktop::start_at(&policy(input), launch, &input.bundle, Path::new(env!("CARGO_BIN_EXE_pryx-agent"))).unwrap(),
    );
    process_policy::enter(&policy(input)).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let client = reqwest::Client::builder().unix_socket(PathBuf::from(format!("\0{}", input.address)))
            .timeout(pryx_agent::contract::TOOL_STARTUP).build().unwrap();
        let deadline = Instant::now() + pryx_agent::contract::TOOL_STARTUP;
        loop {
            if client.get("http://localhost/vnc.html").send().await.is_ok_and(|r| r.status().is_success()) { break }
            assert!(child.0.try_wait().unwrap().is_none(), "Desktop worker exited");
            assert!(Instant::now() < deadline, "Desktop readiness deadline");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        fs::write(input.root.join("data/ready.json"), fs::read(scratch.join("desktop-ready.json")).unwrap()).unwrap();
        let actions = input.root.join("data/actions.json");
        while !actions.exists() {
            assert!(Instant::now() < deadline, "Application geometry deadline");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let points: Value = serde_json::from_slice(&fs::read(actions).unwrap()).unwrap();
        for name in ["input", "button"] {
            desktop_command(&client, &token, "xdotool", json!(["mousemove","--sync",points[name]["x"].to_string(),points[name]["y"].to_string(),"click","1"]), None).await;
            if name == "input" {
                desktop_command(&client, &token, "xdotool", json!(["type","--clearmodifiers","--delay","0","--file","-"]), Some("Native desktop typed text")).await;
            }
        }
        let shot = scratch.join("screen.png");
        desktop_command(&client, &token, "scrot", json!(["--silent",shot.to_string_lossy()]), None).await;
        assert!(fs::read(&shot).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(client.post("http://localhost/command").body("{}").send().await.unwrap().status(), reqwest::StatusCode::FORBIDDEN);
        let expires = pryx_agent::telemetry::now() + 3;
        fs::write(scratch.join("tokens"), format!("{token}: unix_socket:/tmp/pryx-desktop/vnc.sock\n")).unwrap();
        fs::write(scratch.join("desktop-access.json"), json!({"token":token,"expiresAt":expires}).to_string()).unwrap();
        let stream = tokio::net::UnixStream::connect(format!("\0{}",input.address)).await.unwrap();
        let (mut websocket, _) = tokio_tungstenite::client_async(format!("ws://localhost/websockify?token={token}"), stream).await.unwrap();
        let first = tokio::time::timeout(pryx_agent::contract::TOOL_STARTUP, websocket.next()).await.unwrap().unwrap().unwrap();
        assert!(first.into_data().starts_with(b"RFB "), "Actual VNC greeting required");
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(Ok(message)) = websocket.next().await { if message.is_close() { break } }
        }).await.expect("Expired VNC connection remained open");
        let stream = tokio::net::UnixStream::connect(format!("\0{}",input.address)).await.unwrap();
        assert!(tokio_tungstenite::client_async(format!("ws://localhost/websockify?token={token}"), stream).await.is_err());
        fs::write(input.root.join("data/result.json"), json!({"headfulApplication":true,"screenshot":true,"novncPage":true,"vncGreeting":true,"expiredConnectionClosed":true,"expiredReconnectDenied":true,"privateCommandAuthentication":true}).to_string()).unwrap();
        while !input.root.join("data/stop").exists() { tokio::time::sleep(Duration::from_millis(20)).await }
    });
}
fn neighbour(input: &Input) {
    process_policy::enter(&policy(input)).unwrap();
    assert!(fs::read(input.root.parent().unwrap().join("owner/scratch/Xauthority")).is_err());
    let address = SocketAddr::from_abstract_name(&input.address).unwrap();
    let error = UnixStream::connect_addr(&address).unwrap_err();
    let mut aliases = Vec::new();
    for pid in &input.peers {
        for socket in ["vnc.sock", "web.sock"] {
            let path = format!("/proc/{pid}/root/tmp/pryx-desktop/{socket}");
            let error = UnixStream::connect(&path).unwrap_err();
            aliases.push(json!({"path":path,"error":error.to_string()}));
        }
    }
    fs::write(input.root.join("data/neighbour.json"), json!({"uid":rustix::process::geteuid().as_raw(),"abstractDenied":error.to_string(),"privateAliasesDenied":aliases,"ownerCookieReadDenied":true}).to_string()).unwrap();
}
#[test]
#[ignore = "closed subprocess entry for hosted desktop acceptance"]
fn desktop_application_worker() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let input: Input = serde_json::from_str(&std::env::var(INPUT).unwrap()).unwrap();
    if input.neighbour { neighbour(&input) } else { owner(&input) }
}
fn descendants(pid: u32) -> Vec<u32> {
    let processes =
        procfs::process::all_processes().unwrap().flatten().filter_map(|p| p.stat().ok()).collect::<Vec<_>>();
    let mut result = vec![pid];
    loop {
        let before = result.len();
        for p in &processes {
            if result.contains(&(p.ppid as u32)) && !result.contains(&(p.pid as u32)) {
                result.push(p.pid as u32)
            }
        }
        if before == result.len() {
            return result;
        }
    }
}
fn diagnostics(root: &Path) -> String {
    [
        root.join("owner.stderr"),
        root.join("owner/scratch/desktop.stderr"),
        root.join("owner/scratch/desktop-phase.json"),
    ]
    .iter()
    .map(|p| fs::read_to_string(p).unwrap_or_default())
    .collect::<Vec<_>>()
    .join("\n")
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit complete staged tools, namespaces and fully enforced process policy"]
async fn real_headful_input_vnc_and_neighbour_privacy() {
    assert!(!rustix::process::geteuid().is_root());
    assert!(pryx_agent::process_capabilities::probe().filesystem);
    assert!(pryx_agent::process_capabilities::private_ipc_probe().supported());
    let observations = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = observations.clone();
    let app = axum::Router::new()
        .route(
            "/",
            axum::routing::get(|| async { axum::response::Html(include_str!("fixtures/desktop-application.html")) }),
        )
        .route(
            "/observe",
            axum::routing::post(move |axum::Json(value): axum::Json<Value>| async move {
                captured.lock().unwrap().push(value);
            }),
        );
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let serving = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    for role in ["owner", "neighbour"] {
        for path in ["data", "scratch", "downloads"] {
            fs::create_dir_all(root.path().join(role).join(path)).unwrap()
        }
    }
    let mut input = Input {
        root: root.path().join("owner"),
        bundle: PathBuf::from(std::env::var_os("PRYX_DESKTOP_APPLICATION_BUNDLE").unwrap()).canonicalize().unwrap(),
        address: format!("pryx-desktop-{}", uuid::Uuid::new_v4()),
        application_url: url,
        neighbour: false,
        peers: Vec::new(),
    };
    let mut owner = Running(
        worker(&input)
            .stdout(Stdio::null())
            .stderr(fs::File::create(root.path().join("owner.stderr")).unwrap())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + pryx_agent::contract::TOOL_STARTUP + Duration::from_secs(10);
    loop {
        let ready = observations.lock().unwrap().iter().find(|v| v["kind"] == "ready").cloned();
        if input.root.join("data/ready.json").exists()
            && let Some(ready) = ready
        {
            fs::write(input.root.join("data/actions.json"), ready.to_string()).unwrap();
            break;
        }
        assert!(owner.0.try_wait().unwrap().is_none() && Instant::now() < deadline, "{}", diagnostics(root.path()));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    while !input.root.join("data/result.json").exists() {
        assert!(owner.0.try_wait().unwrap().is_none() && Instant::now() < deadline, "{}", diagnostics(root.path()));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let observed = observations.lock().unwrap().clone();
    assert!(observed.iter().any(|v| v["kind"] == "click"));
    assert!(observed.iter().any(|v| v["kind"] == "type" && v["value"] == "Native desktop typed text"));
    input.peers = descendants(owner.0.id());
    assert!(input.peers.len() > 5);
    let processes = input
        .peers
        .iter()
        .filter_map(|pid| Some((*pid, fs::read_to_string(format!("/proc/{pid}/cmdline")).ok()?)))
        .collect::<Vec<_>>();
    let broker = processes.iter().find(|(_, args)| args.contains("desktop/adapter/start.mjs")).unwrap().0;
    let renderer = processes.iter().find(|(_, args)| args.contains("--type=renderer")).unwrap().0;
    let status = |pid: u32, name: &str| {
        fs::read_to_string(format!("/proc/{pid}/status"))
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}:")))
            .unwrap()
            .trim()
            .to_string()
    };
    assert_ne!(status(renderer, "Uid").split_whitespace().next().unwrap(), "0");
    assert_eq!(status(renderer, "NoNewPrivs"), "1");
    assert!(status(renderer, "NSpid").split_whitespace().count() > status(broker, "NSpid").split_whitespace().count());
    assert!(
        status(renderer, "Seccomp_filters").parse::<u32>().unwrap()
            > status(broker, "Seccomp_filters").parse::<u32>().unwrap()
    );
    let live = input
        .peers
        .iter()
        .filter(|pid| UnixStream::connect(format!("/proc/{pid}/root/tmp/pryx-desktop/web.sock")).is_ok())
        .count();
    assert!(live > 0, "Actual live private endpoint required");
    let result: Value = serde_json::from_slice(&fs::read(input.root.join("data/result.json")).unwrap()).unwrap();
    input.neighbour = true;
    input.root = root.path().join("neighbour");
    let mut neighbour = Running(worker(&input).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = neighbour.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let mut diagnostic = String::new();
    neighbour.0.stderr.take().unwrap().read_to_string(&mut diagnostic).unwrap();
    assert!(status.success(), "{diagnostic}");
    let neighbour: Value = serde_json::from_slice(&fs::read(input.root.join("data/neighbour.json")).unwrap()).unwrap();
    let report = json!({"scope":"Staged actual native worker; installed SDK/edge acceptance still required","uid":rustix::process::geteuid().as_raw(),"owner":result,"observations":observed,"neighbour":neighbour,"unconfinedLiveAliases":live,"rendererOwnPidNamespace":true,"rendererOwnSeccomp":true,"rendererNoNewPrivileges":true});
    if let Some(path) = std::env::var_os("PRYX_DESKTOP_APPLICATION_REPORT") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap()
    }
    fs::write(root.path().join("owner/data/stop"), b"stop").unwrap();
    println!("{report}");
    serving.abort();
}
