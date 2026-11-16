// SPDX-License-Identifier: FSL-1.1-ALv2
//! Raw-engine, data-less fixtures prove ingress independently of the data-owner capability.
//! They keep the standard nonzero workload policy and do not bypass production ownership checks.
use super::*;
use crate::{labels::LABEL_INTERNAL_BINDINGS, protocol::Proto};
use bollard::models::{ContainerCreateBody, NetworkCreateRequest};
use std::{collections::HashMap, net::Ipv4Addr, path::Path, process::Command};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    endpoint: String,
    name: String,
    network: String,
    _files: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for args in [vec!["rm", "--force", &self.name], vec!["network", "rm", &self.network]] {
            let _ = Command::new("docker")
                .env_remove("DOCKER_HOST")
                .env_remove("DOCKER_CONTEXT")
                .env_remove("DOCKER_TLS")
                .env_remove("DOCKER_TLS_VERIFY")
                .env_remove("DOCKER_CERT_PATH")
                .args(["--host", &self.endpoint])
                .args(args)
                .output();
        }
    }
}

#[tokio::test]
#[ignore = "requires a real Docker engine and PRYX_INTERNAL_INGRESS_AGENT static binary"]
async fn engine_private_exec_ingress() {
    let endpoint = std::env::var("DOCKER_HOST").unwrap_or_else(|_| "unix:///var/run/docker.sock".into());
    let socket = endpoint.strip_prefix("unix://").expect("local Unix engine");
    let agent = PathBuf::from(std::env::var_os("PRYX_INTERNAL_INGRESS_AGENT").expect("static agent fixture"));
    let runtime = DockerRuntime::new(Path::new(socket)).unwrap().with_internal_ingress_agent(&agent);
    let docker = runtime.docker().unwrap();
    let metadata = runtime.info().await.unwrap();
    assert!(metadata.rootless && !metadata.podman, "this fixture proves the rootless Docker exec path");
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let staged = super::super::exec_agent::stage(&agent, &data).unwrap();
    let service = directory.path().join("service");
    let result = Command::new("cc")
        .args(["-static", "-O2"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ingress-service.c"))
        .arg("-o")
        .arg(&service)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let name = format!("pryx-ingress-{}", uuid::Uuid::new_v4().simple());
    let network = format!("{name}-net");
    let fixture =
        Fixture { endpoint: endpoint.clone(), name: name.clone(), network: network.clone(), _files: directory };
    docker
        .create_network(NetworkCreateRequest {
            name: network.clone(),
            internal: Some(true),
            options: Some(HashMap::from([("com.docker.network.bridge.enable_icc".into(), "false".into())])),
            ..Default::default()
        })
        .await
        .unwrap();
    let bindings = vec![binding(Proto::Tcp, 8080), binding(Proto::Udp, 8081)];
    let versions: serde_json::Value =
        serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/.github/engine-versions.json")))
            .unwrap();
    let body: ContainerCreateBody = serde_json::from_value(serde_json::json!({
        "Image": versions["image"]["reference"], "User": "1000:1000", "Entrypoint": ["/fixture"],
        "Labels": {LABEL_INTERNAL_BINDINGS: serde_json::to_string(&bindings).unwrap(),
            crate::labels::LABEL_INTERNAL_INGRESS: pryx_agent::exec_ingress::VERSION.to_string()},
        "HostConfig": {"ReadonlyRootfs": true, "Privileged": false, "CapDrop": ["ALL"],
            "SecurityOpt": ["no-new-privileges"], "NetworkMode": network,
            "Mounts": [{"Type": "bind", "Source": staged, "Target": pryx_agent::contract::BINARY_PATH, "ReadOnly": true},
                       {"Type": "bind", "Source": service, "Target": "/fixture", "ReadOnly": true}]}
    })).unwrap();
    let options = CreateContainerOptionsBuilder::new().name(&name).build();
    let made = docker.create_container(Some(options), body).await.unwrap();
    docker.start_container(&made.id, None).await.unwrap();
    let inspected = docker.inspect_container(&made.id, None).await.unwrap();
    assert!(inspected.network_settings.as_ref().and_then(|n| n.ports.as_ref()).is_none_or(HashMap::is_empty));
    let target = Target::new(&runtime, &inspected, &bindings[0]).await.unwrap();
    let capacity = Arc::new(tokio::sync::Semaphore::new(1));
    let mut connection = target.connect(Some(capacity.clone().acquire_owned().await.unwrap())).await.unwrap();
    assert_eq!(capacity.available_permits(), 0);
    let payload = (0..=u8::MAX).cycle().take(96 * 1024).collect::<Vec<_>>();
    connection.stream.write_all(&payload).await.unwrap();
    connection.stream.shutdown().await.unwrap();
    let mut answer = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), connection.stream.read_to_end(&mut answer)).await.unwrap().unwrap();
    let mut expected = b"after-eof:".to_vec();
    expected.extend(payload);
    assert!(answer == expected, "real Docker attachment changed the delayed half-close reply");
    drop(connection);
    healthy_and_finished(&docker, &made.id, &inspected).await;
    let permit = tokio::time::timeout(Duration::from_secs(3), capacity.acquire()).await.unwrap().unwrap();
    drop(permit);
    udp(&runtime, &inspected, &bindings[1]).await;
    healthy_and_finished(&docker, &made.id, &inspected).await;
    let forbidden = binding(Proto::Tcp, 8082);
    assert!(Target::new(&runtime, &inspected, &forbidden).await.is_err());
    // Disconnect an active opaque stream, rather than a fully completed response.
    drop(target.connect(None).await.unwrap());
    healthy_and_finished(&docker, &made.id, &inspected).await;
    stalled_owner_and_replacement(&runtime, &agent, &docker, &inspected, &bindings[0], target).await;
    drop(fixture);
}

fn binding(protocol: Proto, container_port: u16) -> PortBinding {
    PortBinding { container_port, host_port: 0, host_ips: vec![Ipv4Addr::LOCALHOST.into()], protocol }
}

async fn healthy_and_finished(docker: &Docker, id: &str, before: &ContainerInspectResponse) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let info = docker.inspect_container(id, None).await.unwrap();
            assert_eq!(info.id, before.id);
            let state = info.state.as_ref().unwrap();
            assert_eq!(state.running, Some(true), "ordinary pipe EOF must not retire a healthy sandbox");
            assert_eq!(state.started_at, before.state.as_ref().unwrap().started_at);
            let mut active = false;
            for exec in info.exec_ids.as_ref().into_iter().flatten() {
                active |= docker.inspect_exec(exec).await.unwrap().running == Some(true);
            }
            if !active {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("disconnected helpers must exit promptly, before fallback retirement");
}

async fn udp(runtime: &DockerRuntime, info: &ContainerInspectResponse, binding: &PortBinding) {
    let target = Target::new(runtime, info, binding).await.unwrap();
    let first = target.connect(None).await.unwrap();
    let second = target.connect(None).await.unwrap();
    let (mut first, mut second) =
        (Framed::new(first.stream, packet_codec()), Framed::new(second.stream, packet_codec()));
    let mut peers = Vec::new();
    for (index, payload) in
        [Bytes::from_static(b"first"), Bytes::new(), Bytes::from(vec![0xa5; 65_500])].into_iter().enumerate()
    {
        let stream = if index == 1 { &mut second } else { &mut first };
        stream.send(payload.clone()).await.unwrap();
        let answer = tokio::time::timeout(Duration::from_secs(5), stream.next()).await.unwrap().unwrap().unwrap();
        peers.push(answer[..2].to_vec());
        assert_eq!(&answer[2..], payload.as_ref());
    }
    assert_ne!(peers[0], peers[1]);
    assert_eq!(peers[0], peers[2]);
}

async fn stalled_owner_and_replacement(
    runtime: &DockerRuntime,
    agent: &Path,
    docker: &Docker,
    before: &ContainerInspectResponse,
    binding: &PortBinding,
    old: Arc<Target>,
) {
    let id = before.id.as_ref().unwrap();
    let capacity = Arc::new(tokio::sync::Semaphore::new(1));
    let abandoned = old.connect(Some(capacity.clone().acquire_owned().await.unwrap())).await.unwrap();
    // Stop only this fixture's helper from its own PID namespace. It cannot process pipe
    // EOF, which is the real orphan-adoption case that previously reacquired Starts.
    let pause = docker
        .create_exec(
            id,
            CreateExecOptions::<String> {
                cmd: Some(vec!["/bin/sh".into(), "-c".into(), "kill -STOP $(pidof .pryx-agent)".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let _ = docker
        .start_exec(&pause.id, Some(StartExecOptions { detach: true, tty: false, output_capacity: None }))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let stopped = docker.inspect_exec(&pause.id).await.unwrap();
            if stopped.running == Some(false) {
                assert_eq!(stopped.exit_code, Some(0), "the fixture must actually stop its helper");
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let fresh = DockerRuntime::new(Path::new(&runtime.socket)).unwrap().with_internal_ingress_agent(agent);
    fresh.info().await.unwrap();
    let error = tokio::time::timeout(QUICK + Duration::from_secs(10), fresh.start(id))
        .await
        .expect("idempotent start with a stalled orphan must finish within its cleanup bound")
        .unwrap_err();
    assert!(error.to_string().contains("old container start was retired"), "{error}");
    assert_eq!(docker.inspect_container(id, None).await.unwrap().state.unwrap().running, Some(false));
    assert_eq!(
        capacity.available_permits(),
        0,
        "the old pipe keeps its budget until its owner is dropped and confirmed"
    );
    drop(abandoned);
    docker.start_container(id, None).await.unwrap();
    let replacement = docker.inspect_container(id, None).await.unwrap();
    assert_ne!(replacement.state.as_ref().unwrap().started_at, before.state.as_ref().unwrap().started_at);
    assert!(old.connect(None).await.is_err(), "a previous target must not attach to a later start");
    let current = Target::new(runtime, &replacement, binding).await.unwrap();
    drop(current.connect(None).await.unwrap());
    healthy_and_finished(docker, id, &replacement).await;
    let permit = tokio::time::timeout(Duration::from_secs(3), capacity.acquire()).await.unwrap().unwrap();
    drop(permit);
}
