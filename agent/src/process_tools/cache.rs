// SPDX-License-Identifier: FSL-1.1-ALv2
//! Acquisition runs outside guest filesystem grants. A complete pinned archive and its pinned
//! file inventory are verified before atomic cache publication or any runtime execution.
use super::{Artifact, Policy};
use futures_util::StreamExt;
use ring::digest::{Context, SHA256};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    time::Duration,
};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
pub(super) fn hash(mut source: impl Read) -> io::Result<String> {
    let mut digest = Context::new(&SHA256);
    let mut bytes = [0; 64 * 1024];
    loop {
        let length = source.read(&mut bytes)?;
        if length == 0 {
            break;
        }
        digest.update(&bytes[..length]);
    }
    Ok(digest.finish().as_ref().iter().map(|byte| format!("{byte:02x}")).collect())
}
fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}
fn address_allowed(url: &reqwest::Url) -> bool {
    url.username().is_empty()
        && url.password().is_none()
        && (url.scheme() == "https"
            || (cfg!(debug_assertions)
                && url.scheme() == "http"
                && url.host_str().is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "[::1]"))))
}

fn host_client() -> Result<reqwest::ClientBuilder, Box<dyn std::error::Error>> {
    let mut client = reqwest::Client::builder();
    // Host trust configuration belongs to the pre-filesystem-policy worker. Workload
    // environment variables and bundle libraries never choose an acquisition trust store.
    const MAX_HOST_ROOTS_BYTES: u64 = 4 * 1024 * 1024;
    for path in [
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
        "/etc/ssl/ca-bundle.pem",
        "/etc/ssl/cert.pem",
    ] {
        let source = match File::open(path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !source.metadata()?.is_file() {
            return Err(invalid("Host CA bundle must be a regular file").into());
        }
        let mut bytes = Vec::new();
        source.take(MAX_HOST_ROOTS_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_HOST_ROOTS_BYTES {
            return Err(invalid("Host CA bundle exceeds its read bound").into());
        }
        for certificate in reqwest::Certificate::from_pem_bundle(&bytes)? {
            client = client.add_root_certificate(certificate);
        }
        break;
    }
    Ok(client)
}

pub(super) async fn prepare(policy: &Policy) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let artifact = policy.artifact.as_ref().ok_or("This installation has no pinned process tool artifact")?;
    for digest in [&artifact.sha256, &artifact.manifest_sha256] {
        if digest.len() != SHA256.output_len() * 2 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid("Invalid process tool digest").into());
        }
    }
    let destination = policy.cache.join(&artifact.sha256);
    if destination.exists() {
        verify(&destination, artifact)?;
        readonly(&destination)?;
        return Ok(destination);
    }
    // This sibling is deliberately outside both the cache read grant and scratch write grant.
    let temporary = tempfile::Builder::new().prefix("pryx-tools-").tempdir_in(&policy.downloads)?;
    let archive = temporary.path().join("bundle.tar.gz");
    download(artifact, &archive).await?;
    let expanded = temporary.path().join("expanded");
    fs::create_dir(&expanded)?;
    extract(&archive, &expanded, artifact)?;
    verify(&expanded, artifact)?;
    // Moving a directory across parents requires owner write access to update its `..`.
    // The guest cache grant is already read-only; publish complete verified bytes first,
    // then seal all modes before acknowledging readiness or launching any program.
    match fs::rename(&expanded, &destination) {
        Ok(()) => (),
        Err(error) if destination.exists() => {
            verify(&destination, artifact)?;
            let _ = error;
        }
        Err(error) => return Err(error.into()),
    }
    readonly(&destination)?;
    Ok(destination)
}

async fn download(artifact: &Artifact, archive: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let url = reqwest::Url::parse(&artifact.url)?;
    if !address_allowed(&url) {
        return Err(invalid("Process tool acquisition requires HTTPS").into());
    }
    let client = host_client()?
        .timeout(Duration::from_mins(5))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !address_allowed(attempt.url()) {
                attempt.error("Process tool redirect refused")
            } else {
                attempt.follow()
            }
        }))
        .build()?;
    let response = client.get(url).send().await?.error_for_status()?;
    if response.content_length().is_some_and(|bytes| bytes != artifact.bytes) {
        return Err(invalid("Process tool archive length mismatch").into());
    }
    let mut output = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(archive)?;
    let mut chunks = response.bytes_stream();
    let mut digest = Context::new(&SHA256);
    let mut total = 0u64;
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        total = total.checked_add(chunk.len() as u64).ok_or_else(|| invalid("Process tool archive is too large"))?;
        if total > artifact.bytes {
            return Err(invalid("Process tool archive is too large").into());
        }
        output.write_all(&chunk)?;
        digest.update(&chunk);
    }
    let observed: String = digest.finish().as_ref().iter().map(|byte| format!("{byte:02x}")).collect();
    if total != artifact.bytes || observed != artifact.sha256 {
        return Err(invalid("Process tool archive digest or length mismatch").into());
    }
    output.sync_all()?;
    Ok(())
}

fn extract(archive: &Path, output: &Path, artifact: &Artifact) -> io::Result<()> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
    let (mut bytes, mut count) = (0u64, 0u64);
    for entry in archive.entries()? {
        let mut entry = entry?;
        count += 1;
        bytes = bytes.checked_add(entry.size()).ok_or_else(|| invalid("Tool bundle expansion overflow"))?;
        if count > artifact.entries || bytes > artifact.unpacked_bytes || !relative(&entry.path()?) {
            return Err(invalid("Tool bundle expansion exceeds its pinned bounds"));
        }
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() && !kind.is_symlink() {
            return Err(invalid("Unsupported process tool archive entry"));
        }
        if kind.is_symlink() {
            let target = entry.link_name()?.ok_or_else(|| invalid("Missing bundle link target"))?;
            if target.is_absolute() {
                return Err(invalid("Absolute process tool link"));
            }
            let mut depth = entry.path()?.parent().map(|path| path.components().count()).unwrap_or(0);
            for part in target.components() {
                match part {
                    Component::Normal(_) => depth += 1,
                    Component::CurDir => (),
                    Component::ParentDir if depth > 0 => depth -= 1,
                    _ => return Err(invalid("Escaping process tool link")),
                }
            }
        }
        if !entry.unpack_in(output)? {
            return Err(invalid("Process tool archive escapes its destination"));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct Inventory {
    files: Vec<Member>,
}
#[derive(Deserialize)]
struct Member {
    path: PathBuf,
    sha256: Option<String>,
    link: Option<PathBuf>,
}
fn verify(root: &Path, artifact: &Artifact) -> io::Result<()> {
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(invalid("Process tool cache root must be a real directory"));
    }
    let inventory = root.join("manifest.json");
    if hash(File::open(&inventory)?)? != artifact.manifest_sha256 {
        return Err(invalid("Process tool inventory digest mismatch"));
    }
    let manifest: Inventory = serde_json::from_reader(File::open(&inventory)?)?;
    let canonical = fs::canonicalize(root)?;
    let mut expected = BTreeSet::from([PathBuf::from("manifest.json")]);
    for member in manifest.files {
        if !relative(&member.path) || !expected.insert(member.path.clone()) {
            return Err(invalid("Invalid process tool inventory path"));
        }
        let path = root.join(&member.path);
        let metadata = fs::symlink_metadata(&path)?;
        match (member.sha256, member.link) {
            (Some(wanted), None) if metadata.is_file() && !metadata.is_symlink() => {
                if hash(File::open(path)?)? != wanted {
                    return Err(invalid("Cached process tool digest mismatch"));
                }
            }
            (None, Some(wanted)) if metadata.is_symlink() => {
                if fs::read_link(&path)? != wanted || !fs::canonicalize(path)?.starts_with(&canonical) {
                    return Err(invalid("Cached process tool link mismatch"));
                }
            }
            _ => return Err(invalid("Cached process tool type mismatch")),
        }
    }
    check_extra(root, root, &expected)?;
    Ok(())
}
fn check_extra(root: &Path, directory: &Path, expected: &BTreeSet<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            check_extra(root, &entry.path(), expected)?;
        } else if !expected.contains(entry.path().strip_prefix(root).expect("cache descendant")) {
            return Err(invalid("Uninventoried process tool cache entry"));
        }
    }
    Ok(())
}
fn readonly(root: &Path) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            continue;
        }
        if entry.file_type()?.is_dir() {
            readonly(&entry.path())?;
        } else {
            let executable = entry.metadata()?.permissions().mode() & 0o111 != 0;
            fs::set_permissions(entry.path(), fs::Permissions::from_mode(if executable { 0o555 } else { 0o444 }))?;
        }
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o555))
}
