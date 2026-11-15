// SPDX-License-Identifier: FSL-1.1-ALv2
//! Verify the complete host-owned filesystem component before invoking its unchanged programs,
//! each under the installation launcher's filesystem-tool confinement, since their inputs are
//! guest-written disks and tenant-chosen layers. Image references and workload environment
//! never select executable paths or library search paths.
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
    launcher: PathBuf,
}

/// One confined run of a component program over host-staged files.
pub struct Run<'a> {
    pub program: &'a str,
    pub arguments: Vec<String>,
    /// Files and directories it may read, besides its own component.
    pub read: Vec<PathBuf>,
    /// Existing files it may write; it creates nothing outside its private directory.
    pub write: Vec<PathBuf>,
    /// The largest file it may write.
    pub file_bytes: u64,
}

pub struct Finished {
    pub status: std::process::ExitStatus,
    pub output: Vec<u8>,
    pub diagnostic: Vec<u8>,
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
    /// The manifest digest comes from the protected installation's release descriptor, and the
    /// launcher is that installation's verified static agent.
    pub fn verified(root: &Path, manifest_sha256: &str, launcher: &Path) -> Result<Self, RuntimeError> {
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
        regular(launcher).map_err(error)?;
        Ok(Self { root: root.join("tools"), identity: manifest_sha256.into(), launcher: launcher.into() })
    }

    pub fn launcher(&self) -> &Path {
        &self.launcher
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Run one program under the launcher's filesystem-tool confinement: its component, its
    /// read grants and its existing outputs only, no socket or capability, no core and no
    /// file past `file_bytes`. A fresh private directory and lease under `scratch` hold its
    /// working files and are removed after it. Dropping the future kills the program.
    pub async fn run(
        &self,
        run: Run<'_>,
        scratch: &Path,
        deadline: Option<tokio::time::Instant>,
        // Bounds its standard output and its diagnostics, each.
        diagnostic_bytes: u64,
    ) -> Result<Finished, RuntimeError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let private = tempfile::Builder::new()
            .prefix(".vm-tool-")
            .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .tempdir_in(scratch)
            .map_err(error)?;
        let directory = private.path().join("work");
        std::os::unix::fs::DirBuilderExt::mode(&mut fs::DirBuilder::new(), 0o700).create(&directory).map_err(error)?;
        let text =
            |path: &Path| path.to_str().map(String::from).ok_or_else(|| error("VM filesystem tool path is not text"));
        let mut arguments = vec!["--library-path".into(), text(&self.root.join("libraries"))?];
        arguments.push(text(&self.root.join("helpers").join(run.program))?);
        arguments.extend(run.arguments);
        let mut read = vec![self.root.clone()];
        read.extend(run.read);
        let plan = pryx_agent::vm_monitor::Plan {
            program: self.root.join("libraries/ld-musl-x86_64.so.1"),
            arguments,
            directory,
            lock: private.path().join("tool.lock"),
            parent: std::process::id(),
            read,
            write: run.write,
            role: pryx_agent::vm_monitor::Role::FilesystemTool {
                file_bytes: run.file_bytes,
                configuration: Some(self.root.join("config/mke2fs.conf")),
            },
        };
        let mut child = tokio::process::Command::new(&self.launcher)
            .arg(pryx_agent::vm_monitor::MODE)
            .env_clear()
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(error)?;
        let finished = async {
            let mut input = child.stdin.take().ok_or_else(|| error("Missing VM filesystem tool plan pipe"))?;
            input.write_all(&serde_json::to_vec(&plan).map_err(error)?).await.map_err(error)?;
            input.shutdown().await.map_err(error)?;
            drop(input);
            let pipes = (child.stdout.take(), child.stderr.take());
            let (Some(stdout), Some(stderr)) = pipes else {
                return Err(error("Missing VM filesystem tool output pipes"));
            };
            let (mut output, mut diagnostic) = (Vec::new(), Vec::new());
            let (mut stdout, mut stderr) = (stdout.take(diagnostic_bytes + 1), stderr.take(diagnostic_bytes + 1));
            let (read_output, read_diagnostic) =
                tokio::join!(stdout.read_to_end(&mut output), stderr.read_to_end(&mut diagnostic));
            read_output.map_err(error)?;
            read_diagnostic.map_err(error)?;
            if output.len() as u64 > diagnostic_bytes || diagnostic.len() as u64 > diagnostic_bytes {
                child.kill().await.map_err(error)?;
                return Err(error("VM filesystem tool output exceeds its bound"));
            }
            Ok(Finished { status: child.wait().await.map_err(error)?, output, diagnostic })
        };
        match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, finished)
                .await
                .map_err(|_| RuntimeError::Timeout("VM filesystem tool exceeded its deadline".into()))?,
            None => finished.await,
        }
    }
}
