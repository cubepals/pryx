// SPDX-License-Identifier: FSL-1.1-ALv2
//! A captured standard-engine bridge refusal exercises manager admission and actual Docker
//! create bodies. Private loopback work needs no bridge; internet never weakens ICC policy.
#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::{
        config::Ids,
        manager::{Manager, Precondition, Records},
        protocol::NetworkMode,
    };
    use axum::{Json, Router, body::to_bytes, extract::Request, routing::any};
    use std::os::unix::fs::MetadataExt;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    #[derive(Default)]
    struct Wire {
        bridges: AtomicUsize,
        allow_bridge: AtomicBool,
        network: std::sync::Mutex<Option<bool>>,
        network_reads: AtomicUsize,
        network_failure: AtomicBool,
        creates: std::sync::Mutex<Vec<(serde_json::Value, serde_json::Value)>>,
        latest: std::sync::Mutex<Option<serde_json::Value>>,
    }

    #[test]
    fn disconnected_ingress_is_scoped_to_a_verified_rootless_docker_container() {
        let info = RuntimeInfo { rootless: true, private_ingress: Some(true), ..Default::default() };
        assert!(info.disconnected_ingress(crate::protocol::Isolation::Container.as_str()));
        for isolation in
            [crate::protocol::Isolation::Gvisor, crate::protocol::Isolation::Process, crate::protocol::Isolation::Vm]
        {
            assert!(!info.disconnected_ingress(isolation.as_str()));
        }
        for info in [
            RuntimeInfo { rootless: false, ..info.clone() },
            RuntimeInfo { podman: true, ..info.clone() },
            RuntimeInfo { private_ingress: None, ..info.clone() },
            RuntimeInfo { private_ingress: Some(false), ..info },
        ] {
            assert!(!info.disconnected_ingress(crate::protocol::Isolation::Container.as_str()));
        }
    }

    #[tokio::test]
    async fn private_ingress_recovers_after_a_failed_metadata_probe() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let socket = directory.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let wire = Arc::new(Wire::default());
        let app = Router::new().fallback(any(move |request| {
            answer(request, wire.clone(), serde_json::Value::Null, serde_json::Value::Null)
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let response = directory.path().join("metadata.json");
        let agent = compile_metadata_agent(
            directory.path(),
            &format!(
                r#"#include <stdio.h>
int main(void) {{
    FILE *response = fopen({response:?}, "r");
    if (response == NULL) return 1;
    int byte;
    while ((byte = fgetc(response)) != EOF) putchar(byte);
    return ferror(response) != 0;
}}
"#
            ),
        );
        let ids =
            Ids { uid: rustix::process::geteuid().as_raw().max(1), gid: rustix::process::getegid().as_raw().max(1) };
        let mut runtime = DockerRuntime::new(&socket).unwrap().with_internal_ingress_agent(&agent);
        runtime.ownership = ownership::Policy { user: ids, owner: Some(ids) };

        let stamp = || {
            let file = std::fs::symlink_metadata(&agent).unwrap();
            (file.dev(), file.ino(), file.size(), file.ctime(), file.ctime_nsec())
        };
        let original = stamp();
        assert!(crate::runtime::protected_agent(&agent, false).is_some());
        assert!(crate::runtime::agent_metadata::query(&agent).await.is_none());
        let first = runtime.info().await.unwrap();
        assert!(first.rootless && first.container_policy_problem.is_none());
        assert_eq!(first.private_ingress, Some(false), "A failed probe must leave ingress off");

        let capabilities = pryx_agent::bootstrap_capabilities::probe();
        std::fs::write(response, serde_json::to_vec(&capabilities).unwrap()).unwrap();
        assert_eq!(stamp(), original, "Only the metadata response changed");
        let metadata = crate::runtime::agent_metadata::query(&agent).await.unwrap();
        assert_eq!(metadata.internal_ingress_version, pryx_agent::exec_ingress::VERSION);
        assert_eq!(
            runtime.info().await.unwrap().private_ingress,
            Some(true),
            "The same protected executable now answers the ingress probe"
        );
        server.abort();
    }

    #[tokio::test]
    async fn network_none_manager_admits_without_a_bridge_and_internet_preserves_the_real_refusal() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = directory.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let wire = Arc::new(Wire::default());
        let seen = wire.clone();
        let reserved = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let forged_port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let image_labels = serde_json::json!({crate::labels::LABEL_INTERNAL_INGRESS: pryx_agent::exec_ingress::VERSION.to_string(),
            crate::labels::LABEL_INTERNAL_BINDINGS: serde_json::to_string(&vec![crate::runtime::PortBinding {
                container_port: 8080, host_port: forged_port, host_ips: vec![std::net::Ipv4Addr::LOCALHOST.into()],
                protocol: crate::protocol::Proto::Tcp,
            }]).unwrap()});
        let failure: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/rootless-bridge-failure.json"
        )))
        .unwrap();
        let original = failure["message"].as_str().unwrap().to_owned();
        let app = Router::new()
            .fallback(any(move |request| answer(request, seen.clone(), failure.clone(), image_labels.clone())));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let capabilities = serde_json::to_string(&pryx_agent::bootstrap_capabilities::Capabilities {
            internal_ingress_version: pryx_agent::exec_ingress::VERSION,
            ..Default::default()
        })
        .unwrap();
        let agent = metadata_agent(directory.path(), &capabilities);
        let ids =
            Ids { uid: rustix::process::geteuid().as_raw().max(1), gid: rustix::process::getegid().as_raw().max(1) };
        let mut last = None;
        for routed in [false, true] {
            last = Some(private_admission(directory.path(), &agent, ids, routed, &wire, &original).await);
        }
        assert_eq!(wire.bridges.load(Ordering::SeqCst), 2);
        assert_eq!(wire.creates.lock().unwrap().len(), 2);
        let (manager, runtime) = last.unwrap();
        connected_and_loss(manager, runtime, &wire, &original, forged_port, directory.path(), &agent).await;
        server.abort();
    }

    async fn answer(
        request: Request,
        wire: Arc<Wire>,
        failure: serde_json::Value,
        image_labels: serde_json::Value,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let route = request.uri().path().to_owned();
        if route.ends_with("/version") {
            return (
                StatusCode::OK,
                Json(
                    serde_json::to_value(bollard::models::SystemVersion {
                        version: Some("fixture".into()),
                        api_version: Some(bollard::API_DEFAULT_VERSION.to_string()),
                        ..Default::default()
                    })
                    .unwrap(),
                ),
            );
        }
        if route.ends_with("/info") {
            return (
                StatusCode::OK,
                Json(
                    serde_json::to_value(bollard::models::SystemInfo {
                        security_options: Some(vec![ROOTLESS_SECURITY_OPTION.into()]),
                        live_restore_enabled: Some(true),
                        ..Default::default()
                    })
                    .unwrap(),
                ),
            );
        }
        if route.ends_with("/networks/create") {
            wire.bridges.fetch_add(1, Ordering::SeqCst);
            if wire.allow_bridge.load(Ordering::SeqCst) {
                return (
                    StatusCode::CREATED,
                    Json(
                        serde_json::to_value(bollard::models::NetworkCreateResponse {
                            id: "connected-fixture".into(),
                            ..Default::default()
                        })
                        .unwrap(),
                    ),
                );
            }
            return (
                StatusCode::from_u16(failure["statusCode"].as_u64().unwrap() as u16).unwrap(),
                Json(serde_json::json!({"message": failure["message"]})),
            );
        }
        if route.ends_with("/containers/create") {
            return created_container(request, &wire, image_labels).await;
        }
        if route.contains("/networks/") {
            return network_observation(&wire, failure);
        }
        if route.contains("/images/") {
            return (StatusCode::OK, Json(serde_json::json!({"Config": {"Labels": image_labels}})));
        }
        if route.ends_with("/containers/json") {
            return (StatusCode::OK, Json(serde_json::json!([])));
        }
        if route.ends_with("/containers/private-fixture/json")
            && let Some(latest) = wire.latest.lock().unwrap().clone()
        {
            return (StatusCode::OK, Json(latest));
        }
        (StatusCode::NOT_FOUND, Json(serde_json::json!({"message": "fixture resource absent"})))
    }

    async fn created_container(
        request: Request,
        wire: &Wire,
        image_labels: serde_json::Value,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let body: serde_json::Value =
            serde_json::from_slice(&to_bytes(request.into_body(), 64 * 1024).await.unwrap()).unwrap();
        // Retain only non-secret wire policy, never the bootstrap environment.
        wire.creates
            .lock()
            .unwrap()
            .push((body["HostConfig"].clone(), body["Labels"][crate::labels::LABEL_INTERNAL_BINDINGS].clone()));
        // Engine image labels are defaults; create's labels override matching names.
        let mut labels = image_labels;
        labels.as_object_mut().unwrap().extend(body["Labels"].as_object().unwrap().clone());
        let mounts: Vec<_> = body["HostConfig"]["Mounts"].as_array().unwrap().iter().map(|mount| {
                serde_json::json!({"Source": mount["Source"], "Destination": mount["Target"], "RW": !mount["ReadOnly"].as_bool().unwrap_or(false)})
            }).collect();
        *wire.latest.lock().unwrap() = Some(serde_json::json!({"Id": "private-fixture", "Name": "/private-fixture",
                "State": {"Running": true, "Status": "running", "StartedAt": "2026-11-16T00:00:00Z"},
                "Config": {"Labels": labels}, "HostConfig": body["HostConfig"], "Mounts": mounts}));
        (
            StatusCode::CREATED,
            Json(
                serde_json::to_value(bollard::models::ContainerCreateResponse {
                    id: "private-fixture".into(),
                    ..Default::default()
                })
                .unwrap(),
            ),
        )
    }

    fn network_observation(wire: &Wire, failure: serde_json::Value) -> (StatusCode, Json<serde_json::Value>) {
        wire.network_reads.fetch_add(1, Ordering::SeqCst);
        if wire.network_failure.load(Ordering::SeqCst) {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(failure));
        }
        match *wire.network.lock().unwrap() {
            Some(isolated) => (
                StatusCode::OK,
                Json(
                    serde_json::to_value(bollard::models::Network {
                        options: Some(std::collections::HashMap::from([(ICC_OPTION.into(), (!isolated).to_string())])),
                        ..Default::default()
                    })
                    .unwrap(),
                ),
            ),
            None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"message": "fixture resource absent"}))),
        }
    }

    // Trusted metadata only. Actual agent TCP/UDP behavior belongs to the real-engine case.
    fn metadata_agent(directory: &Path, output: &str) -> std::path::PathBuf {
        compile_metadata_agent(
            directory,
            &format!("#include <stdio.h>\nint main(void) {{ puts({output:?}); return 0; }}\n"),
        )
    }

    fn compile_metadata_agent(directory: &Path, code: &str) -> std::path::PathBuf {
        let source = directory.join("metadata.c");
        let agent = directory.join("agent");
        std::fs::write(&source, code).unwrap();
        assert!(
            std::process::Command::new("cc")
                .args(["-static", "-O2"])
                .arg(&source)
                .arg("-o")
                .arg(&agent)
                .status()
                .unwrap()
                .success()
        );
        agent
    }

    fn workload(network: NetworkMode) -> crate::protocol::WorkloadSpec {
        serde_json::from_value(serde_json::json!({"image": "alpine:3.22", "entrypoint": ["true"], "network": network,
            "resources": {"memoryMb": 256, "allowUnenforcedMemory": true}, "storage": {"mountPath": "/data", "sizeGb": 1},
            "ports": [{"name": "http", "containerPort": 8080, "audience": ["control"]}]})).unwrap()
    }

    async fn private_admission(
        directory: &Path,
        agent: &Path,
        ids: Ids,
        routed: bool,
        wire: &Wire,
        original: &str,
    ) -> (Arc<Manager>, Arc<DockerRuntime>) {
        let mut runtime =
            DockerRuntime::new(&directory.join("engine.sock")).unwrap().with_internal_ingress_agent(agent);
        runtime.ownership = ownership::Policy { user: ids, owner: Some(ids) };
        let runtime = Arc::new(runtime);
        let backend: Arc<dyn ContainerRuntime> = if routed {
            Arc::new(crate::runtime::router::Router::new(vec![runtime.clone()]).unwrap())
        } else {
            runtime.clone()
        };
        let config: crate::config::Config = toml::from_str(&format!(
            "deployment_id = \"test\"\nnode_id = \"n\"\nstate_dir = {:?}\n[api]\nlisten = \"127.0.0.1:0\"\n[workloads]\nuser = \"{}:{}\"\ndata_owner = \"{}:{}\"\nagent_binary = {:?}\nallowed_images = [\"alpine:\"]\n[capacity]\nmin_free_disk_mb = 1\n",
            directory.join(if routed {"routed"} else {"single"}), ids.uid, ids.gid, ids.uid, ids.gid, agent)).unwrap();
        let store = crate::store::Store::open(&config.state_dir).unwrap();
        let manager = Manager::new(
            Arc::new(config),
            backend,
            store,
            Arc::new(crate::metrics::Metrics::new()),
            Arc::new(|_, _| true),
        );
        let report = manager.reconcile(Records::FromDisk).await;
        assert!(report.error.is_none(), "{report:?}");
        assert_eq!(
            wire.bridges.load(Ordering::SeqCst),
            usize::from(routed),
            "reconcile must not depend on an unused connected bridge"
        );
        manager
            .ensure(
                crate::ids::WorkloadId::parse(if routed { "routed" } else { "single" }).unwrap(),
                workload(NetworkMode::None),
                Precondition::None,
                None,
            )
            .await
            .unwrap();
        let body = wire.creates.lock().unwrap().last().unwrap().0.clone();
        assert_eq!(body["NetworkMode"], network::DISCONNECTED_NETWORK);
        assert!(body["PortBindings"].as_object().unwrap().is_empty());
        assert_eq!(body["ReadonlyRootfs"], true);
        assert_eq!(body["Privileged"], false);
        assert_eq!(body["CapDrop"], serde_json::json!(["ALL"]));
        assert!(body["SecurityOpt"].as_array().unwrap().iter().any(|value| value == "no-new-privileges:true"));
        let count = wire.creates.lock().unwrap().len();
        let error = manager
            .ensure(
                crate::ids::WorkloadId::parse("internet").unwrap(),
                workload(NetworkMode::Internet),
                Precondition::None,
                None,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains(original), "the original engine refusal must remain visible");
        assert_eq!(wire.creates.lock().unwrap().len(), count, "no internet compute after a bridge-policy refusal");
        existing_network_monitoring(&manager, wire, original).await;
        (manager, runtime)
    }

    async fn existing_network_monitoring(manager: &Arc<Manager>, wire: &Wire, original: &str) {
        let creates = wire.bridges.load(Ordering::SeqCst);
        for isolated in [Some(false), Some(true), None] {
            *wire.network.lock().unwrap() = isolated;
            let reads = wire.network_reads.load(Ordering::SeqCst);
            let source = if isolated == Some(false) { Records::FromDisk } else { Records::InMemory };
            let report = manager.reconcile(source).await;
            assert!(report.error.is_none(), "{report:?}");
            assert!(wire.network_reads.load(Ordering::SeqCst) > reads, "reconciliation must observe existing bridges");
            assert_eq!(
                manager.host_issues().iter().any(|issue| issue.code == crate::protocol::IssueCode::NetworkNotIsolated),
                isolated == Some(false),
                "only an observed isolated bridge or an absent unused bridge clears the host issue"
            );
            assert_eq!(wire.bridges.load(Ordering::SeqCst), creates, "monitoring must not create an unused bridge");
            if isolated == Some(false) {
                let before = wire.creates.lock().unwrap().len();
                let error = manager
                    .ensure(
                        crate::ids::WorkloadId::parse("unsafe-internet").unwrap(),
                        workload(NetworkMode::Internet),
                        Precondition::None,
                        None,
                    )
                    .await
                    .unwrap_err();
                assert!(matches!(&error, crate::manager::NodeError::Runtime(RuntimeError::Other(_))), "{error}");
                assert_eq!(wire.creates.lock().unwrap().len(), before, "unsafe ICC cannot admit internet compute");
                assert_eq!(wire.bridges.load(Ordering::SeqCst), creates, "unsafe ICC cannot be silently replaced");
            }
        }
        wire.network_failure.store(true, Ordering::SeqCst);
        let report = manager.reconcile(Records::InMemory).await;
        assert!(report.error.as_ref().is_some_and(|error| error.contains(original)), "{report:?}");
        assert_eq!(wire.bridges.load(Ordering::SeqCst), creates, "a failed observation cannot create a bridge");
        wire.network_failure.store(false, Ordering::SeqCst);
    }

    async fn connected_and_loss(
        manager: Arc<Manager>,
        runtime: Arc<DockerRuntime>,
        wire: &Wire,
        original: &str,
        forged_port: u16,
        directory: &Path,
        agent: &Path,
    ) {
        wire.allow_bridge.store(true, Ordering::SeqCst);
        manager
            .ensure(
                crate::ids::WorkloadId::parse("connected").unwrap(),
                workload(NetworkMode::Internet),
                Precondition::None,
                None,
            )
            .await
            .unwrap();
        let (connected, declarations) = wire.creates.lock().unwrap().last().unwrap().clone();
        assert_ne!(connected["NetworkMode"], network::DISCONNECTED_NETWORK);
        assert!(!connected["PortBindings"].as_object().unwrap().is_empty());
        assert_eq!(declarations, "[]", "an internet image cannot declare a private host listener");
        runtime.inspect("private-fixture").await.unwrap().unwrap();
        let unclaimed = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, forged_port))
            .expect("a forged image binding must not create a host listener during restore");
        drop(unclaimed);
        assert_eq!(wire.bridges.load(Ordering::SeqCst), 3, "connected create still invokes strict bridge construction");
        assert_eq!(wire.creates.lock().unwrap().len(), 3);
        existing_network_monitoring(&manager, wire, original).await;
        historical_internet_restore(&runtime, wire, forged_port).await;
        assert_eq!(metadata_agent(directory, "{}"), agent);
        assert_eq!(runtime.info().await.unwrap().private_ingress, Some(false));
        let mut legacy = super::super::tests::spec();
        legacy.internal_network = true;
        legacy.allow_unenforced_memory = Some(true);
        legacy.agent = Some(crate::runtime::AgentBoot {
            binary: agent.to_owned(),
            sandbox: "legacy".into(),
            epoch: 0,
            secret_env: "UNUSED_FIXTURE_BOOTSTRAP".into(),
            domain: "fixture.invalid".into(),
            agent_port: 8090,
            spare_ports: Vec::new(),
            keep_alive: false,
            public_origin: None,
        });
        let error = runtime.create(&legacy).await.unwrap_err();
        assert!(error.to_string().contains("network-policy"), "legacy metadata cannot enable disconnected ingress");
        assert_eq!(wire.creates.lock().unwrap().len(), 3);
        wire.allow_bridge.store(false, Ordering::SeqCst);
        let error = manager
            .ensure(
                crate::ids::WorkloadId::parse("agent-lost").unwrap(),
                workload(NetworkMode::Internet),
                Precondition::None,
                None,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains(original), "loss of private support must retain deferred bridge admission");
        assert_eq!(wire.bridges.load(Ordering::SeqCst), 4);
        assert_eq!(wire.creates.lock().unwrap().len(), 3);
    }

    async fn historical_internet_restore(runtime: &DockerRuntime, wire: &Wire, forged_port: u16) {
        let forged = serde_json::to_string(&vec![crate::runtime::PortBinding {
            container_port: 8080,
            host_port: forged_port,
            host_ips: vec![std::net::Ipv4Addr::LOCALHOST.into()],
            protocol: crate::protocol::Proto::Tcp,
        }])
        .unwrap();
        {
            let mut latest = wire.latest.lock().unwrap();
            let latest = latest.as_mut().unwrap();
            latest["Config"]["Labels"][crate::labels::LABEL_INTERNAL_BINDINGS] = forged.clone().into();
            latest["NetworkSettings"] = serde_json::json!({"Networks": {"historical": {"IPAddress": std::net::Ipv4Addr::LOCALHOST.to_string()}}});
        }
        for encoded in [forged.as_str(), "not-json"] {
            wire.latest.lock().unwrap().as_mut().unwrap()["Config"]["Labels"][crate::labels::LABEL_INTERNAL_BINDINGS] =
                encoded.into();
            for rootless in [true, false] {
                let recovered = DockerRuntime::new(Path::new(&runtime.socket)).unwrap();
                *recovered.info.lock().unwrap() = Some(RuntimeInfo { rootless, ..Default::default() });
                recovered.inspect("private-fixture").await.unwrap().unwrap();
                let unclaimed = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, forged_port))
                    .expect("historical image bindings cannot create a listener on an internet sandbox");
                drop(unclaimed);
            }
        }
        // Without pryxd's record the bindings are not pryxd's: nothing listens and nothing fails.
        {
            let mut latest = wire.latest.lock().unwrap();
            let labels = &mut latest.as_mut().unwrap()["Config"]["Labels"];
            labels.as_object_mut().unwrap().remove(crate::labels::LABEL_RECORD);
            labels[crate::labels::LABEL_INTERNAL_BINDINGS] = forged.into();
        }
        let recovered = DockerRuntime::new(Path::new(&runtime.socket)).unwrap();
        *recovered.info.lock().unwrap() = Some(RuntimeInfo::default());
        recovered.inspect("private-fixture").await.unwrap().unwrap();
        drop(
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, forged_port))
                .expect("bindings without pryxd's record cannot create a listener"),
        );
    }
}
