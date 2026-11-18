// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Labels of pryxd's own names that pryxd did not write, on the selected real engine. An image's
//! labels reach every container made from it, so private bindings an image carries, or that a
//! container inherited before pryxd overrode them at create, open no listener on the host.
use super::*;
use pryxd::labels::LABEL_INTERNAL_BINDINGS;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};

const BASE: &str = "alpine:3.22";

/// One private listener as `blocklyd.internal-bindings` holds it (format v1), on loopback.
fn forged(port: u16) -> String {
    serde_json::json!([{
        "container_port": pryxd::examples::HTTP_PORT,
        "protocol": "tcp",
        "host_ips": [Ipv4Addr::LOCALHOST],
        "host_port": port,
    }])
    .to_string()
}

/// A loopback port nothing listens on: the kernel's own choice, released at once.
fn unused_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port()
}

fn assert_nothing_listens(port: u16, container: &str) {
    let reached =
        std::net::TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_secs(2));
    assert!(reached.is_err(), "{container}'s label, which pryxd did not write, opened 127.0.0.1:{port}");
}

fn backend(socket: &std::path::Path) -> bollard::Docker {
    bollard::Docker::connect_with_unix(socket.to_str().unwrap(), 60, bollard::API_DEFAULT_VERSION).unwrap()
}

/// Removes what a case made outside pryxd's own deployment, passed or panicked.
struct Leftovers {
    socket: std::path::PathBuf,
    containers: Vec<String>,
    image: Option<String>,
}
impl Drop for Leftovers {
    fn drop(&mut self) {
        let host = format!("unix://{}", self.socket.display());
        if !self.containers.is_empty() {
            let _ = std::process::Command::new("docker")
                .args(["--host", &host, "rm", "-f"])
                .args(&self.containers)
                .output();
        }
        if let Some(image) = &self.image {
            let _ = std::process::Command::new("docker").args(["--host", &host, "rmi", "-f", image]).output();
        }
    }
}

#[tokio::test]
#[ignore = "needs the actual selected engine"]
async fn engine_inspection_ignores_private_bindings_pryxd_did_not_write() {
    let socket = engine::selected_socket().await;
    let runtime = super::gvisor::engine_runtime(&socket);
    let dep = deployment();
    let backend = backend(&socket);
    runtime.ensure_image(BASE, Duration::from_secs(120)).await.unwrap();
    let names = ["forged", "malformed"].map(|name| format!("{name}-{dep}"));
    let _leftovers = Leftovers { socket: socket.clone(), containers: names.to_vec(), image: None };
    let port = unused_port();
    // As a container made before pryxd overrode an image's label at create, or by anyone else:
    // on an ordinary network, carrying the label its image gave it.
    for (name, label) in names.iter().zip([forged(port), "forged".to_owned()]) {
        let options = bollard::query_parameters::CreateContainerOptionsBuilder::new().name(name).build();
        let body = bollard::models::ContainerCreateBody {
            image: Some(BASE.into()),
            cmd: Some(vec!["sleep".into(), "300".into()]),
            labels: Some(HashMap::from([(LABEL_INTERNAL_BINDINGS.to_owned(), label)])),
            host_config: Some(bollard::models::HostConfig {
                runtime: super::gvisor::on_gvisor().then(|| pryxd::runtime::docker::GVISOR_RUNTIME.to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        };
        backend.create_container(Some(options), body).await.unwrap();
        backend.start_container(name, None).await.unwrap();
        let seen = runtime.inspect(name).await.unwrap_or_else(|error| panic!("{name} is inspected: {error}"));
        assert!(seen.is_some(), "{name} is this backend's container");
        assert_nothing_listens(port, name);
    }
}

#[tokio::test]
#[ignore = "needs the actual selected engine"]
async fn engine_an_image_carrying_private_bindings_gives_its_sandbox_none() {
    let socket = engine::selected_socket().await;
    let runtime = Arc::new(super::gvisor::engine_runtime(&socket));
    let Some(limits) = engine::limits_or_mapping_refusal(&runtime, &socket).await else { return };
    let dep = deployment();
    let backend = backend(&socket);
    runtime.ensure_image(BASE, Duration::from_secs(120)).await.unwrap();
    let port = unused_port();
    let (repository, source) = (format!("pryx-forged-{dep}"), format!("source-{dep}"));
    let image = format!("{repository}:1");
    let _leftovers = Leftovers { socket: socket.clone(), containers: vec![source.clone()], image: Some(image.clone()) };
    // The image: the base, committed from a container that carries the label.
    let options = bollard::query_parameters::CreateContainerOptionsBuilder::new().name(&source).build();
    let body = bollard::models::ContainerCreateBody {
        image: Some(BASE.into()),
        cmd: Some(vec!["true".into()]),
        labels: Some(HashMap::from([(LABEL_INTERNAL_BINDINGS.to_owned(), forged(port))])),
        ..Default::default()
    };
    backend.create_container(Some(options), body).await.unwrap();
    let options = bollard::query_parameters::CommitContainerOptionsBuilder::new()
        .container(&source)
        .repo(&repository)
        .tag("1")
        .pause(false)
        .build();
    backend.commit_container(options, bollard::models::ContainerConfig::default()).await.unwrap();
    let carried =
        backend.inspect_image(&image).await.unwrap().config.and_then(|config| config.labels).unwrap_or_default();
    assert_eq!(carried.get(LABEL_INTERNAL_BINDINGS), Some(&forged(port)), "the image carries the label");
    let directory = tempfile::tempdir().unwrap();
    let _cleanup = engine::EngineCleanup { socket: socket.clone(), deployment: dep.clone() };
    let manager = manager_on(directory.path(), runtime.clone(), &engine::configuration(&dep, limits.rootless));
    let report = manager.reconcile(Records::FromDisk).await;
    assert!(report.error.is_none(), "{report:?}");
    let mut value = support::spec_json();
    value["image"] = image.clone().into();
    value["resources"] = serde_json::json!({"memoryMb": 256, "allowUnenforcedMemory": true});
    value["entrypoint"] = serde_json::json!(["sleep", "300"]);
    manager.ensure(id("forged"), serde_json::from_value(value).unwrap(), Precondition::None, None).await.unwrap();
    manager.start(id("forged"), None).await.unwrap();
    let options = bollard::query_parameters::ListContainersOptionsBuilder::new()
        .all(true)
        .filters(&HashMap::from([("label".to_owned(), vec![format!("{LABEL_DEPLOYMENT}={dep}")])]))
        .build();
    let containers = backend.list_containers(Some(options)).await.unwrap();
    let [container] = containers.as_slice() else { panic!("one sandbox: {containers:?}") };
    let container = container.id.clone().unwrap();
    assert!(runtime.inspect(&container).await.unwrap().is_some());
    assert_nothing_listens(port, &container);
    let labels = backend.inspect_container(&container, None).await.unwrap().config.and_then(|config| config.labels);
    assert_eq!(
        labels.unwrap_or_default().get(LABEL_INTERNAL_BINDINGS).map(String::as_str),
        Some("[]"),
        "pryxd writes the label over the image's"
    );
    cleanup(&manager, &["forged"]).await;
}
