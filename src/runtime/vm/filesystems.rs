// SPDX-License-Identifier: FSL-1.1-ALv2
//! Verify the complete host-owned filesystem component before invoking its unchanged programs.
//! Image references and workload environment never select executable paths or library search paths.
use super::error;
use crate::runtime::RuntimeError;
use ring::digest::{Context, SHA256};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

#[derive(Clone)]
pub struct FilesystemTools {
    root: PathBuf,
    identity: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRecord {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    mode: u32,
}

fn regular(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.mode() & 0o022 != 0
        || (metadata.uid() != rustix::process::geteuid().as_raw() && metadata.uid() != 0)
    {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Unsafe VM filesystem component file"));
    }
    Ok(file)
}

pub fn sha256(mut source: impl Read) -> io::Result<String> {
    let mut digest = Context::new(&SHA256);
    let mut buffer = [0; 64 * 1024];
    loop {
        let length = source.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    Ok(hex::encode(digest.finish()))
}

impl FilesystemTools {
    /// The manifest digest comes from the protected installation's release descriptor.
    pub fn verified(root: &Path, manifest_sha256: &str) -> Result<Self, RuntimeError> {
        if !root.is_absolute() {
            return Err(error("VM filesystem component path must be absolute"));
        }
        for ancestor in root.ancestors() {
            if !fs::symlink_metadata(ancestor).map_err(error)?.is_dir() {
                return Err(error("VM filesystem component ancestor must be a real directory"));
            }
        }
        let manifest = regular(&root.join("filesystem-manifest.json")).map_err(error)?;
        const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
        if manifest.metadata().map_err(error)?.len() > MAX_MANIFEST_BYTES {
            return Err(error("VM filesystem component manifest exceeds its bound"));
        }
        if sha256(&manifest).map_err(error)? != manifest_sha256 {
            return Err(error("VM filesystem component manifest checksum mismatch"));
        }
        let mut bytes = Vec::new();
        // Hashing a borrowed File advances its shared offset. Rewind the same
        // descriptor instead of trusting a replacement path for deserialization.
        use std::io::{Seek, SeekFrom};
        let mut manifest = manifest;
        manifest.seek(SeekFrom::Start(0)).map_err(error)?;
        manifest.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes).map_err(error)?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(error("VM filesystem component manifest exceeds its bound"));
        }
        let records: Vec<FileRecord> = serde_json::from_slice(&bytes).map_err(error)?;
        let mut expected = BTreeMap::new();
        for record in records {
            if record.path.components().any(|part| !matches!(part, Component::Normal(_)))
                || !record.path.starts_with("tools")
            {
                return Err(error("Invalid VM filesystem component member"));
            }
            let path = record.path.strip_prefix("tools").map_err(error)?.to_path_buf();
            if expected.insert(path, record).is_some() {
                return Err(error("Repeated VM filesystem component member"));
            }
        }
        let mut seen = BTreeSet::new();
        let walked = crate::tree::walk(&root.join("tools"), &[], &mut |entry| {
            let invalid =
                || io::Error::new(io::ErrorKind::InvalidData, "VM filesystem component differs from its manifest");
            if entry.meta.mode & 0o022 != 0
                || (entry.meta.uid != rustix::process::geteuid().as_raw() && entry.meta.uid != 0)
            {
                return Err(invalid());
            }
            match entry.kind {
                crate::tree::Kind::Dir if expected.keys().any(|path| path.starts_with(entry.path)) => (),
                crate::tree::Kind::File(file) => {
                    let record = expected.get(entry.path).ok_or_else(invalid)?;
                    if entry.meta.size != record.bytes
                        || entry.meta.mode & 0o777 != record.mode
                        || sha256(file)? != record.sha256
                    {
                        return Err(invalid());
                    }
                    seen.insert(entry.path.to_path_buf());
                }
                _ => return Err(invalid()),
            }
            Ok(())
        })
        .map_err(error)?;
        if walked.skipped != 0 || seen.len() != expected.len() {
            return Err(error("VM filesystem component walk was incomplete"));
        }
        for required in ["helpers/mkfs.erofs", "helpers/mke2fs", "libraries/ld-musl-x86_64.so.1", "config/mke2fs.conf"]
        {
            if !seen.contains(Path::new(required)) {
                return Err(error("VM filesystem component lacks a required program or configuration"));
            }
        }
        Ok(Self { root: root.join("tools"), identity: manifest_sha256.into() })
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }

    pub fn builder(&self, program: &str) -> std::process::Command {
        let mut command = std::process::Command::new(self.root.join("libraries/ld-musl-x86_64.so.1"));
        command
            .env_clear()
            .arg("--library-path")
            .arg(self.root.join("libraries"))
            .arg(self.root.join("helpers").join(program))
            .env("MKE2FS_CONFIG", self.root.join("config/mke2fs.conf"));
        command
    }
}
