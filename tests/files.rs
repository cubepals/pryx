// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Node file access over real mutual TLS while compute is stopped, with exact epochs and the
//! same descriptor walk used in the agent. No Docker-backed suite runs in these tests.

mod support;

use hyper::StatusCode;
use pryxd::manager::Precondition;
use support::{CLIENT, client, fixture, id, pki, request, serve, spec};

#[tokio::test]
async fn stopped_files_stream_with_exact_epochs_and_refuse_links() {
    let f = fixture("").await;
    let key = id("files");
    f.manager.ensure(key.clone(), spec(), Precondition::None, Some(7)).await.unwrap();
    let pki = pki(f.dir.path());
    let me = client(&pki.ca, CLIENT);
    let server = serve(f.manager.clone()).await;
    let path = "/v1/workloads/files/files/content?path=src/blob";
    let bytes = vec![0x42; 3 * 1024 * 1024];
    let result =
        request(server.addr, &pki.ca.pem, Some(&me), "PUT", path, &[("blocklyd-epoch", "6")], Some(bytes.clone()))
            .await
            .unwrap();
    assert_eq!(result.status, StatusCode::CONFLICT);
    let result =
        request(server.addr, &pki.ca.pem, Some(&me), "PUT", path, &[("blocklyd-epoch", "7")], Some(bytes.clone()))
            .await
            .unwrap();
    assert_eq!(result.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&result.body));
    use std::os::unix::fs::MetadataExt;
    let owner = f.manager.config.data_owner_ids();
    for path in ["src", "src/blob"] {
        let metadata = std::fs::metadata(f.manager.store.data_dir(&key).join(path)).unwrap();
        assert_eq!((metadata.uid(), metadata.gid()), owner);
    }
    let result =
        request(server.addr, &pki.ca.pem, Some(&me), "GET", path, &[("blocklyd-epoch", "7")], None).await.unwrap();
    assert_eq!(result.status, StatusCode::OK);
    assert_eq!(result.body, bytes);
    let result = request(
        server.addr,
        &pki.ca.pem,
        Some(&me),
        "GET",
        "/v1/workloads/files/files/list?path=src",
        &[("blocklyd-epoch", "7")],
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.json()["files"][0]["size"], bytes.len());
    std::os::unix::fs::symlink("/etc", f.manager.store.data_dir(&key).join("outside")).unwrap();
    let result = request(
        server.addr,
        &pki.ca.pem,
        Some(&me),
        "GET",
        "/v1/workloads/files/files/content?path=outside/passwd",
        &[("blocklyd-epoch", "7")],
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.status, StatusCode::UNPROCESSABLE_ENTITY);
    let result = request(
        server.addr,
        &pki.ca.pem,
        Some(&me),
        "GET",
        "/v1/workloads/files/files/stat?path=missing",
        &[("blocklyd-epoch", "7")],
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.status, StatusCode::NOT_FOUND);
    assert_eq!(result.json()["error"]["code"], "file_not_found");
    assert_eq!(f.manager.view(&key).unwrap().state, pryxd::protocol::WorkloadState::Created);
}
