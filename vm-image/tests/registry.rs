// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real public OCI registries and the signature-verified filesystem component, without an
//! engine or root. The optional VM workflow stages these inputs and runs this acceptance.
use pryx_vm_image::image::Images;
use pryxd::runtime::{
    RuntimeError,
    vm::{
        filesystems::FilesystemTools,
        image::{self, Request},
    },
};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

#[tokio::test]
#[ignore = "requires the verified VM filesystem component; the optional VM workflow runs it"]
async fn real_oci_layers_are_verified_cached_and_corruption_is_refused() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let component =
        PathBuf::from(std::env::var_os("PRYX_VM_FILESYSTEM_COMPONENT").expect("Stage the VM filesystem component"));
    let manifest = std::env::var("PRYX_VM_FILESYSTEM_MANIFEST").expect("Pin the staged component manifest");
    let tools = FilesystemTools::verified(&component, &manifest).unwrap();
    let staging = tempfile::tempdir().unwrap();
    let alias = staging.path().join("component-link");
    std::os::unix::fs::symlink(&component, &alias).unwrap();
    assert!(FilesystemTools::verified(&alias, &manifest).is_err());
    let cache = staging.path().join("images");
    let images = Images::new(cache.clone(), tools).unwrap();
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_pryx-vm-image"));
    let mut observations = Vec::new();
    let inputs: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../templates/vm-tools/image-inputs.json")).unwrap();
    for input in inputs {
        let reference = input["reference"].as_str().unwrap();
        let image = images.prepare(reference, Duration::from_secs(120)).await.unwrap();
        assert!(!image.layers.is_empty());
        let before = image
            .layers
            .iter()
            .map(|path| {
                let metadata = fs::metadata(path).unwrap();
                assert!(metadata.is_file() && metadata.len() > 0);
                assert_eq!(metadata.permissions().mode() & 0o222, 0);
                (metadata.len(), metadata.modified().unwrap())
            })
            .collect::<Vec<_>>();
        let again = images.prepare(reference, Duration::from_secs(120)).await.unwrap();
        assert_eq!(image.digest, again.digest);
        assert_eq!(image.layers, again.layers);
        for (path, expected) in again.layers.iter().zip(&before) {
            let metadata = fs::metadata(path).unwrap();
            assert_eq!((metadata.len(), metadata.modified().unwrap()), *expected);
        }
        let request = Request {
            component: component.clone(),
            manifest_sha256: manifest.clone(),
            cache: cache.clone(),
            image: reference.into(),
            timeout_seconds: 120,
        };
        let prepared = image::prepare(&helper, &request).await.unwrap();
        assert_eq!(prepared.digest, image.digest);
        assert_eq!(prepared.layers, image.layers);
        assert_eq!(
            serde_json::to_value(&prepared.configuration).unwrap(),
            serde_json::to_value(
                serde_json::from_value::<oci_spec::image::ImageConfiguration>(
                    serde_json::to_value(&image.configuration).unwrap()
                )
                .unwrap()
            )
            .unwrap()
        );
        // A second process must wait for the same kernel cache lock, not publish
        // layer bytes against a different process's completed record.
        let lock =
            fs::OpenOptions::new().read(true).write(true).open(cache.join(&manifest).join("prepare.lock")).unwrap();
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
        let short = Request { timeout_seconds: 1, ..request };
        assert!(matches!(image::prepare(&helper, &short).await, Err(RuntimeError::Timeout(_))));
        drop(lock);
        let request = Request { timeout_seconds: 120, ..short };
        assert_eq!(image::prepare(&helper, &request).await.unwrap().layers, image.layers);
        observations.push(serde_json::json!({"image": reference, "manifestDigest": image.digest,
            "layers": before.iter().map(|(bytes, _)| *bytes).collect::<Vec<_>>(), "cacheReused": true}));
        let corrupt = &image.layers[0];
        fs::set_permissions(corrupt, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(corrupt, b"corrupt cache").unwrap();
        fs::set_permissions(corrupt, fs::Permissions::from_mode(0o444)).unwrap();
        assert!(matches!(
            images.prepare(reference, Duration::from_secs(120)).await,
            Err(RuntimeError::ImageCacheInvalid)
        ));
        assert!(matches!(image::prepare(&helper, &request).await, Err(RuntimeError::ImageCacheInvalid)));
        fs::remove_file(corrupt).unwrap();
        // This test has deliberately poisoned a complete record. Remove that private
        // record before moving to another image, rather than repairing a bad cache silently.
        fs::remove_file(corrupt.with_extension("json")).unwrap();
    }
    if let Some(path) = std::env::var_os("PRYX_VM_IMAGE_REPORT") {
        fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({"scope": "Real OCI preparation only; no production VM boot", "status": "success", "images": observations})).unwrap()).unwrap();
    }
}
