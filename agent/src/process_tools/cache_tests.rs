// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real HTTP and maintained gzip/tar exercise the native acquisition boundary. These small
//! fixtures test integrity/publication; real upstream tool acceptance runs separately in CI.
use super::*;
use std::{
    fs,
    io::Cursor,
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicUsize, Ordering},
};

fn bundle(external_link: bool) -> (Vec<u8>, String, u64, u64) {
    let body = b"A verified immutable fixture\n";
    let inventory = serde_json::to_vec(&serde_json::json!({"files":[
        {"path":"tool.txt","sha256":cache::hash(Cursor::new(body)).unwrap()}
    ]}))
    .unwrap();
    let digest = cache::hash(Cursor::new(&inventory)).unwrap();
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
    for (path, bytes) in [("tool.txt", body.as_slice()), ("manifest.json", inventory.as_slice())] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, path, bytes).unwrap();
    }
    if external_link {
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(tar::EntryType::Symlink);
        archive.append_link(&mut header, "outside", "/tmp/pryx-foreign-target").unwrap();
    }
    let bytes = archive.into_inner().unwrap().finish().unwrap();
    (bytes, digest, (body.len() + inventory.len()) as u64, if external_link { 3 } else { 2 })
}

struct Fixture {
    policy: Policy,
    calls: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
    _root: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        for entry in fs::read_dir(&self.policy.cache).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
    }
}
impl Fixture {
    async fn new(external_link: bool) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (bytes, manifest_sha256, unpacked_bytes, entries) = bundle(external_link);
        let digest = cache::hash(Cursor::new(&bytes)).unwrap();
        let length = bytes.len() as u64;
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let router = axum::Router::new().route(
            "/bundle",
            axum::routing::get(move || {
                let body = bytes.clone();
                observed.fetch_add(1, Ordering::SeqCst);
                async move { body }
            }),
        );
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let url = format!("http://{}/bundle", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let root = tempfile::tempdir().unwrap();
        let policy = Policy {
            cache: root.path().join("cache"),
            downloads: root.path().join("downloads"),
            scratch: root.path().join("scratch"),
            browser_on: false,
            unavailable: None,
            artifact: Some(Artifact { url, sha256: digest, manifest_sha256, bytes: length, unpacked_bytes, entries }),
        };
        for path in [&policy.cache, &policy.downloads, &policy.scratch] {
            fs::create_dir(path).unwrap()
        }
        Self { policy, calls, server, _root: root }
    }
}

#[tokio::test]
async fn acquisition_is_lazy_verified_and_reuses_only_a_complete_immutable_cache() {
    let fixture = Fixture::new(false).await;
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    let root = cache::prepare(&fixture.policy).await.unwrap();
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read(root.join("tool.txt")).unwrap(), b"A verified immutable fixture\n");
    assert_eq!(fs::metadata(&root).unwrap().permissions().mode() & 0o222, 0);
    assert_eq!(cache::prepare(&fixture.policy).await.unwrap(), root);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(root.join("injected.py"), b"untrusted extra module").unwrap();
    assert!(cache::prepare(&fixture.policy).await.is_err());
}

#[tokio::test]
async fn incorrect_digests_bounds_and_external_links_never_publish_a_cache() {
    for mode in ["archive-digest", "inventory-digest", "entry-count", "expanded-bytes", "external-link"] {
        let mut fixture = Fixture::new(mode == "external-link").await;
        let artifact = fixture.policy.artifact.as_mut().unwrap();
        match mode {
            "archive-digest" => artifact.sha256 = "0".repeat(64),
            "inventory-digest" => artifact.manifest_sha256 = "0".repeat(64),
            "entry-count" => artifact.entries = 1,
            "expanded-bytes" => artifact.unpacked_bytes = 1,
            _ => (),
        }
        assert!(cache::prepare(&fixture.policy).await.is_err(), "{mode}");
        assert_eq!(fs::read_dir(&fixture.policy.cache).unwrap().count(), 0, "{mode}");
        assert_eq!(fs::read_dir(&fixture.policy.downloads).unwrap().count(), 0, "{mode}");
    }
}
