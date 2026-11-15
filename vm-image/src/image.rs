// SPDX-License-Identifier: FSL-1.1-ALv2
//! Daemonless OCI image preparation through the maintained registry client and EROFS builder.
//! Layers stay archives until the external builder consumes them; image paths never enter the
//! host tree. Only whole, digest-verified outputs are published, under a private builder cache.
use super::{
    error,
    filesystems::{FilesystemTools, Run, sha256},
};
use axum::http::StatusCode;
use futures_util::TryStreamExt;
use oci_client::errors::{OciDistributionError, OciErrorCode};
use oci_client::{Client, Reference, config::ConfigFile, manifest::OciDescriptor, secrets::RegistryAuth};
use oci_spec::image::{Arch, Digest, DigestAlgorithm, Os};
use pryxd::runtime::RuntimeError;
use rustix::fs::{FlockOperation, flock};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;

// Private cache namespace for deterministic lower identity; legacy cache bytes stay intact.
const CONVERSION: &str = "erofs-diff-id-v1";
// Private acquisition bounds, not OCI protocol limits or sandbox resource guarantees.
const MAX_LAYER_BYTES: u64 = 4 * 1024 * 1024 * 1024;
// erofs-utils' mkfs ftruncates its output to its whole 2 TiB block address space before it
// writes, and shrinks the file to the image at the end (an actual trace: ftruncate(fd, 1 << 41)).
// A written-file limit below that kills it before any data, so this is its bound.
const EROFS_RESERVATION_BYTES: u64 = 1 << 41;
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

/// One prepared manifest: its layers in order, each named by source digest and diff ID.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CachedImage {
    layers: Vec<(String, String)>,
    configuration: ConfigFile,
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
    /// The read-only file's kernel identity when it was hashed; while it is unchanged the
    /// layer is not hashed again on every start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hashed: Option<Identity>,
}

/// Device, inode, size, and modification and change times: any write, chmod or replacement
/// of the layer file changes at least one.
type Identity = (u64, u64, u64, (i64, i64), (i64, i64));

fn identity(metadata: &fs::Metadata) -> Identity {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        (metadata.mtime(), metadata.mtime_nsec()),
        (metadata.ctime(), metadata.ctime_nsec()),
    )
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
        let root = root.join(CONVERSION);
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
        self.prepare_pinned(image, None, timeout).await
    }

    /// With `pinned`, the image is exactly that manifest digest: its complete cached record
    /// prepares it without the registry, and a registry answer for another digest is refused.
    pub async fn prepare_pinned(
        &self,
        image: &str,
        pinned: Option<&str>,
        timeout: Duration,
    ) -> Result<PreparedImage, RuntimeError> {
        tokio::time::timeout(timeout, async {
            let _guard = self.transition.lock().await;
            // Different helper processes share this builder cache. A process-local mutex
            // alone cannot keep a complete record paired with its original layer bytes.
            let lock = self.lock().await?;
            let result = match pinned {
                Some(digest) => match self.offline(digest)? {
                    Some(prepared) => Ok(prepared),
                    None => self.pull(image, Some(digest)).await,
                },
                None => self.pull(image, None).await,
            };
            drop(lock);
            result
        })
        .await
        .map_err(|_| RuntimeError::Timeout("VM OCI image preparation exceeded its deadline".into()))?
    }

    /// The actual per-conversion process lock, also used by acceptance of competing builders.
    pub fn lock_path(&self) -> PathBuf {
        self.root.join("prepare.lock")
    }

    async fn lock(&self) -> Result<File, RuntimeError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.lock_path())
            .map_err(error)?;
        let metadata = file.metadata().map_err(error)?;
        if !metadata.is_file() || metadata.mode() & 0o077 != 0 || metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(error("VM image cache lock must be a private owned regular file"));
        }
        loop {
            match flock(&file, FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => return Ok(file),
                Err(rustix::io::Errno::WOULDBLOCK) => tokio::time::sleep(Duration::from_millis(25)).await,
                Err(problem) => return Err(error(problem)),
            }
        }
    }

    fn image_record(&self, digest: &str) -> Result<PathBuf, RuntimeError> {
        Ok(self.root.join(format!("{}.image.json", digest_name(digest)?)))
    }

    /// A pinned manifest whose record and every converted layer are complete in this cache.
    fn offline(&self, digest: &str) -> Result<Option<PreparedImage>, RuntimeError> {
        let record = self.image_record(digest)?;
        let file = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&record) {
            Ok(file) => file,
            Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(problem) => return Err(error(problem)),
        };
        if !file.metadata().map_err(error)?.is_file() {
            return Err(error("VM image cache record must be regular"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES * 2 + 1).read_to_end(&mut bytes).map_err(error)?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES * 2 {
            return Err(error("Oversized VM image cache record"));
        }
        let saved: CachedImage = serde_json::from_slice(&bytes).map_err(error)?;
        let mut layers = Vec::new();
        for (source, diff_id) in &saved.layers {
            let name = digest_name(source)?;
            let path = self.root.join(format!("{name}.erofs"));
            if !Self::cached(&path, &self.root.join(format!("{name}.json")), source, diff_id)? {
                return Ok(None);
            }
            layers.push(path);
        }
        Ok(Some(PreparedImage { digest: digest.into(), layers, configuration: saved.configuration }))
    }

    async fn pull(&self, image: &str, pinned: Option<&str>) -> Result<PreparedImage, RuntimeError> {
        let named: Reference = image.parse().map_err(error)?;
        let reference = match pinned {
            Some(digest) => Reference::with_digest(named.registry().into(), named.repository().into(), digest.into()),
            None => named,
        };
        let (manifest, digest) =
            registry(|| self.client.pull_image_manifest(&reference, &RegistryAuth::Anonymous)).await?;
        digest_name(&digest)?;
        if pinned.is_some_and(|pinned| pinned != digest) {
            return Err(error("VM image manifest differs from its pinned digest"));
        }
        if manifest.layers.len() > MAX_LAYERS {
            return Err(error("VM OCI image exceeds the layer admission bound"));
        }
        // Explicitly private: a confined filesystem program refuses an ancestor that an
        // ordinary user's group-writable umask (002) would otherwise leave writable.
        let private = || tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).to_owned();
        let staging = private().tempdir_in(&self.root).map_err(error)?;
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
            let temporary = private().tempdir_in(staging.path()).map_err(error)?;
            let work = temporary.path().to_path_buf();
            let generated = convert(&self.tools, &blob, &work, &layer.media_type, diff_id).await?;
            let file = File::open(&generated).map_err(error)?;
            let mut saved = CachedLayer {
                source: layer.digest.clone(),
                diff_id: diff_id.clone(),
                image_bytes: file.metadata().map_err(error)?.len(),
                image_sha256: sha256(&file).map_err(error)?,
                hashed: None,
            };
            file.sync_all().map_err(error)?;
            fs::set_permissions(&generated, fs::Permissions::from_mode(0o444)).map_err(error)?;
            // A missing record never authenticates an existing image. A completed record
            // follows the synced image; cancellation or failure cannot publish partial data.
            fs::rename(generated, &path).map_err(error)?;
            // The identity after the rename, which changes the inode's change time.
            saved.hashed = Some(identity(&fs::symlink_metadata(&path).map_err(error)?));
            pryxd::durable::write_atomic(&record, &serde_json::to_vec(&saved).map_err(error)?, 0o600).map_err(error)?;
            layers.push(path);
        }
        // After every layer and its record, so a record names only complete conversions.
        let saved = CachedImage {
            layers: manifest
                .layers
                .iter()
                .map(|layer| layer.digest.clone())
                .zip(configuration.rootfs.diff_ids.clone())
                .collect(),
            configuration: configuration.clone(),
        };
        pryxd::durable::write_atomic(&self.image_record(&digest)?, &serde_json::to_vec(&saved).map_err(error)?, 0o600)
            .map_err(error)?;
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
            || (saved.hashed != Some(identity(&metadata)) && sha256(file).map_err(error)? != saved.image_sha256)
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

fn layer_uuid(diff_id: &str) -> Result<uuid::Uuid, RuntimeError> {
    use sha2::{Digest as _, Sha256};
    let digest: Digest = diff_id.parse().map_err(error)?;
    let hash = Sha256::digest(digest.to_string().as_bytes());
    let mut bytes = uuid::Bytes::default();
    let length = bytes.len();
    bytes.copy_from_slice(&hash[..length]);
    Ok(uuid::Builder::from_custom_bytes(bytes).into_uuid())
}

async fn convert(
    tools: &FilesystemTools,
    blob: &Path,
    work: &Path,
    media: &str,
    diff_id: &str,
) -> Result<PathBuf, RuntimeError> {
    let filesystem_uuid = layer_uuid(diff_id)?;
    let (input, output, media, diff_id) =
        (blob.to_path_buf(), work.to_path_buf(), media.to_owned(), diff_id.to_owned());
    let tar = tokio::task::spawn_blocking(move || expand(&input, &output, &media, &diff_id)).await.map_err(error)??;
    let image = work.join("layer.erofs");
    // The program writes only this pre-created file, never a path of the tenant's choosing.
    OpenOptions::new().write(true).create_new(true).mode(0o600).open(&image).map_err(error)?;
    let text = |path: &Path| path.to_str().map(String::from).ok_or_else(|| error("VM layer path is not text"));
    let arguments = ["--quiet", "--tar=f", "--aufs", "--ovlfs-strip=0", "-T0", "--mkfs-time", "--preserve-mtime", "-U"];
    let run = Run {
        program: "mkfs.erofs",
        arguments: arguments
            .into_iter()
            .map(String::from)
            .chain([filesystem_uuid.to_string(), text(&image)?, text(&tar)?])
            .collect(),
        read: vec![tar.clone()],
        write: vec![image.clone()],
        file_bytes: EROFS_RESERVATION_BYTES,
    };
    const MAX_DIAGNOSTIC_BYTES: u64 = 32 * 1024;
    // The caller's preparation deadline drops this future, which kills the program.
    let finished = tools.run(run, work, None, MAX_DIAGNOSTIC_BYTES).await?;
    if !finished.status.success() {
        return Err(error(format!(
            "VM EROFS builder failed: {}: {}",
            finished.status,
            String::from_utf8_lossy(&finished.diagnostic)
        )));
    }
    Ok(image)
}
