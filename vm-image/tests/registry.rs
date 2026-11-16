// SPDX-License-Identifier: FSL-1.1-ALv2
//! Real public OCI registries and the signature-verified filesystem component, without an
//! engine or root. The optional VM workflow stages these inputs and runs this acceptance.
use pryx_vm_image::image::Images;
use pryxd::runtime::{
    RuntimeError,
    vm::{
        filesystems::{FilesystemTools, sha256},
        image::{self, Request},
    },
};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::PathBuf,
    time::Duration,
};

#[tokio::test]
#[ignore = "requires the verified VM filesystem component; the optional VM workflow runs it"]
async fn real_oci_layers_are_verified_cached_and_corruption_is_refused() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let component =
        PathBuf::from(std::env::var_os("PRYX_VM_FILESYSTEM_COMPONENT").expect("Stage the VM filesystem component"));
    let manifest = std::env::var("PRYX_VM_FILESYSTEM_MANIFEST").expect("Pin the staged component manifest");
    // The VM workflow stages the bundle, and so its launcher, beside the component.
    let launcher = std::env::var_os("PRYX_VM_HOST_LAUNCHER")
        .map(PathBuf::from)
        .unwrap_or_else(|| component.parent().unwrap().join("vm-runtime/bin/vm-launcher"));
    let tools = FilesystemTools::verified(&component, &manifest, &launcher).unwrap();
    let staging = tempfile::tempdir().unwrap();
    let alias = staging.path().join("component-link");
    std::os::unix::fs::symlink(&component, &alias).unwrap();
    assert!(FilesystemTools::verified(&alias, &manifest, &launcher).is_err());
    let cache = staging.path().join("images");
    let images = Images::new(cache.clone(), tools.clone()).unwrap();
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_pryx-vm-image"));
    let mut observations = Vec::new();
    let inputs: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../templates/vm-tools/image-inputs.json")).unwrap();
    for (index, input) in inputs.iter().enumerate() {
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
        // A pinned digest prepares from its complete cache alone: a moved or unreachable name
        // cannot change the lower layers or block a wake.
        let offline = images
            .prepare_pinned("registry.invalid/pryx/unreachable:moved", Some(&image.digest), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!((&offline.digest, &offline.layers), (&image.digest, &image.layers));
        // A reference naming its own digest is immutable, so it is served offline as well.
        let named = reference.rsplit_once('@').expect("inputs name their digests").1;
        let renamed = images
            .prepare(&format!("registry.invalid/pryx/unreachable@{named}"), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!((&renamed.digest, &renamed.layers), (&image.digest, &image.layers));
        for (path, expected) in again.layers.iter().zip(&before) {
            let metadata = fs::metadata(path).unwrap();
            assert_eq!((metadata.len(), metadata.modified().unwrap()), *expected);
        }
        let request = Request {
            component: component.clone(),
            manifest_sha256: manifest.clone(),
            launcher: launcher.clone(),
            cache: cache.clone(),
            image: reference.into(),
            pinned: None,
            timeout_seconds: 120,
            free_floor_bytes: 0,
            keep: Vec::new(),
            evict: false,
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
        let lock = fs::OpenOptions::new().read(true).write(true).open(images.lock_path()).unwrap();
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
        let short = Request { timeout_seconds: 1, ..request };
        assert!(matches!(image::prepare(&helper, &short).await, Err(RuntimeError::Timeout(_))));
        drop(lock);
        let request = Request { timeout_seconds: 120, ..short };
        assert_eq!(image::prepare(&helper, &request).await.unwrap().layers, image.layers);
        let fresh = Images::new(staging.path().join(format!("fresh-{index}")), tools.clone())
            .unwrap()
            .prepare(reference, Duration::from_secs(120))
            .await
            .unwrap();
        assert_eq!(image.layers.len(), fresh.layers.len());
        for (original, rebuilt) in image.layers.iter().zip(&fresh.layers) {
            assert_eq!(
                sha256(fs::File::open(original).unwrap()).unwrap(),
                sha256(fs::File::open(rebuilt).unwrap()).unwrap(),
                "An independently rebuilt lower must preserve every byte and its origin identity"
            );
        }
        observations.push(serde_json::json!({"image": reference, "manifestDigest": image.digest,
            "layers": before.iter().map(|(bytes, _)| *bytes).collect::<Vec<_>>(), "cacheReused": true, "independentRebuildIdentical": true}));
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
    // A VM keeps its manifest's layers through an eviction for another image, and through the
    // host's floor, and they serve offline; with nothing keeping them, they go.
    let cache = staging.path().join("eviction");
    let request = |reference: &str, keep: Vec<String>, evict: bool, free_floor_bytes: u64| Request {
        component: component.clone(),
        manifest_sha256: manifest.clone(),
        launcher: launcher.clone(),
        cache: cache.clone(),
        image: reference.into(),
        pinned: None,
        timeout_seconds: 120,
        free_floor_bytes,
        keep,
        evict,
    };
    let (kept_reference, other_reference) =
        (inputs[0]["reference"].as_str().unwrap(), inputs[1]["reference"].as_str().unwrap());
    let kept = image::prepare(&helper, &request(kept_reference, Vec::new(), false, 0)).await.unwrap();
    let identities = |layers: &[PathBuf]| {
        layers.iter().map(|layer| fs::metadata(layer).map(|info| (info.dev(), info.ino())).ok()).collect::<Vec<_>>()
    };
    let before = identities(&kept.layers);
    assert!(before.iter().all(Option::is_some));
    let other = image::prepare(&helper, &request(other_reference, vec![kept.digest.clone()], true, 0)).await.unwrap();
    assert_eq!(identities(&kept.layers), before, "A kept manifest lost a layer");
    let offline = Request {
        pinned: Some(kept.digest.clone()),
        ..request("registry.invalid/pryx/unreachable:moved", Vec::new(), false, 0)
    };
    assert_eq!(image::prepare(&helper, &offline).await.unwrap().layers, kept.layers);
    image::prepare(&helper, &request(other_reference, Vec::new(), true, 0)).await.unwrap();
    assert!(identities(&kept.layers).iter().all(Option::is_none), "Layers no VM keeps were kept");
    assert!(identities(&other.layers).iter().all(Option::is_some));
    // Past the floor nothing is fetched, even once nothing else is left to remove.
    let refused = match image::prepare(&helper, &request(kept_reference, Vec::new(), false, u64::MAX)).await {
        Ok(_) => panic!("An image was fetched past the host's floor"),
        Err(problem) => problem.to_string(),
    };
    assert!(refused.contains("free-space floor"), "{refused}");
    assert!(identities(&kept.layers).iter().all(Option::is_none));
    observations
        .push(serde_json::json!({"eviction": "kept manifest kept and served offline; unkept removed; floor refused"}));
    if let Some(path) = std::env::var_os("PRYX_VM_IMAGE_REPORT") {
        fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({"scope": "Real OCI preparation only; no production VM boot", "status": "success", "images": observations})).unwrap()).unwrap();
    }
}
