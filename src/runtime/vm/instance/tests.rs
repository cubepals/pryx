// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real KVM acceptance for the runtime seam, not just a monitor fixture. The existing
//! optional VM job supplies the complete pinned archive and real public OCI registry inputs.
use super::*;
use crate::runtime::{AgentBoot, ContainerRuntime, ContainerStatus, PortBinding};
use futures_util::StreamExt as _;
use std::net::{Ipv4Addr, TcpListener};
use tokio::io::AsyncReadExt;

pub(super) fn input(name: &str) -> PathBuf {
    std::env::var_os(name).unwrap_or_else(|| panic!("Stage {name}")).into()
}
pub(super) fn spec(data: &Path, image: &str) -> ContainerSpec {
    let sandbox = uuid::Uuid::new_v4().simple().to_string();
    let secret_env = format!("{}{}", pryx_agent::contract::SECRET_ENV_PREFIX, uuid::Uuid::new_v4().simple());
    let (_, encoded) = pryx_agent::auth::SigningKey::generate().unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let uid = rustix::process::geteuid().as_raw().checked_add(1).unwrap();
    ContainerSpec {
        agent: Some(AgentBoot {
            public_origin: None,
            binary: std::env::current_exe().unwrap(),
            sandbox: sandbox.clone(),
            epoch: 1,
            secret_env: secret_env.clone(),
            domain: "vm.pryx.invalid".into(),
            agent_port: pryx_agent::contract::AGENT_PORT,
            spare_ports: Vec::new(),
            keep_alive: true,
        }),
        name: format!("vm-{sandbox}"),
        cgroup_parent: None,
        image: image.into(),
        packages: Vec::new(),
        entrypoint: None,
        workdir: None,
        egress: None,
        env: vec![(secret_env, encoded)],
        labels: BTreeMap::new(),
        user: format!("{uid}:{uid}"),
        memory_bytes: 256 * 1024 * 1024,
        storage_bytes: 64 * 1024 * 1024,
        cpu_weight_requested: false,
        allow_unenforced_memory: None,
        require_cpu_limit: true,
        require_pids_limit: true,
        require_browser_sandbox: None,
        nano_cpus: Some(2_000_000_000),
        cpu_shares: 1024,
        pids_limit: 256,
        read_only_rootfs: true,
        tmp_size_mb: 32,
        data_dir: data.into(),
        mount_path: "/data".into(),
        ports: vec![PortBinding {
            container_port: pryx_agent::contract::AGENT_PORT,
            protocol: crate::protocol::Proto::Tcp,
            host_ips: vec![Ipv4Addr::LOCALHOST.into()],
            host_port: port,
        }],
        stop_signal: crate::protocol::StopSignal::SIGTERM.as_str().into(),
        stop_timeout_secs: 15,
        network: String::new(),
        internal_network: true,
        log_max_size_mb: 1,
        log_max_files: 1,
        oom_score_adj: 0,
    }
}
pub(super) fn file_request(spec: &ContainerSpec) -> crate::runtime::files::Request {
    let agent = spec.agent.as_ref().unwrap();
    crate::runtime::files::Request {
        name: spec.name.clone(),
        sandbox: agent.sandbox.clone(),
        epoch: agent.epoch,
        data_dir: spec.data_dir.clone(),
        mount_path: spec.mount_path.clone(),
        user: spec.user.clone(),
        memory_bytes: spec.memory_bytes,
        storage_bytes: spec.storage_bytes,
        pids_limit: spec.pids_limit,
        tmp_size_mb: spec.tmp_size_mb,
        log_max_size_mb: spec.log_max_size_mb,
    }
}
async fn exec(runtime: &VmRuntime, id: &str, script: &str) -> Vec<u8> {
    eprintln!("VM runtime exec: {script}");
    let output = runtime
        .exec(
            id,
            &["/bin/sh".into(), "-c".into(), script.into()],
            Duration::from_secs(5),
            pryx_agent::contract::CAPTURE_BYTES,
        )
        .await
        .unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert!(!output.timed_out && !output.killed);
    output.stdout
}
async fn workload_log(runtime: &VmRuntime, id: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let chunks: Vec<_> = runtime.logs(id, crate::runtime::LogOptions::default()).collect().await;
        if chunks.into_iter().any(|chunk| chunk.unwrap().bytes.ends_with(b" workload-log")) {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "Guest entrypoint output did not reach logs");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn guest_firewall(runtime: &VmRuntime, id: &str) {
    use pryx_agent::vm_boot;
    let bundle = runtime.bundle().await.unwrap();
    let host = bundle.filesystems().builder("nft").arg("--version").output().unwrap();
    assert!(host.status.success());
    let root = Path::new(vm_boot::BOOTSTRAP_ROOT).join(vm_boot::FIREWALL_DIRECTORY);
    let loader = root.join(vm_boot::FIREWALL_LOADER);
    let libraries = loader.parent().unwrap();
    let arguments: Vec<String> = [
        loader.to_str().unwrap(),
        "--library-path",
        libraries.to_str().unwrap(),
        root.join(vm_boot::FIREWALL_PROGRAM).to_str().unwrap(),
        "--version",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let version = runtime.exec(id, &arguments, Duration::from_secs(5), 4096).await.unwrap();
    assert_eq!(version.exit_code, Some(0));
    assert_eq!(version.stdout, host.stdout, "Guest uses the same unchanged verified nft program");
    let mut flush = arguments;
    flush.pop();
    flush.extend(["flush".into(), "ruleset".into()]);
    let refused = runtime.exec(id, &flush, Duration::from_secs(5), 4096).await.unwrap();
    assert!(refused.exit_code.is_some_and(|code| code != 0));
    assert!(!refused.timed_out && !refused.killed, "Guest policy is refused by the kernel");
}
async fn guest_socket_policy(runtime: &VmRuntime, spec: &ContainerSpec, id: &str) {
    let files = runtime.data_service(file_request(spec)).await.unwrap();
    let probe = fs::read(input("PRYX_VM_POLICY_PROBE")).unwrap();
    let response = files
        .request(crate::runtime::files::Operation::Write, "policy-probe", crate::http_client::full(probe))
        .await
        .unwrap();
    assert_eq!(response.status(), hyper::StatusCode::NO_CONTENT);
    files.close().await.unwrap();
    let result = exec(runtime, id, "chmod 500 /data/policy-probe; /data/policy-probe --policy-only").await;
    let observed: BTreeMap<String, bool> = serde_json::from_slice(&result).unwrap();
    assert!(observed["vsockSocketDenied"] && observed["vsockSocketpairDenied"] && observed["ioUringSetupDenied"]);
    exec(runtime, id, "rm /data/policy-probe").await;
}
pub(super) async fn artifact_server() -> (Artifact, tokio::task::JoinHandle<()>) {
    let mut artifact: Artifact =
        serde_json::from_slice(&fs::read(input("PRYX_VM_RUNTIME_DESCRIPTOR")).unwrap()).unwrap();
    let archive = input("PRYX_VM_RUNTIME_ARCHIVE");
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    artifact.url = format!("http://{}/runtime", listener.local_addr().unwrap());
    let router = axum::Router::new().route(
        "/runtime",
        axum::routing::get(move || {
            let archive = archive.clone();
            async move {
                let file = tokio::fs::File::open(archive).await.unwrap();
                let stream = futures_util::stream::try_unfold(file, |mut file| async move {
                    let mut bytes = vec![0; 64 * 1024];
                    let size = file.read(&mut bytes).await?;
                    bytes.truncate(size);
                    Ok::<_, std::io::Error>((size != 0).then_some((bytes, file)))
                });
                axum::body::Body::from_stream(stream)
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (artifact, server)
}
#[tokio::test]
#[ignore = "requires real KVM, verified VM archive and public registries; existing optional VM CI runs it"]
async fn actual_runtime_cold_boot_exec_stop_wake_copy_and_fork() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let temporary = tempfile::tempdir().unwrap();
    let (artifact, server) = artifact_server().await;
    let mut runtime = VmRuntime::new(
        temporary.path(),
        Settings { artifact, ingress_connections: 8, free_floor_bytes: 0, image_timeout: Duration::from_secs(120) },
    )
    .await
    .unwrap();
    let inputs: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../../templates/vm-tools/image-inputs.json")).unwrap();
    for (index, image) in inputs.iter().enumerate() {
        eprintln!("VM runtime image {index}: create and initial boot");
        let data = temporary.path().join(format!("data{index}"));
        directory(&data).unwrap();
        let mut original_spec = spec(&data, image["reference"].as_str().unwrap());
        if index == 1 {
            original_spec.stop_signal = crate::protocol::StopSignal::SIGINT.as_str().into();
            original_spec.agent.as_mut().unwrap().keep_alive = false;
            original_spec.entrypoint = Some(vec!["/bin/sh".into(), "-c".into(),
                "trap 'printf interrupted > /data/signal; exit 0' INT; printf ready > /data/signal-ready; printf workload-log; while :; do sleep 1; done".into()]);
        }
        let id = runtime.create(&original_spec).await.unwrap();
        let settings = runtime.state.settings.clone();
        assert!(VmRuntime::new(temporary.path(), settings.clone()).await.is_err(), "Only one runtime may own journals");
        drop(runtime);
        runtime = VmRuntime::new(temporary.path(), settings).await.unwrap();
        assert_eq!(runtime.inspect(&id).await.unwrap().unwrap().status, ContainerStatus::Created);
        runtime.start(&id).await.unwrap();
        observe_live(&runtime, &id, &original_spec, index).await;
        runtime.stop(&id, &original_spec.stop_signal, Duration::from_secs(15)).await.unwrap();
        assert_eq!(runtime.inspect(&id).await.unwrap().unwrap().status, ContainerStatus::Exited);
        assert!(runtime.stats(&id).await.unwrap().is_none(), "Stopped guests must not return stale counters");
        let settings = runtime.state.settings.clone();
        drop(runtime);
        runtime = VmRuntime::new(temporary.path(), settings).await.unwrap();
        let restored = runtime.inspect(&id).await.unwrap().unwrap();
        assert_eq!(restored.status, ContainerStatus::Exited);
        assert_eq!(restored.exit_code, 0);
        assert_eq!(restored.env, original_spec.env.iter().cloned().collect());
        let sandbox = &original_spec.agent.as_ref().unwrap().sandbox;
        let copy_lease = runtime.data_copy_lease(sandbox, &data).await.unwrap();
        assert!(runtime.start(&id).await.is_err(), "A copy lease must fence another writable guest");
        let copy = temporary.path().join(format!("copy{index}"));
        crate::tree::copy_tree(&data, &copy, None, &[], &mut |_| Ok(())).unwrap();
        drop(copy_lease);
        eprintln!("VM runtime image {index}: file-only guest");
        let bytes = offline_files(&runtime, &id, &original_spec, index).await;
        runtime.remove(&id).await.unwrap();
        let retained_files = runtime.data_service(file_request(&original_spec)).await.unwrap();
        assert!(runtime.inspect(&id).await.unwrap().is_none());
        assert!(runtime.list(&[]).await.unwrap().is_empty(), "Maintenance must not create a runtime entry");
        let response = retained_files
            .request(crate::runtime::files::Operation::Read, "offline", crate::http_client::empty())
            .await
            .unwrap();
        assert_eq!(crate::http_client::read_body(response, bytes.len()).await.unwrap().as_ref(), bytes);
        retained_files.close().await.unwrap();
        let id = runtime.create(&original_spec).await.unwrap();
        runtime.start(&id).await.unwrap();
        assert_eq!(exec(&runtime, &id, "wc -c < /data/offline").await, format!("{}\n", bytes.len()).as_bytes());
        assert_eq!(exec(&runtime, &id, "cat /data/retained").await, b"durable");
        eprintln!("VM runtime image {index}: independent fork");
        let fork_spec = spec(&copy, image["reference"].as_str().unwrap());
        let another = tempfile::tempdir().unwrap();
        let independent = VmRuntime::new(another.path(), runtime.state.settings.clone()).await.unwrap();
        let fork = independent.create(&fork_spec).await.unwrap();
        independent.start(&fork).await.unwrap();
        assert_eq!(exec(&independent, &fork, "cat /data/retained; printf fork > /data/retained").await, b"durable");
        assert_eq!(exec(&runtime, &id, "cat /data/retained").await, b"durable");
        independent.remove(&fork).await.unwrap();
        eprintln!("VM runtime image {index}: interrupted live runtime and failed ingress");
        runtime = interrupted(runtime, temporary.path(), &id, &original_spec, &bytes).await;
        eprintln!("VM runtime image {index}: immediate guest exit");
        immediate_exit(&runtime, temporary.path(), image["reference"].as_str().unwrap(), index).await;
        for unrestricted in [false, true] {
            super::network_acceptance::check(
                &runtime,
                temporary.path(),
                image["reference"].as_str().unwrap(),
                index,
                unrestricted,
            )
            .await;
        }
    }
    server.abort();
    println!(
        "Actual runtime seam: both OCI images, 2-vCPU boot, exec, clean stop, cold wake, copied-disk fork isolation and remove passed"
    );
}

#[test]
fn unfinished_launch_refuses_a_stopped_observation_until_its_kernel_lease_retires() {
    let root = tempfile::tempdir().unwrap();
    let lease = root.path().join("monitor.lock");
    let mut entry = Entry {
        id: format!("vm-{}", uuid::Uuid::new_v4().simple()),
        spec: spec(root.path(), "unused"),
        started: Some(crate::clock::now()),
        finished: None,
        code: UNKNOWN_EXIT_CODE,
        agent_state: None,
    };
    let held = lock(&lease).unwrap();
    assert!(matches!(runtime::stopped_status(&entry, &lease), Err(RuntimeError::Unavailable(_))));
    drop(held);
    assert_eq!(runtime::stopped_status(&entry, &lease).unwrap(), ContainerStatus::Dead);
    entry.finished = Some(crate::clock::now());
    assert_eq!(runtime::stopped_status(&entry, &lease).unwrap(), ContainerStatus::Dead);
    entry.code = 0;
    assert_eq!(runtime::stopped_status(&entry, &lease).unwrap(), ContainerStatus::Exited);
    entry.started = None;
    assert_eq!(runtime::stopped_status(&entry, &lease).unwrap(), ContainerStatus::Created);
}

#[test]
fn launch_directory_is_private_before_bootstrap_creation() {
    use std::os::unix::fs::PermissionsExt;
    let parent = tempfile::tempdir().unwrap();
    let boot = boot_directory(parent.path()).unwrap();
    assert_eq!(fs::metadata(boot.path()).unwrap().permissions().mode() & 0o777, 0o700);
}

#[test]
fn maintenance_has_private_fresh_credentials_and_no_workload_or_extra_ingress() {
    let directory = tempfile::tempdir().unwrap();
    let mut original = spec(directory.path(), "invalid");
    let old_key = agent::key(&original).unwrap().public_key();
    original.env.push(("WORKLOAD_SECRET".into(), "private-workload-value".into()));
    original.agent.as_mut().unwrap().spare_ports.push(3000);
    original.entrypoint = Some(vec!["workload".into()]);
    original.workdir = Some("/workload".into());
    original.read_only_rootfs = false;
    let maintenance = maintenance::specification(&original).unwrap();
    let second = maintenance::specification(&original).unwrap();
    assert_eq!(maintenance.env.len(), 1);
    assert_ne!(agent::key(&maintenance).unwrap().public_key(), old_key);
    assert_ne!(agent::key(&maintenance).unwrap().public_key(), agent::key(&second).unwrap().public_key());
    assert!(maintenance.entrypoint.as_ref().unwrap().is_empty());
    assert_eq!(maintenance.workdir.as_deref(), Some("/"));
    assert!(maintenance.internal_network && maintenance.egress.is_none() && maintenance.read_only_rootfs);
    assert!(maintenance.agent.as_ref().unwrap().spare_ports.is_empty());
    assert_eq!(maintenance.ports.len(), 1);
    assert_eq!(maintenance.ports[0].host_port, 0);
    assert_eq!(maintenance.ports[0].host_ips, vec![std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)]);
    assert_eq!(maintenance.mount_path, original.mount_path);
    assert_eq!(maintenance.user, original.user);
    assert_eq!(maintenance.memory_bytes, original.memory_bytes);
}

#[test]
fn private_journals_preserve_stopped_state_and_refuse_links_modes_and_invalid_values() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let entries = root.path().join("entries");
    directory(&entries).unwrap();
    let now = crate::clock::now();
    let mut entry = Entry {
        id: format!("vm-{}", uuid::Uuid::new_v4().simple()),
        spec: spec(root.path(), "invalid"),
        started: Some(now),
        finished: Some(now),
        code: 7,
        agent_state: None,
    };
    let path = entries.join(format!("{}.json", entry.id));
    let write =
        |entry: &Entry| crate::durable::write_atomic(&path, &serde_json::to_vec(entry).unwrap(), 0o600).unwrap();
    write(&entry);
    let loaded = journal::load(root.path()).unwrap();
    assert_eq!(loaded[&entry.id].code, 7);
    assert_eq!(loaded[&entry.id].started, Some(now));
    assert_eq!(loaded[&entry.id].finished, Some(now));
    assert_eq!(loaded[&entry.id].spec.env, entry.spec.env);
    entry.finished = None;
    write(&entry);
    let interrupted = journal::load(root.path()).unwrap();
    assert_eq!(interrupted[&entry.id].code, UNKNOWN_EXIT_CODE);
    assert!(interrupted[&entry.id].finished.is_some());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(journal::load(root.path()).is_err());
    write(&entry);
    let other = root.path().join("other");
    fs::hard_link(&path, &other).unwrap();
    assert!(journal::load(root.path()).is_err());
    fs::remove_file(&other).unwrap();
    fs::rename(&path, &other).unwrap();
    symlink(&other, &path).unwrap();
    assert!(journal::load(root.path()).is_err());
    fs::remove_file(&path).unwrap();
    write(&entry);
    let secret = uuid::Uuid::new_v4().simple().to_string();
    fs::write(&path, format!("{{\"env\":\"{secret}\"}}")).unwrap();
    let error = journal::load(root.path()).err().unwrap().to_string();
    assert_eq!(error, "Invalid private VM journal");
    assert!(!error.contains(&secret));
    write(&entry);
    let mut duplicate = entry.clone();
    duplicate.id = format!("vm-{}", uuid::Uuid::new_v4().simple());
    crate::durable::write_atomic(
        &entries.join(format!("{}.json", duplicate.id)),
        &serde_json::to_vec(&duplicate).unwrap(),
        0o600,
    )
    .unwrap();
    assert!(journal::load(root.path()).is_err());
}

#[test]
fn retained_file_spec_has_only_host_owned_data_policy() {
    let root = tempfile::tempdir().unwrap();
    let original = spec(root.path(), "workload-image");
    let minimal = services::specification(file_request(&original), std::env::current_exe().unwrap());
    assert!(minimal.image.is_empty() && minimal.env.is_empty() && minimal.labels.is_empty());
    assert!(minimal.packages.is_empty() && minimal.ports.is_empty());
    assert!(minimal.entrypoint.as_ref().unwrap().is_empty());
    assert_eq!(minimal.user, original.user);
    assert_eq!(minimal.data_dir, original.data_dir);
    assert_eq!(minimal.memory_bytes, original.memory_bytes);
    assert_eq!(minimal.storage_bytes, original.storage_bytes);
    let prepared = maintenance::specification(&minimal).unwrap();
    assert_eq!(prepared.env.len(), 1);
    assert_ne!(agent::key(&prepared).unwrap().public_key(), agent::key(&original).unwrap().public_key());
    let (memory, cpus, _, _) = launch::resources(&prepared).unwrap();
    assert_eq!(memory, original.memory_bytes as u64 / (1024 * 1024));
    assert_eq!(cpus, 1);
}

async fn observe_live(runtime: &VmRuntime, id: &str, original_spec: &ContainerSpec, index: usize) {
    let data = &original_spec.data_dir;
    assert_eq!(runtime.inspect(id).await.unwrap().unwrap().status, ContainerStatus::Running);
    guest_firewall(runtime, id).await;
    guest_socket_policy(runtime, original_spec, id).await;
    if index == 1 {
        assert_eq!(
            exec(runtime, id, "while ! test -e /data/signal-ready; do sleep 0.01; done; cat /data/signal-ready").await,
            b"ready"
        );
        workload_log(runtime, id).await;
    }
    assert_eq!(exec(runtime, id, "printf durable > /data/retained; printf ready").await, b"ready");
    assert_eq!(exec(runtime, id, "grep -c '^processor' /proc/cpuinfo").await, b"2\n");
    let active_files = runtime.data_service(file_request(original_spec)).await.unwrap();
    let response = active_files
        .request(crate::runtime::files::Operation::Read, "retained", crate::http_client::empty())
        .await
        .unwrap();
    assert_eq!(crate::http_client::read_body(response, 1024).await.unwrap(), "durable");
    active_files.close().await.unwrap();
    assert_eq!(runtime.inspect(id).await.unwrap().unwrap().status, ContainerStatus::Running);
    let mut wrong_epoch = file_request(original_spec);
    wrong_epoch.epoch += 1;
    assert!(runtime.data_service(wrong_epoch).await.is_err());
    assert!(
        runtime.data_copy_lease(&original_spec.agent.as_ref().unwrap().sandbox, data).await.is_err(),
        "A running monitor must refuse a disk copy lease"
    );
    observe_resources(runtime, id, original_spec).await;
}

async fn offline_files(runtime: &VmRuntime, id: &str, original_spec: &ContainerSpec, index: usize) -> Vec<u8> {
    let sandbox = &original_spec.agent.as_ref().unwrap().sandbox;
    let data = &original_spec.data_dir;
    let files = runtime.open_files(id).await.unwrap();
    assert!(runtime.data_copy_lease(sandbox, data).await.is_err(), "File service holds the real disk lease");
    let response = files
        .request(hyper::Method::GET, "/_pryx/files/content?path=retained", crate::http_client::empty())
        .await
        .unwrap();
    assert_eq!(response.status(), hyper::StatusCode::OK);
    assert_eq!(crate::http_client::read_body(response, 1024).await.unwrap(), "durable");
    let response =
        files.request(hyper::Method::POST, "/_pryx/processes", crate::http_client::full("{}")).await.unwrap();
    assert_eq!(response.status(), hyper::StatusCode::NOT_FOUND);
    let files = files.into_service().unwrap();
    if index == 1 {
        let response =
            files.request(crate::runtime::files::Operation::Read, "signal", crate::http_client::empty()).await.unwrap();
        assert_eq!(crate::http_client::read_body(response, 1024).await.unwrap(), "interrupted");
        let response = files
            .request(crate::runtime::files::Operation::Read, pryx_agent::logbook::LOG_PATH, crate::http_client::empty())
            .await
            .unwrap();
        let bytes = crate::http_client::read_body(response, pryx_agent::contract::CAPTURE_BYTES * 4).await.unwrap();
        let chunks = crate::runtime::logs::records(&bytes, &crate::runtime::LogOptions::default()).unwrap();
        assert!(chunks.iter().any(|chunk| chunk.bytes.ends_with(b" workload-log")));
    }
    let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(pryx_agent::contract::MAX_REQUEST_BYTES + 1).collect();
    let response = files
        .request(crate::runtime::files::Operation::Write, "offline", crate::http_client::full(bytes.clone()))
        .await
        .unwrap();
    assert_eq!(response.status(), hyper::StatusCode::NO_CONTENT);
    let response =
        files.request(crate::runtime::files::Operation::Read, "offline", crate::http_client::empty()).await.unwrap();
    assert_eq!(crate::http_client::read_body(response, bytes.len()).await.unwrap().as_ref(), bytes);
    files.close().await.unwrap();
    drop(runtime.data_copy_lease(sandbox, data).await.unwrap());
    bytes
}

async fn interrupted(
    runtime: VmRuntime,
    root: &Path,
    id: &str,
    original_spec: &ContainerSpec,
    bytes: &[u8],
) -> VmRuntime {
    let data = &original_spec.data_dir;
    let settings = runtime.state.settings.clone();
    drop(runtime);
    let runtime = VmRuntime::new(root, settings).await.unwrap();
    let interrupted = runtime.inspect(id).await.unwrap().unwrap();
    assert_eq!(interrupted.status, ContainerStatus::Dead);
    assert_eq!(interrupted.exit_code, UNKNOWN_EXIT_CODE, "Daemon loss must not invent a signed guest exit");
    let occupied = occupied_ingress(original_spec.ports[0].host_port).await;
    assert!(runtime.start(id).await.is_err(), "A real occupied ingress must refuse startup");
    drop(occupied);
    let deadline = tokio::time::Instant::now() + crate::runtime::ENGINE_PROBE_TIMEOUT;
    loop {
        match runtime.inspect(id).await {
            Ok(Some(info)) => {
                assert_eq!(info.status, ContainerStatus::Dead);
                assert_eq!(info.exit_code, UNKNOWN_EXIT_CODE);
                break;
            }
            Err(problem) if problem.is_unavailable() => {}
            Ok(_) => panic!("Failed launch lost its runtime entry"),
            Err(problem) => panic!("Unexpected failed-start error: {problem}"),
        }
        assert!(tokio::time::Instant::now() < deadline, "Failed launch did not release its real monitor lease");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    runtime.start(id).await.unwrap();
    assert_eq!(exec(&runtime, id, "cat /data/retained").await, b"durable");
    assert_eq!(exec(&runtime, id, "wc -c < /data/offline").await, format!("{}\n", bytes.len()).as_bytes());
    runtime.remove(id).await.unwrap();
    assert!(runtime.inspect(id).await.unwrap().is_none());
    assert!(data.join(super::super::disk::FILE).is_file());
    runtime
}

async fn immediate_exit(runtime: &VmRuntime, root: &Path, image: &str, index: usize) {
    let job_data = root.join(format!("job{index}"));
    directory(&job_data).unwrap();
    let mut job_spec = spec(&job_data, image);
    job_spec.agent.as_mut().unwrap().keep_alive = false;
    job_spec.entrypoint = Some(vec!["/bin/sh".into(), "-c".into(), "exit 7".into()]);
    let job = runtime.create(&job_spec).await.unwrap();
    runtime.start(&job).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let info = runtime.inspect(&job).await.unwrap().unwrap();
        if info.status == ContainerStatus::Exited {
            assert_eq!(info.exit_code, 7);
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "Completed guest was not retired");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    runtime.remove(&job).await.unwrap();
}

async fn observe_resources(runtime: &VmRuntime, id: &str, original_spec: &ContainerSpec) {
    let observed = runtime.inspect(id).await.unwrap().unwrap();
    let snapshot = observed.agent_state.expect("Guest metadata must cross the real vsock forwarder");
    assert!(snapshot.activity.at > 0);
    assert_eq!(snapshot.activity.streams, 0);
    assert!(snapshot.ports.targets.is_empty());
    let polled = runtime.inspect(id).await.unwrap().unwrap().agent_state.unwrap();
    assert_eq!(polled.activity.at, snapshot.activity.at, "Node polling must not touch guest activity");
    let resources = runtime.stats(id).await.unwrap().unwrap();
    assert!(resources.cpu_total_ns.unwrap() > 0);
    assert!(resources.memory_usage.unwrap() > 0);
    assert!(resources.memory_usage.unwrap() <= original_spec.memory_bytes as u64);
    assert!(resources.pids.unwrap() > 0 && resources.pids.unwrap() <= original_spec.pids_limit as u64);
    assert_eq!(resources.memory_limit, Some(original_spec.memory_bytes as u64));
    assert!(resources.rx_bytes.is_some() && resources.tx_bytes.is_some());
    assert_eq!(
        runtime.inspect(id).await.unwrap().unwrap().agent_state.unwrap().activity.at,
        snapshot.activity.at,
        "Resource polling must not touch workload activity"
    );
}

async fn occupied_ingress(port: u16) -> TcpListener {
    let deadline = tokio::time::Instant::now() + crate::runtime::ENGINE_PROBE_TIMEOUT;
    loop {
        match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            Ok(listener) => break listener,
            Err(problem)
                if problem.kind() == std::io::ErrorKind::AddrInUse && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(problem) => panic!("Owned forwarder did not release its ingress: {problem}"),
        }
    }
}
