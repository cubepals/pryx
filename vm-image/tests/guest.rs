// SPDX-License-Identifier: FSL-1.1-ALv2
//! Boot the actual agent with real OCI layers and KVM, using the maintained monitor SDK.
//! This input-backed acceptance proves guest init and durable shutdown, not production
//! monitor confinement, network policy or the still-unadvertised VM runtime.
use axum::http::{Method, StatusCode};
use base64::{Engine, engine::general_purpose::STANDARD};
use pryx_agent::{
    auth::{Claims, Scope, SigningKey, TOKEN_LIFETIME_SECONDS},
    contract::{AGENT_PORT, IDEMPOTENCY_HEADER},
    process::{Command as GuestCommand, Start},
    vm_boot::{self, Boot, Network},
    vm_monitor::{Plan, Role},
    vm_observation::{self, Proof},
    vm_shutdown,
};
use pryx_vm_image::image::{Images, PreparedImage};
use pryxd::runtime::vm::filesystems::FilesystemTools;
use serde_json::{Value, json};
use std::{
    fs,
    net::Ipv4Addr,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

fn input(name: &str) -> PathBuf {
    std::env::var_os(name).unwrap_or_else(|| panic!("Stage {name}")).into()
}

async fn output(command: &mut Command) -> Vec<u8> {
    let result = command.kill_on_drop(true).output().await.unwrap();
    assert!(result.status.success(), "Fixture failed: {}", String::from_utf8_lossy(&result.stderr));
    result.stdout
}

async fn request(client: &Path, message: Value) -> Result<(StatusCode, Vec<u8>), String> {
    pryxd::runtime::vm::transport::request(
        client,
        &message,
        Duration::from_secs(35),
        pryx_agent::contract::CAPTURE_BYTES * 4,
    )
    .await
    .map_err(|error| error.to_string())
}

struct Guest {
    channel: PathBuf,
    client: PathBuf,
    token: String,
}

impl Guest {
    async fn call(&self, method: Method, path: &str, body: Value, binary: Option<&[u8]>) -> (StatusCode, Vec<u8>) {
        let mut message = json!({"socket": self.channel, "port": vm_boot::service_port(AGENT_PORT),
            "method": method.as_str(), "path": path, "token": self.token, "body": body});
        message["headers"] = json!({(IDEMPOTENCY_HEADER): [format!("once-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())]});
        if let Some(binary) = binary {
            message["binaryBody"] = STANDARD.encode(binary).into();
        }
        request(&self.client, message).await.unwrap()
    }

    async fn ready(&self) {
        let until = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            // Only this safe health GET retries; exec, file writes and stop are sent once.
            let result = request(
                &self.client,
                json!({"socket": self.channel, "port": vm_boot::service_port(AGENT_PORT),
                "method": Method::GET.as_str(), "path": "/_pryx/health", "token": self.token}),
            )
            .await;
            if result.is_ok_and(|(status, _)| status == StatusCode::OK) {
                break;
            }
            assert!(tokio::time::Instant::now() < until, "Actual VM agent did not become ready");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn command(&self, arguments: Vec<String>) {
        let label = format!("{arguments:?}");
        let start = Start {
            command: GuestCommand::Argv(arguments),
            env: Default::default(),
            cwd: Some("/data".into()),
            stdin: false,
            pty: false,
            cols: None,
            rows: None,
            timeout: Some(10),
        };
        let (status, body) =
            self.call(Method::POST, "/_pryx/processes", serde_json::to_value(start).unwrap(), None).await;
        assert_eq!(status, StatusCode::CREATED);
        let view: Value = serde_json::from_slice(&body).unwrap();
        let path = format!("/_pryx/processes/{}/wait", view["id"].as_str().unwrap());
        let (status, body) = self.call(Method::GET, &path, Value::Null, None).await;
        assert_eq!(status, StatusCode::OK);
        let result: Value = serde_json::from_slice(&body).unwrap();
        if result["code"] != 0 {
            let path = format!("/_pryx/processes/{}/output", view["id"].as_str().unwrap());
            let (status, bytes) = self.call(Method::GET, &path, Value::Null, None).await;
            panic!("Guest command {label} failed: {result}, output {status}: {}", String::from_utf8_lossy(&bytes));
        }
    }
}

fn verify_shutdown(log: &str, key: &SigningKey, encoded: &str, token: &str, nonce: &str, sandbox: &str, kernel: &str) {
    for secret in [encoded, token] {
        assert!(!log.contains(secret), "Bootstrap credential reached serial output");
    }
    let proofs = log
        .lines()
        .filter_map(|line| line.strip_prefix(vm_observation::PREFIX))
        .filter_map(|line| serde_json::from_str::<Proof>(line).ok())
        .collect::<Vec<_>>();
    assert_eq!(proofs.len(), 1, "Expected exactly one clean shutdown proof; see retained serial log");
    assert!(vm_observation::verify(&proofs[0], &key.public_key(), nonce, sandbox, 1));
    assert_eq!(proofs[0].observation.kernel, kernel);
}

struct NetworkFixture {
    proxy: tokio::process::Child,
    http: tokio::task::JoinHandle<()>,
    configuration: Value,
}

impl Drop for NetworkFixture {
    fn drop(&mut self) {
        self.http.abort();
    }
}

impl NetworkFixture {
    async fn start(metadata: &Value, channel: &Path, directory: &Path, round: usize) -> Self {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = metadata["networkHello"].as_str().unwrap().to_owned();
        let router = axum::Router::new().route(
            metadata["networkHelloPath"].as_str().unwrap(),
            axum::routing::get(move || {
                let body = body.clone();
                async move { body }
            }),
        );
        let http = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let mut configuration = metadata["network"].clone();
        configuration["mode"] = "vsock".into();
        configuration["url"] = format!("http://{}:{port}", configuration["host"].as_str().unwrap()).into();
        let path = directory.join(format!("network{round}.json"));
        fs::write(&path, serde_json::to_vec(&configuration).unwrap()).unwrap();
        let transport = format!("{}_{port}", channel.display(), port = metadata["networkPort"].as_u64().unwrap());
        let mut proxy = Command::new(input("PRYX_VM_NETWORK_PROXY"))
            .arg("--config")
            .arg(path)
            .arg("--guest-socket")
            .arg(&transport)
            .arg("--control-socket")
            .arg(directory.join(format!("control{round}.sock")))
            .stdout(Stdio::null())
            .stderr(fs::File::create(directory.join(format!("network{round}.log"))).unwrap())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        while !Path::new(&transport).exists() {
            assert!(proxy.try_wait().unwrap().is_none(), "Maintained host network fixture exited");
            assert!(tokio::time::Instant::now() < until, "Host packet transport did not appear");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Self { proxy, http, configuration }
    }
}

async fn exercise_guest(
    guest: &Guest,
    directory: &Path,
    round: usize,
    uid: u32,
    network_fixture: Option<&NetworkFixture>,
    bootstrap_metadata: &Value,
) -> String {
    let bytes = b"Real VM file round trip with persistent ext4\n";
    if round == 0 {
        let (status, _) =
            guest.call(Method::PUT, "/_pryx/files/content?path=roundtrip", Value::Null, Some(bytes)).await;
        assert!(status.is_success());
    }
    let (status, body) = guest.call(Method::GET, "/_pryx/files/content?path=roundtrip", Value::Null, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, bytes);
    if let Some(fixture) = network_fixture {
        let bytes = serde_json::to_vec(&fixture.configuration).unwrap();
        let (status, _) =
            guest.call(Method::PUT, "/_pryx/files/content?path=network.json", Value::Null, Some(&bytes)).await;
        assert!(status.is_success());
        let probe = Path::new(vm_boot::BOOTSTRAP_ROOT).join(bootstrap_metadata["networkProbe"].as_str().unwrap());
        let restricted = bootstrap_metadata["metadata"]["vmGuestPolicyVersion"].as_u64().unwrap_or_default()
            >= u64::from(pryx_agent::vm_policy::VERSION);
        guest
            .command(vec![
                probe.display().to_string(),
                if restricted { "--verify-restricted" } else { "--verify" }.into(),
                "--config".into(),
                "/data/network.json".into(),
            ])
            .await;
    } else {
        guest.command(vec!["/bin/sh".into(), "-c".into(), "test \"$(ls /sys/class/net)\" = lo".into()]).await;
    }
    let host_secret = directory.join("host-private-file");
    fs::write(&host_secret, "Must remain outside the guest").unwrap();
    guest.command(vec!["/bin/sh".into(), "-c".into(),
        "test ! -e \"$1\" && ! kill -TERM \"$2\" 2>/dev/null && test \"$(id -u)\" = \"$3\" && uname -r > /data/kernel".into(),
        "isolation-check".into(), host_secret.display().to_string(), std::process::id().to_string(), uid.to_string()]).await;
    let (status, kernel) = guest.call(Method::GET, "/_pryx/files/content?path=kernel", Value::Null, None).await;
    assert_eq!(status, StatusCode::OK);
    let kernel = String::from_utf8(kernel).unwrap().trim().to_owned();
    assert_ne!(kernel, fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim());
    kernel
}

async fn exercise_forwarding(guest: &Guest, metadata: &Value, bootstrap: &Value) {
    let echo_port = metadata["forwardPort"].as_u64().unwrap() as u16;
    let ready = format!("forward-{}.ready", uuid::Uuid::new_v4().simple());
    let probe = Path::new(vm_boot::BOOTSTRAP_ROOT).join(bootstrap["networkProbe"].as_str().unwrap());
    let start = Start {
        command: GuestCommand::Argv(vec![
            probe.display().to_string(),
            "--echo".into(),
            echo_port.to_string(),
            "--ready-file".into(),
            format!("/data/{ready}"),
        ]),
        env: Default::default(),
        cwd: Some("/data".into()),
        stdin: false,
        pty: false,
        cols: None,
        rows: None,
        timeout: None,
    };
    assert_eq!(
        guest.call(Method::POST, "/_pryx/processes", serde_json::to_value(start).unwrap(), None).await.0,
        StatusCode::CREATED
    );
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        // Only the safe file GET retries. The process and payload are submitted once.
        if guest.call(Method::GET, &format!("/_pryx/files/content?path={ready}"), Value::Null, None).await.0
            == StatusCode::OK
        {
            break;
        }
        assert!(tokio::time::Instant::now() < until, "Declared-port service did not become ready");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let bindings = [AGENT_PORT, echo_port]
        .map(|port| pryxd::runtime::vm::forward::Binding::service(Ipv4Addr::LOCALHOST.into(), 0, port).unwrap());
    let mut proxy = pryxd::runtime::vm::forward::Forwarder::start(
        &guest.client,
        &guest.channel,
        &bindings,
        8,
        tokio::time::Instant::now() + Duration::from_secs(5),
    )
    .await
    .unwrap();
    let addresses = proxy.addresses().to_vec();
    assert_eq!(addresses.len(), 2);
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let path = format!("http://{}/_pryx/files/content?path=forwarded", addresses[0]);
    let count = metadata["networkBulkBytes"].as_u64().unwrap() as usize;
    let payload = (0..=u8::MAX).cycle().take(count).collect::<Vec<_>>();
    let response = client
        .put(&path)
        .bearer_auth(&guest.token)
        .header(IDEMPOTENCY_HEADER, uuid::Uuid::new_v4().to_string())
        .body(payload.clone())
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let response = client.get(&path).bearer_auth(&guest.token).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await.unwrap().as_ref(), payload);
    let mut stream = tokio::net::TcpStream::connect(addresses[1]).await.unwrap();
    let opaque = &payload[..64 * 1024];
    stream.write_all(opaque).await.unwrap();
    stream.shutdown().await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.take((count + 1) as u64).read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    let mut expected = metadata["forwardPrefix"].as_str().unwrap().as_bytes().to_vec();
    expected.extend_from_slice(opaque);
    expected.extend_from_slice(metadata["forwardSuffix"].as_str().unwrap().as_bytes());
    assert_eq!(response.len(), expected.len(), "TCP reply after client EOF changed length");
    assert!(response == expected, "TCP bytes after client EOF must be retained");
    proxy.stop().await.unwrap();
}

async fn boot(
    tools: &FilesystemTools,
    image: &PreparedImage,
    directory: &Path,
    writes: &Path,
    round: usize,
    networked: bool,
) -> Value {
    let configurator = input("PRYX_VM_CONFIGURATOR");
    let metadata: Value = serde_json::from_slice(&output(Command::new(&configurator).arg("--metadata")).await).unwrap();
    let bootstrap = input("PRYX_VM_BOOTSTRAP");
    let bootstrap_metadata: Value =
        serde_json::from_slice(&fs::read(bootstrap.join("bootstrap.json")).unwrap()).unwrap();
    let channel = directory.join(format!("v{round}.sock"));
    let api = directory.join(format!("a{round}.sock"));
    let mut arguments = vec![
        "--kernel".into(),
        input("PRYX_VM_KERNEL").display().to_string(),
        "--root".into(),
        bootstrap.join("bootstrap.erofs").display().to_string(),
        "--writes".into(),
        writes.display().to_string(),
        "--socket".into(),
        channel.display().to_string(),
        "--init-mode".into(),
        bootstrap_metadata["metadata"]["vmInitMode"].as_str().unwrap().into(),
    ];
    let ids = image.layers.iter().enumerate().map(|(i, _)| format!("layer{i}")).collect::<Vec<_>>();
    for (id, path) in ids.iter().zip(&image.layers) {
        arguments.extend(["--layer".into(), format!("{id}={}", path.display())]);
    }
    let config = output(Command::new(&configurator).args(&arguments)).await;
    let identities: Value =
        serde_json::from_slice(&output(Command::new(&configurator).args(&arguments).arg("--identities")).await)
            .unwrap();
    let config_path = directory.join(format!("config{round}.json"));
    fs::write(&config_path, config).unwrap();
    let (key, encoded) = SigningKey::generate().unwrap();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let sandbox = "vm-agent-acceptance";
    let token = key
        .sign(&Claims {
            sandbox: sandbox.into(),
            epoch: 1,
            issued_at: now,
            expires_at: now + TOKEN_LIFETIME_SECONDS,
            scope: Scope::Sandbox,
            max_ports: 4,
            max_exec_seconds: Some(15),
            file: None,
        })
        .unwrap();
    let mut environment = image
        .configuration
        .config
        .as_ref()
        .and_then(|c| c.env.as_ref())
        .into_iter()
        .flatten()
        .filter_map(|e| e.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())))
        .collect::<Vec<_>>();
    let secret_env = "PRYX_VM_ACCEPTANCE_KEY";
    environment.push((secret_env.into(), encoded.clone()));
    let uid = rustix::process::getuid().as_raw() + 1;
    let network_fixture =
        if networked { Some(NetworkFixture::start(&metadata, &channel, directory, round).await) } else { None };
    let network = network_fixture.as_ref().map(|fixture| Network {
        interface: "tap0".into(),
        mac: fixture.configuration["guestMac"].as_str().unwrap().into(),
        mtu: fixture.configuration["mtu"].as_u64().unwrap() as u32,
        vsock_port: Some(metadata["networkPort"].as_u64().unwrap() as u32),
        policy: None,
    });
    let boot = Boot {
        version: vm_boot::VERSION,
        nonce: nonce.clone(),
        cid: metadata["guestCid"].as_u64().unwrap() as u32,
        layers: ids.iter().map(|id| identities[id].as_str().unwrap().into()).collect(),
        writes: identities[metadata["writeDrive"].as_str().unwrap()].as_str().unwrap().into(),
        hostname: "vm-agent-acceptance".into(),
        uid,
        gid: uid,
        data: "/data".into(),
        read_only_root: false,
        temporary_bytes: 32 * 1024 * 1024,
        pids_limit: 256,
        arguments: vec![
            "--sandbox".into(),
            sandbox.into(),
            "--epoch".into(),
            "1".into(),
            "--data".into(),
            "/data".into(),
            "--domain".into(),
            "vm.pryx.invalid".into(),
            "--port".into(),
            AGENT_PORT.to_string(),
            "--vm-port-base".into(),
            vm_boot::SERVICE_PORT_BASE.to_string(),
            "--vm-ports".into(),
            metadata["forwardPort"].as_u64().unwrap().to_string(),
            "--secret-env".into(),
            secret_env.into(),
            "--keep-alive".into(),
        ],
        environment,
        network_arguments: None,
        network,
    };
    let serial_path = directory.join(format!("boot{round}.serial.log"));
    let locks = directory.parent().unwrap().join("monitor-locks");
    match fs::DirBuilder::new().mode(0o700).create(&locks) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => panic!("Create monitor lease directory: {error}"),
    }
    let lease_path = locks.join(format!("{}.lock", directory.file_name().unwrap().to_str().unwrap()));
    let mut read = vec![input("PRYX_VM_KERNEL"), bootstrap.join("bootstrap.erofs"), config_path.clone()];
    read.extend(image.layers.iter().cloned());
    let plan = Plan {
        program: input("PRYX_VM_MONITOR"),
        arguments: vec![
            "--api-sock".into(),
            api.display().to_string(),
            "--config-file".into(),
            config_path.display().to_string(),
        ],
        directory: directory.into(),
        lock: lease_path.clone(),
        parent: std::process::id(),
        read,
        write: vec![writes.into()],
        role: Role::Ordinary,
    };
    let mut monitor = pryxd::runtime::vm::machine::Machine::launch(
        &input("PRYX_VM_HOST_LAUNCHER"),
        &plan,
        &channel,
        &serial_path,
        &boot,
        tokio::time::Instant::now() + Duration::from_secs(15),
        pryx_agent::contract::CAPTURE_BYTES * 4,
    )
    .await
    .unwrap();
    let guest = Guest { channel, client: input("PRYX_VM_AGENT_CLIENT"), token: token.clone() };
    guest.ready().await;
    let lease = fs::OpenOptions::new().read(true).write(true).open(&lease_path).unwrap();
    assert_eq!(
        rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap_err(),
        rustix::io::Errno::WOULDBLOCK
    );
    monitor.bootstrapped(tokio::time::Instant::now() + Duration::from_secs(1)).await.unwrap();
    let kernel = exercise_guest(&guest, directory, round, uid, network_fixture.as_ref(), &bootstrap_metadata).await;
    exercise_forwarding(&guest, &metadata, &bootstrap_metadata).await;
    let other_nonce = uuid::Uuid::new_v4().simple().to_string();
    assert_ne!(nonce, other_nonce);
    let replay = vm_shutdown::request(&key, &other_nonce, sandbox, 1);
    assert!(
        request(
            &guest.client,
            json!({"socket": guest.channel,
        "port": vm_shutdown::PORT, "shutdown": replay})
        )
        .await
        .is_err()
    );
    assert_eq!(guest.call(Method::GET, "/_pryx/health", Value::Null, None).await.0, StatusCode::OK);
    let (status, _) = request(&guest.client, json!({"socket": api, "stop": true})).await.unwrap();
    assert!(status.is_success());
    // This published kernel has no keyboard driver. Preserve the real monitor
    // action assertion, then ask trusted PID 1 explicitly to flush and power off.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let shutdown = vm_shutdown::request(&key, &nonce, sandbox, 1);
    let (status, _) = tokio::time::timeout_at(
        deadline,
        request(
            &guest.client,
            json!({"socket": guest.channel,
            "port": vm_shutdown::PORT, "shutdown": shutdown}),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(status, StatusCode::ACCEPTED);
    let status = tokio::time::timeout_at(deadline, monitor.wait()).await.expect("VM did not power off").unwrap();
    assert!(status.success());
    tokio::time::timeout_at(deadline, monitor.finish_serial()).await.unwrap().unwrap();
    assert!(!monitor.serial_truncated(), "All serial bytes must remain available to the secrecy assertions");
    rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    drop(lease);
    if let Some(mut fixture) = network_fixture {
        fixture.proxy.kill().await.unwrap();
    }
    let log = fs::read_to_string(&serial_path).unwrap();
    verify_shutdown(&log, &key, &encoded, &token, &nonce, sandbox, &kernel);
    let mut check = tools.builder("e2fsck");
    let check = check.arg("-f").arg("-n").arg(writes).output().unwrap();
    assert!(check.status.success(), "Stopped VM disk failed read-only integrity check");
    json!({"round": round, "kernel": kernel, "guestUid": uid, "signedCleanShutdown": true,
        "streamingBinaryAndDeclaredTcp": true, "monitorConfined": true, "monitorLeaseHeldUntilExit": true,
        "authenticatedInitShutdown": true, "shutdownReplayRefused": true,
        "persistentFile": true, "hostFileDenied": true, "hostSignalDenied": true,
        "networked": networked, "dhcpDnsHttpAndManagementDenial": networked})
}

#[tokio::test]
#[ignore = "requires real KVM and staged verified inputs; the optional VM workflow runs it"]
async fn actual_agent_boots_both_oci_images_and_reboots_synced_disks() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let component = input("PRYX_VM_FILESYSTEM_COMPONENT");
    let manifest = std::env::var("PRYX_VM_FILESYSTEM_MANIFEST").unwrap();
    let tools = FilesystemTools::verified(&component, &manifest).unwrap();
    let work = input("PRYX_VM_GUEST_WORK");
    fs::create_dir(&work).unwrap();
    let images = Images::new(work.join("images"), tools.clone()).unwrap();
    let inputs: Vec<Value> = serde_json::from_str(include_str!("../../templates/vm-tools/image-inputs.json")).unwrap();
    let mut report = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        let image = images.prepare(input["reference"].as_str().unwrap(), Duration::from_secs(120)).await.unwrap();
        let directory = work.join(format!("image{index}"));
        fs::DirBuilder::new().mode(0o700).create(&directory).unwrap();
        let writes = directory.join("writes.ext4");
        fs::File::create_new(&writes).unwrap().set_len(512 * 1024 * 1024).unwrap();
        assert!(tools.builder("mke2fs").args(["-q", "-F", "-t", "ext4"]).arg(&writes).status().unwrap().success());
        let mut rounds = Vec::new();
        for round in 0..2 {
            rounds.push(boot(&tools, &image, &directory, &writes, round, false).await);
        }
        rounds.push(boot(&tools, &image, &directory, &writes, 2, true).await);
        report.push(json!({"image": input["reference"], "manifestDigest": image.digest, "boots": rounds}));
    }
    fs::write(
        work.join("guest-acceptance.json"),
        serde_json::to_vec_pretty(&json!({
        "scope": "Actual guest init and agent only; production VM backend and confinement still incomplete",
        "status": "success", "images": report}))
        .unwrap(),
    )
    .unwrap();
}
