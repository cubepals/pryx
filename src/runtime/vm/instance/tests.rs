// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real KVM acceptance for the runtime seam, not just a monitor fixture. The existing
//! optional VM job supplies the complete pinned archive and real public OCI registry inputs.
use super::*;
use crate::runtime::{AgentBoot, ContainerRuntime, ContainerStatus, PortBinding};
use std::net::{Ipv4Addr, TcpListener};
use tokio::io::AsyncReadExt;

fn input(name: &str) -> PathBuf {
    std::env::var_os(name).unwrap_or_else(|| panic!("Stage {name}")).into()
}
fn spec(data: &Path, image: &str) -> ContainerSpec {
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
async fn exec(runtime: &VmRuntime, id: &str, script: &str) -> Vec<u8> {
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
#[tokio::test]
#[ignore = "requires real KVM, verified VM archive and public registries; existing optional VM CI runs it"]
async fn actual_runtime_cold_boot_exec_stop_wake_copy_and_fork() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let temporary = tempfile::tempdir().unwrap();
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
    let runtime = VmRuntime::new(
        temporary.path(),
        Settings { artifact, ingress_connections: 8, free_floor_bytes: 0, image_timeout: Duration::from_secs(120) },
    )
    .await
    .unwrap();
    let inputs: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../../templates/vm-tools/image-inputs.json")).unwrap();
    for (index, image) in inputs.iter().enumerate() {
        let data = temporary.path().join(format!("data{index}"));
        directory(&data).unwrap();
        let original_spec = spec(&data, image["reference"].as_str().unwrap());
        let id = runtime.create(&original_spec).await.unwrap();
        runtime.start(&id).await.unwrap();
        assert_eq!(runtime.inspect(&id).await.unwrap().unwrap().status, ContainerStatus::Running);
        assert_eq!(exec(&runtime, &id, "printf durable > /data/retained; printf ready").await, b"ready");
        assert_eq!(exec(&runtime, &id, "grep -c '^processor' /proc/cpuinfo").await, b"2\n");
        let observed = runtime.inspect(&id).await.unwrap().unwrap();
        let snapshot = observed.agent_state.expect("Guest metadata must cross the real vsock forwarder");
        assert!(snapshot.activity.at > 0);
        assert_eq!(snapshot.activity.streams, 0);
        assert!(snapshot.ports.targets.is_empty());
        let polled = runtime.inspect(&id).await.unwrap().unwrap().agent_state.unwrap();
        assert_eq!(polled.activity.at, snapshot.activity.at, "Node polling must not touch guest activity");

        runtime.stop(&id, &original_spec.stop_signal, Duration::from_secs(15)).await.unwrap();
        assert_eq!(runtime.inspect(&id).await.unwrap().unwrap().status, ContainerStatus::Exited);
        let copy = temporary.path().join(format!("copy{index}"));
        crate::tree::copy_tree(&data, &copy, None, &[], &mut |_| Ok(())).unwrap();
        runtime.start(&id).await.unwrap();
        assert_eq!(exec(&runtime, &id, "cat /data/retained").await, b"durable");
        let fork_spec = spec(&copy, image["reference"].as_str().unwrap());
        let fork = runtime.create(&fork_spec).await.unwrap();
        runtime.start(&fork).await.unwrap();
        assert_eq!(exec(&runtime, &fork, "cat /data/retained; printf fork > /data/retained").await, b"durable");
        assert_eq!(exec(&runtime, &id, "cat /data/retained").await, b"durable");
        runtime.remove(&fork).await.unwrap();
        runtime.remove(&id).await.unwrap();
        assert!(runtime.inspect(&id).await.unwrap().is_none());
        assert!(data.join(super::super::disk::FILE).is_file());
        let job_data = temporary.path().join(format!("job{index}"));
        directory(&job_data).unwrap();
        let mut job_spec = spec(&job_data, image["reference"].as_str().unwrap());
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
    server.abort();
    println!(
        "Actual runtime seam: both OCI images, 2-vCPU boot, exec, clean stop, cold wake, copied-disk fork isolation and remove passed"
    );
}

#[test]
fn launch_directory_is_private_before_bootstrap_creation() {
    use std::os::unix::fs::PermissionsExt;
    let parent = tempfile::tempdir().unwrap();
    let boot = boot_directory(parent.path()).unwrap();
    assert_eq!(fs::metadata(boot.path()).unwrap().permissions().mode() & 0o777, 0o700);
}
