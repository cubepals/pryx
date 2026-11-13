// SPDX-License-Identifier: FSL-1.1-ALv2
//! Daemonless OCI image preparation through the maintained registry client and EROFS builder.
//! Layers stay archives until the external builder consumes them; image paths never enter the
//! host tree. Only whole, digest-verified outputs are published, under a private builder cache.
use super::{
    error,
    filesystems::{FilesystemTools, sha256},
};
use crate::runtime::RuntimeError;
use axum::http::StatusCode;
use futures_util::TryStreamExt;
use oci_client::errors::{OciDistributionError, OciErrorCode};
use oci_client::{Client, Reference, config::ConfigFile, manifest::OciDescriptor, secrets::RegistryAuth};
use oci_spec::image::{Arch, Digest, DigestAlgorithm, Os};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// Private acquisition bounds, not OCI protocol limits or sandbox resource guarantees.
const MAX_LAYER_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_CONFIG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LAYERS: usize = 128;
const READ_ATTEMPTS: usize = 3;

fn retryable(failure: &OciDistributionError) -> bool {
    let transient = |code: StatusCode| {
        code.is_server_error() || matches!(code, StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS)
    };
    match failure {
        OciDistributionError::RequestError(error) => {
            error.is_connect() || error.is_timeout() || error.status().is_some_and(transient)
        }
        OciDistributionError::ServerError { code, .. } => StatusCode::from_u16(*code).is_ok_and(transient),
        OciDistributionError::RegistryError { envelope, .. } => {
            envelope.errors.iter().any(|error| error.code == OciErrorCode::Toomanyrequests)
        }
        _ => false,
    }
}

async fn registry<T, F, Fut>(mut read: F) -> Result<T, RuntimeError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, OciDistributionError>>,
{
    for attempt in 0..READ_ATTEMPTS {
        match read().await {
            Ok(value) => return Ok(value),
            Err(failure) if attempt + 1 < READ_ATTEMPTS && retryable(&failure) => {
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
            }
            Err(OciDistributionError::RequestError(failure)) => return Err(error(failure.without_url())),
            Err(failure) => return Err(error(failure)),
        }
    }
    unreachable!("Bounded registry reads return on their last attempt")
}

pub struct PreparedImage {
    pub digest: String,
    /// OCI order, oldest layer first. The guest reverses this for overlayfs lowerdir.
    pub layers: Vec<PathBuf>,
    pub configuration: ConfigFile,
}

pub struct Images {
    root: PathBuf,
    tools: FilesystemTools,
    client: Client,
    transition: tokio::sync::Mutex<()>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CachedLayer {
    source: String,
    diff_id: String,
    image_sha256: String,
    image_bytes: u64,
}

fn digest_name(value: &str) -> Result<String, RuntimeError> {
    let digest: Digest = value.parse().map_err(error)?;
    if !matches!(digest.algorithm(), DigestAlgorithm::Sha256 | DigestAlgorithm::Sha512) {
        return Err(error("VM image cache requires an OCI SHA-256 or SHA-512 digest"));
    }
    Ok(format!("{}-{}", digest.algorithm(), digest.digest()))
}

fn directory(path: &Path) -> Result<(), RuntimeError> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(problem) if problem.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(problem) => return Err(error(problem)),
    }
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    if !metadata.is_dir() || metadata.mode() & 0o077 != 0 || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(error("VM image cache must be a private owned directory"));
    }
    Ok(())
}

impl Images {
    /// Call only after the manager's allowed-image admission. No workload can choose this root.
    pub fn new(root: PathBuf, tools: FilesystemTools) -> Result<Self, RuntimeError> {
        directory(&root)?;
        let root = root.join(tools.identity());
        directory(&root)?;
        let client = Client::try_from(oci_client::client::ClientConfig {
            read_timeout: Some(Duration::from_secs(30)),
            connect_timeout: Some(Duration::from_secs(10)),
            ..Default::default()
        })
        .map_err(error)?;
        Ok(Self { root, tools, client, transition: Default::default() })
    }

    pub async fn prepare(&self, image: &str, timeout: Duration) -> Result<PreparedImage, RuntimeError> {
        tokio::time::timeout(timeout, async {
            let _guard = self.transition.lock().await;
            self.pull(image).await
        })
        .await
        .map_err(|_| RuntimeError::Timeout("VM OCI image preparation exceeded its deadline".into()))?
    }

    async fn pull(&self, image: &str) -> Result<PreparedImage, RuntimeError> {
        let reference: Reference = image.parse().map_err(error)?;
        let (manifest, digest) =
            registry(|| self.client.pull_image_manifest(&reference, &RegistryAuth::Anonymous)).await?;
        digest_name(&digest)?;
        if manifest.layers.len() > MAX_LAYERS {
            return Err(error("VM OCI image exceeds the layer admission bound"));
        }
        let staging = tempfile::tempdir_in(&self.root).map_err(error)?;
        let config = staging.path().join("configuration");
        self.blob(&reference, &manifest.config, &config, MAX_CONFIG_BYTES).await?;
        let configuration: ConfigFile = serde_json::from_reader(File::open(config).map_err(error)?).map_err(error)?;
        if configuration.architecture != Arch::Amd64 || configuration.os != Os::Linux {
            return Err(error("The published VM inputs require an amd64 Linux image; no emulation is available"));
        }
        if configuration.rootfs.diff_ids.len() != manifest.layers.len() {
            return Err(error("OCI image rootfs digest count differs from its layer count"));
        }
        let mut layers = Vec::new();
        for (index, (layer, diff_id)) in manifest.layers.iter().zip(&configuration.rootfs.diff_ids).enumerate() {
            compressed(&layer.media_type)?;
            let name = digest_name(&layer.digest)?;
            digest_name(diff_id)?;
            let path = self.root.join(format!("{name}.erofs"));
            let record = self.root.join(format!("{name}.json"));
            if Self::cached(&path, &record, &layer.digest, diff_id)? {
                layers.push(path);
                continue;
            }
            let blob = staging.path().join(format!("layer-{index}.blob"));
            self.blob(&reference, layer, &blob, MAX_LAYER_BYTES).await?;
            let temporary = tempfile::tempdir_in(staging.path()).map_err(error)?;
            let work = temporary.path().to_path_buf();
            let generated = convert(&self.tools, &blob, &work, &layer.media_type, diff_id).await?;
            let file = File::open(&generated).map_err(error)?;
            let saved = CachedLayer {
                source: layer.digest.clone(),
                diff_id: diff_id.clone(),
                image_bytes: file.metadata().map_err(error)?.len(),
                image_sha256: sha256(&file).map_err(error)?,
            };
            file.sync_all().map_err(error)?;
            fs::set_permissions(&generated, fs::Permissions::from_mode(0o444)).map_err(error)?;
            // A missing record never authenticates an existing image. A completed record
            // follows the synced image; cancellation or failure cannot publish partial data.
            fs::rename(generated, &path).map_err(error)?;
            crate::durable::write_atomic(&record, &serde_json::to_vec(&saved).map_err(error)?, 0o600).map_err(error)?;
            layers.push(path);
        }
        Ok(PreparedImage { digest, layers, configuration })
    }

    fn cached(path: &Path, record: &Path, source: &str, diff_id: &str) -> Result<bool, RuntimeError> {
        let saved = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(record) {
            Ok(file) => {
                if !file.metadata().map_err(error)?.is_file() {
                    return Err(error("VM layer cache record must be regular"));
                }
                const MAX_RECORD_BYTES: u64 = 4096;
                let mut bytes = Vec::new();
                file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes).map_err(error)?;
                if bytes.len() as u64 > MAX_RECORD_BYTES {
                    return Err(error("Oversized VM layer cache record"));
                }
                serde_json::from_slice::<CachedLayer>(&bytes).map_err(error)?
            }
            Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(problem) => return Err(error(problem)),
        };
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(error)?;
        let metadata = file.metadata().map_err(error)?;
        if saved.source != source
            || saved.diff_id != diff_id
            || !metadata.is_file()
            || metadata.mode() & 0o222 != 0
            || metadata.len() != saved.image_bytes
            || sha256(file).map_err(error)? != saved.image_sha256
        {
            return Err(RuntimeError::ImageCacheInvalid);
        }
        Ok(true)
    }

    async fn blob(
        &self,
        image: &Reference,
        layer: &OciDescriptor,
        path: &Path,
        maximum: u64,
    ) -> Result<(), RuntimeError> {
        let expected = u64::try_from(layer.size).map_err(error)?;
        if expected > maximum {
            return Err(error("OCI blob exceeds the VM acquisition bound"));
        }
        for address in layer.urls.iter().flatten() {
            let parsed = reqwest::Url::parse(address).map_err(error)?;
            if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
                return Err(error("VM OCI external blob addresses require credential-free HTTPS"));
            }
        }
        let mut stream = registry(|| self.client.pull_blob_stream(image, layer)).await?;
        if stream.content_length.is_some_and(|length| length != expected) {
            return Err(error("OCI blob response length differs from its descriptor"));
        }
        let mut file =
            tokio::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).await.map_err(error)?;
        let mut received = 0_u64;
        // Consume the verifying stream through EOF: a truncated or corrupt final digest
        // is the client's real error, never a successful partial cache entry.
        while let Some(bytes) = stream.try_next().await.map_err(error)? {
            received = received.checked_add(bytes.len() as u64).ok_or_else(|| error("OCI blob length overflow"))?;
            if received > expected {
                return Err(error("OCI blob exceeds its declared size"));
            }
            file.write_all(&bytes).await.map_err(error)?;
        }
        if received != expected {
            return Err(error("OCI blob is shorter than its declared size"));
        }
        file.sync_all().await.map_err(error)?;
        Ok(())
    }
}

fn compressed(media: &str) -> Result<bool, RuntimeError> {
    use oci_client::manifest::{
        IMAGE_DOCKER_LAYER_GZIP_MEDIA_TYPE, IMAGE_DOCKER_LAYER_TAR_MEDIA_TYPE, IMAGE_LAYER_GZIP_MEDIA_TYPE,
        IMAGE_LAYER_MEDIA_TYPE,
    };
    match media {
        IMAGE_LAYER_GZIP_MEDIA_TYPE | IMAGE_DOCKER_LAYER_GZIP_MEDIA_TYPE => Ok(true),
        IMAGE_LAYER_MEDIA_TYPE | IMAGE_DOCKER_LAYER_TAR_MEDIA_TYPE => Ok(false),
        _ => Err(error(format!("Unsupported VM OCI layer media type: {media}"))),
    }
}

fn expand(blob: &Path, work: &Path, media: &str, diff_id: &str) -> Result<PathBuf, RuntimeError> {
    let source = File::open(blob).map_err(error)?;
    let reader: Box<dyn Read> =
        if compressed(media)? { Box::new(flate2::read::MultiGzDecoder::new(source)) } else { Box::new(source) };
    let tar = work.join("layer.tar");
    let mut output = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tar).map_err(error)?;
    if std::io::copy(&mut reader.take(MAX_LAYER_BYTES + 1), &mut output).map_err(error)? > MAX_LAYER_BYTES {
        return Err(error("Expanded OCI layer exceeds the VM image acquisition bound"));
    }
    output.flush().map_err(error)?;
    let digest: Digest = diff_id.parse().map_err(error)?;
    let matches = match digest.algorithm() {
        DigestAlgorithm::Sha256 => sha256(File::open(&tar).map_err(error)?).map_err(error)? == digest.digest(),
        DigestAlgorithm::Sha512 => {
            use sha2::{Digest as _, Sha512};
            let mut source = File::open(&tar).map_err(error)?;
            let mut hash = Sha512::new();
            let mut buffer = [0; 64 * 1024];
            loop {
                let length = source.read(&mut buffer).map_err(error)?;
                if length == 0 {
                    break;
                }
                hash.update(&buffer[..length]);
            }
            hex::encode(hash.finalize()) == digest.digest()
        }
        _ => false,
    };
    if !matches {
        return Err(error("Uncompressed OCI layer differs from its rootfs diff_id"));
    }
    Ok(tar)
}

async fn convert(
    tools: &FilesystemTools,
    blob: &Path,
    work: &Path,
    media: &str,
    diff_id: &str,
) -> Result<PathBuf, RuntimeError> {
    let (input, output, media, diff_id) =
        (blob.to_path_buf(), work.to_path_buf(), media.to_owned(), diff_id.to_owned());
    let tar = tokio::task::spawn_blocking(move || expand(&input, &output, &media, &diff_id)).await.map_err(error)??;
    let image = work.join("layer.erofs");
    let mut builder = tools.builder("mkfs.erofs");
    builder
        .args(["--quiet", "--tar=f", "--aufs", "--ovlfs-strip=0"])
        .arg(&image)
        .arg(&tar)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = tokio::process::Command::from(builder).kill_on_drop(true).spawn().map_err(error)?;
    const MAX_DIAGNOSTIC_BYTES: u64 = 32 * 1024;
    let mut diagnostic = Vec::new();
    child
        .stderr
        .take()
        .ok_or_else(|| error("VM filesystem diagnostic pipe is missing"))?
        .take(MAX_DIAGNOSTIC_BYTES + 1)
        .read_to_end(&mut diagnostic)
        .await
        .map_err(error)?;
    if diagnostic.len() as u64 > MAX_DIAGNOSTIC_BYTES {
        child.kill().await.map_err(error)?;
        return Err(error("VM EROFS builder diagnostic exceeds its bound"));
    }
    let status = child.wait().await.map_err(error)?;
    if !status.success() {
        return Err(error(format!("VM EROFS builder failed: {status}: {}", String::from_utf8_lossy(&diagnostic))));
    }
    Ok(image)
}
