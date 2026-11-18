// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host artifact acquisition before a filtered process starts: cold caches, immutable consumers,
//! cancellation and generation ownership. Actual browser/desktop/kernel payloads stay in their
//! separate native-tool acceptance; this fixture exercises their common acquisition boundary.
use super::{
    Entry, ProcessRuntime, Served,
    network::{Network, fixture_spec},
};
use crate::bpf::host::{self, Host};
use pryx_agent::process_tool_policy::Artifact;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

struct Fixture {
    runtime: ProcessRuntime,
    entry: Entry,
    artifact: Artifact,
    calls: Arc<AtomicUsize>,
    entered: Arc<tokio::sync::Notify>,
    gate: Arc<tokio::sync::Semaphore>,
    server: tokio::task::JoinHandle<()>,
    root: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.gate.close();
        self.server.abort();
        let _ = crate::tree::unseal(self.root.path());
    }
}

impl Fixture {
    async fn new(held: bool) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let content = b"A pinned native artifact fixture\n";
        let inventory = serde_json::to_vec(&serde_json::json!({"files":[
            {"path":"tool.txt","sha256":digest(content)}
        ]}))
        .unwrap();
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (path, body) in [("tool.txt", content.as_slice()), ("manifest.json", inventory.as_slice())] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, path, body).unwrap();
        }
        let bytes = archive.into_inner().unwrap().finish().unwrap();
        let artifact_sha = digest(&bytes);
        let length = bytes.len() as u64;
        let calls = Arc::new(AtomicUsize::new(0));
        let entered = Arc::new(tokio::sync::Notify::new());
        let gate = Arc::new(tokio::sync::Semaphore::new(usize::from(!held)));
        let (observed, started, ready) = (calls.clone(), entered.clone(), gate.clone());
        let router = axum::Router::new().route(
            "/bundle",
            axum::routing::get(move || {
                let (bytes, calls, entered, gate) = (bytes.clone(), observed.clone(), started.clone(), ready.clone());
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    entered.notify_one();
                    let permit = gate.acquire().await;
                    if permit.is_ok() {
                        (axum::http::StatusCode::OK, bytes)
                    } else {
                        (axum::http::StatusCode::SERVICE_UNAVAILABLE, Vec::new())
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let url = format!("http://{}/bundle", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let runtime = ProcessRuntime::new(&state, crate::config::ProcessConfig::default()).unwrap();
        let mut spec = fixture_spec(&state.join("data"));
        spec.agent = Some(crate::runtime::AgentBoot {
            public_origin: None,
            binary: root.path().join("not-executed"),
            sandbox: "native-fixture".into(),
            epoch: 1,
            secret_env: "FIXTURE_ONLY".into(),
            domain: "pryx.test".into(),
            agent_port: pryx_agent::contract::AGENT_PORT,
            spare_ports: Vec::new(),
            keep_alive: true,
        });
        runtime.create_entry(&spec).unwrap();
        let entry = runtime.entry(&spec.name).unwrap();
        Self {
            runtime,
            entry,
            artifact: Artifact {
                url,
                sha256: artifact_sha,
                manifest_sha256: digest(&inventory),
                bytes: length,
                unpacked_bytes: (content.len() + inventory.len()) as u64,
                entries: 2,
            },
            calls,
            entered,
            gate,
            server,
            root,
        }
    }

    fn base(&self) -> PathBuf {
        self.runtime.state.root.parent().unwrap().join("process-tools")
    }

    fn pending(&self) -> tokio::task::JoinHandle<Result<PathBuf, super::RuntimeError>> {
        let (runtime, entry, artifact) = (self.runtime.clone(), self.entry.clone(), self.artifact.clone());
        tokio::spawn(async move { runtime.acquire_native_artifact(&entry, &artifact).await })
    }

    async fn started(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.entered.notified()).await.unwrap();
    }

    fn no_tenant_effects(&self) {
        assert!(self.runtime.state.owners.lock().unwrap().is_empty());
        assert!(self.runtime.state.children.lock().unwrap().is_empty());
        assert!(
            fs::read_dir(self.base())
                .unwrap()
                .all(|entry| { entry.unwrap().file_name().to_string_lossy().parse::<u32>().is_err() })
        );
    }
}

#[tokio::test]
async fn cold_host_acquisition_is_verified_reusable_and_independent_of_tenant_paths() {
    let fixture = Fixture::new(false).await;
    let cache = fixture.runtime.acquire_native_artifact(&fixture.entry, &fixture.artifact).await.unwrap();
    assert_eq!(cache, fixture.base().join("artifacts"));
    assert_eq!(
        fs::read(cache.join(&fixture.artifact.sha256).join("tool.txt")).unwrap(),
        b"A pinned native artifact fixture\n"
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.runtime.acquire_native_artifact(&fixture.entry, &fixture.artifact).await.unwrap(), cache);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fs::metadata(&cache).unwrap().permissions().mode() & 0o7777, 0o711);
    assert_eq!(fs::metadata(fixture.base().join("acquisition")).unwrap().permissions().mode() & 0o7777, 0o700);
    fixture.no_tenant_effects();
}

#[tokio::test]
async fn unsafe_host_cache_paths_are_refused_without_touching_the_target_or_fetching() {
    use std::os::unix::fs::symlink;
    for name in ["process-tools", "artifacts", "acquisition", "writable", "state-ancestor"] {
        let fixture = Fixture::new(false).await;
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("marker"), b"untouched").unwrap();
        if name == "state-ancestor" {
            fs::set_permissions(fixture.runtime.state.root.parent().unwrap(), fs::Permissions::from_mode(0o777))
                .unwrap();
        } else if name == "process-tools" {
            symlink(outside.path(), fixture.base()).unwrap();
        } else {
            fs::create_dir(fixture.base()).unwrap();
            if name == "writable" {
                fs::create_dir(fixture.base().join("artifacts")).unwrap();
                fs::set_permissions(fixture.base().join("artifacts"), fs::Permissions::from_mode(0o777)).unwrap();
            } else {
                symlink(outside.path(), fixture.base().join(name)).unwrap();
            }
        }
        assert!(fixture.runtime.acquire_native_artifact(&fixture.entry, &fixture.artifact).await.is_err(), "{name}");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
        assert_eq!(fs::read(outside.path().join("marker")).unwrap(), b"untouched");
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
    }
}

#[tokio::test]
async fn poisoned_host_cache_is_rejected_without_replacing_it_or_fetching_again() {
    let fixture = Fixture::new(false).await;
    let cache = fixture.runtime.acquire_native_artifact(&fixture.entry, &fixture.artifact).await.unwrap();
    let root = cache.join(&fixture.artifact.sha256);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(root.join("injected.py"), b"untrusted extra file").unwrap();
    assert!(fixture.runtime.acquire_native_artifact(&fixture.entry, &fixture.artifact).await.is_err());
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read(root.join("injected.py")).unwrap(), b"untrusted extra file");
    fixture.no_tenant_effects();
}

#[tokio::test]
async fn canceled_host_acquisition_leaves_no_partial_cache_tenant_lease_or_child() {
    let fixture = Fixture::new(true).await;
    let pending = fixture.pending();
    fixture.started().await;
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    assert_eq!(fs::read_dir(fixture.base().join("artifacts")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(fixture.base().join("acquisition")).unwrap().count(), 0);
    fixture.no_tenant_effects();
}

#[tokio::test]
async fn completed_host_acquisition_cannot_continue_after_generation_replacement() {
    let fixture = Fixture::new(true).await;
    let pending = fixture.pending();
    fixture.started().await;
    let mut replacement = fixture.entry.clone();
    replacement.id = uuid::Uuid::new_v4().to_string();
    fixture.runtime.save(replacement).unwrap();
    fixture.gate.add_permits(1);
    assert!(matches!(pending.await.unwrap(), Err(super::RuntimeError::Conflict(_))));
    fixture.no_tenant_effects();
}

#[tokio::test]
#[ignore = "requires a dedicated UID reservation"]
async fn completed_host_acquisition_cannot_continue_after_owner_retirement() {
    if !rustix::process::geteuid().is_root() {
        eprintln!("root proves dedicated-UID reservation retirement during acquisition");
        return;
    }
    let fixture = Fixture::new(true).await;
    fixture.runtime.owner("native-fixture").unwrap();
    let pending = fixture.pending();
    fixture.started().await;
    fixture.runtime.state.owners.lock().unwrap().remove("native-fixture");
    fixture.gate.add_permits(1);
    assert!(matches!(pending.await.unwrap(), Err(super::RuntimeError::Conflict(_))));
    fixture.no_tenant_effects();
}

const CONSUMER: &str = "PRYX_HOST_CACHE_CONSUMER";

#[derive(serde::Serialize, serde::Deserialize)]
struct Consumer {
    cache: PathBuf,
    downloads: PathBuf,
    artifact: Artifact,
    succeeds: bool,
}

#[test]
#[ignore = "called only by the controlled filtered dedicated-UID fixture"]
fn consume_host_cache() {
    if std::env::var_os(CONSUMER).is_none() {
        return;
    }
    let input: Consumer = serde_json::from_reader(std::io::stdin().lock().take(64 * 1024)).unwrap();
    let url = reqwest::Url::parse(&input.artifact.url).unwrap();
    let denied = std::net::TcpStream::connect((url.host_str().unwrap(), url.port().unwrap())).unwrap_err();
    assert_eq!(denied.kind(), std::io::ErrorKind::PermissionDenied, "artifact origin stays filtered");
    let _ = rustls::crypto::ring::default_provider().install_default();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let result =
        runtime.block_on(pryx_agent::process_tools::acquire_artifact(input.cache, input.downloads, input.artifact));
    assert_eq!(result.is_ok(), input.succeeds);
}

struct Policed(Network);
impl Drop for Policed {
    fn drop(&mut self) {
        let _ = self.0.release("route-guard");
    }
}

fn consumer(network: &Network, binary: &Path, owner: (u32, u32), input: Consumer) {
    let mut command = Command::new(binary);
    command
        .args([
            "--ignored",
            "--exact",
            "runtime::process::network_native_tests::consume_host_cache",
            "--test-threads",
            "1",
        ])
        .env_clear()
        .env(CONSUMER, "on")
        .uid(owner.0)
        .gid(owner.1)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    host::drop_ambient_on_exec(&mut command);
    let mut child = network.spawn_verified("route-guard", &mut command).unwrap();
    child.stdin.take().unwrap().write_all(&serde_json::to_vec(&input).unwrap()).unwrap();
    assert!(child.wait().unwrap().success());
}

#[tokio::test]
#[ignore = "loads real kernel policy and requires a dedicated UID"]
async fn a_cold_artifact_is_acquired_before_filtered_dedicated_uid_cache_consumption() {
    if !rustix::process::geteuid().is_root() {
        eprintln!("root proves the filtered dedicated-UID host-cache boundary");
        return;
    }
    let fixture = Fixture::new(true).await;
    fs::set_permissions(fixture.root.path(), fs::Permissions::from_mode(0o711)).unwrap();
    let owner = fixture.runtime.owner("native-fixture").unwrap();
    let pending = fixture.pending();
    fixture.started().await;
    fixture.runtime.traversable(&fixture.entry.spec.data_dir).unwrap();
    let downloads = fixture.root.path().join("consumer-downloads");
    fs::create_dir(&downloads).unwrap();
    fs::set_permissions(&downloads, fs::Permissions::from_mode(0o700)).unwrap();
    rustix::fs::chown(
        &downloads,
        Some(rustix::process::Uid::from_raw(owner.0)),
        Some(rustix::process::Gid::from_raw(owner.1)),
    )
    .unwrap();
    let binary = fixture.root.path().join("cache-consumer");
    if fs::hard_link(std::env::current_exe().unwrap(), &binary).is_err() {
        fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
    }
    let network = Policed(Network::new(
        Host::find().unwrap_or_else(|problem| panic!("{}", problem.evidence)),
        Served { ports: 42000..=42999, api_port: 7443 },
        &fixture.runtime.state.root,
    ));
    let mut spec = fixture.entry.spec.clone();
    spec.egress = Some(crate::protocol::EgressPolicy { allow: vec!["example.com".into()], deny: Vec::new() });
    network.0.prepare(&spec).await.unwrap();
    let cache = fixture.base().join("artifacts");
    consumer(
        &network.0,
        &binary,
        owner,
        Consumer {
            cache: cache.clone(),
            downloads: downloads.clone(),
            artifact: fixture.artifact.clone(),
            succeeds: false,
        },
    );
    assert!(network.0.counts("route-guard").unwrap().refused_addresses > 0);
    fixture.gate.add_permits(1);
    assert_eq!(pending.await.unwrap().unwrap(), cache);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1, "only the unfiltered daemon fetched the artifact");
    consumer(
        &network.0,
        &binary,
        owner,
        Consumer { cache, downloads, artifact: fixture.artifact.clone(), succeeds: true },
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1, "the filtered consumer used only verified cached bytes");
    assert_eq!(fixture.runtime.state.owners.lock().unwrap().get("native-fixture"), Some(&owner));
}
