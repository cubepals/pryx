// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Verify the whole release-owned VM bundle before using a monitor, kernel or helper. Lazy
//! acquisition reuses the existing pinned archive cache; the descriptor comes from the signed
//! host installation, never a workload. Every executable remains off until verification passes.
use super::{
    error,
    filesystems::{FilesystemTools, sha256},
};
use crate::runtime::RuntimeError;
pub use pryx_agent::process_tool_policy::Artifact;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{self, Read},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const REQUIRED: &[&str] = &[
    "bin/firecracker",
    "bin/jailer",
    "bin/vm-launcher",
    "bin/pryx-vm-image",
    "bin/vm-config",
    "bin/vm-transport",
    "bin/network-proxy",
    "kernel/vmlinux",
    "kernel/config",
    "bootstrap/bootstrap.erofs",
    "bootstrap/bootstrap.json",
    "filesystem/filesystem-manifest.json",
    "notices/source-distribution.json",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    format: u16,
    files: Vec<Member>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    mode: u32,
}

pub struct Bundle {
    root: PathBuf,
    filesystems: FilesystemTools,
    bootstrap: Bootstrap,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub agent_sha256: String,
    pub image_sha256: String,
    pub metadata: pryx_agent::bootstrap_capabilities::Capabilities,
    pub network_probe: Option<String>,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "VM bundle differs from its pinned inventory")
}

pub(super) fn ancestors(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(invalid());
    }
    for parent in path.ancestors() {
        let meta = fs::symlink_metadata(parent)?;
        let sticky = meta.uid() == 0 && meta.mode() & libc::S_ISVTX != 0;
        if !meta.is_dir()
            || (!sticky && meta.mode() & 0o022 != 0)
            || (meta.uid() != 0 && meta.uid() != rustix::process::geteuid().as_raw())
        {
            return Err(invalid());
        }
    }
    Ok(())
}

fn private(path: &Path) -> io::Result<()> {
    ancestors(path.parent().ok_or_else(invalid)?)?;
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(problem) if problem.kind() == io::ErrorKind::AlreadyExists => (),
        Err(problem) => return Err(problem),
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o077 != 0 {
        return Err(invalid());
    }
    Ok(())
}

impl Bundle {
    pub fn verified(root: &Path, digest: &str) -> Result<Self, RuntimeError> {
        ancestors(root).map_err(error)?;
        let mut manifest = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(root.join("manifest.json"))
            .map_err(error)?;
        let meta = manifest.metadata().map_err(error)?;
        if !meta.is_file() || meta.len() > MAX_MANIFEST_BYTES || meta.mode() & 0o022 != 0 {
            return Err(error(invalid()));
        }
        let mut bytes = Vec::new();
        (&mut manifest).take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes).map_err(error)?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES || sha256(bytes.as_slice()).map_err(error)? != digest {
            return Err(error(invalid()));
        }
        let inventory: Inventory = serde_json::from_slice(&bytes).map_err(error)?;
        if inventory.format != 1 {
            return Err(error(invalid()));
        }
        let mut expected = BTreeMap::new();
        for member in inventory.files {
            if member.path.as_os_str().is_empty()
                || member.path == Path::new("manifest.json")
                || member.path.components().any(|part| !matches!(part, Component::Normal(_)))
                || member.mode & !0o755 != 0
            {
                return Err(error(invalid()));
            }
            if expected.insert(member.path.clone(), member).is_some() {
                return Err(error(invalid()));
            }
        }
        if REQUIRED.iter().any(|path| !expected.contains_key(Path::new(path))) {
            return Err(error(invalid()));
        }
        let mut seen = BTreeSet::new();
        let walked = crate::tree::walk(root, &[], &mut |entry| {
            if entry.meta.mode & 0o022 != 0
                || (entry.meta.uid != 0 && entry.meta.uid != rustix::process::geteuid().as_raw())
            {
                return Err(invalid());
            }
            match entry.kind {
                crate::tree::Kind::Dir if expected.keys().any(|path| path.starts_with(entry.path)) => (),
                crate::tree::Kind::File(file) if entry.path == Path::new("manifest.json") => {
                    if sha256(file)? != digest {
                        return Err(invalid());
                    }
                }
                crate::tree::Kind::File(file) => {
                    let member = expected.get(entry.path).ok_or_else(invalid)?;
                    let mode = entry.meta.mode & 0o7777;
                    if entry.meta.size != member.bytes
                        || sha256(file)? != member.sha256
                        || (mode != member.mode && mode != member.mode & 0o555)
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
            return Err(error(invalid()));
        }
        let component = root.join("filesystem");
        let digest = &expected[Path::new("filesystem/filesystem-manifest.json")].sha256;
        let filesystems = FilesystemTools::verified(&component, digest)?;
        let bootstrap_member = &expected[Path::new("bootstrap/bootstrap.json")];
        if bootstrap_member.bytes > MAX_MANIFEST_BYTES {
            return Err(error(invalid()));
        }
        let bootstrap: Bootstrap =
            serde_json::from_slice(&fs::read(root.join("bootstrap/bootstrap.json")).map_err(error)?).map_err(error)?;
        if bootstrap.network_probe.is_some()
            || bootstrap.agent_sha256 != expected[Path::new("bin/vm-launcher")].sha256
            || bootstrap.image_sha256 != expected[Path::new("bootstrap/bootstrap.erofs")].sha256
            || bootstrap.metadata.vm_init_version != pryx_agent::vm_boot::VERSION
            || bootstrap.metadata.vm_monitor_version != pryx_agent::vm_monitor::VERSION
            || bootstrap.metadata.vm_service_port_base != pryx_agent::vm_boot::SERVICE_PORT_BASE
            || bootstrap.metadata.vm_stream_port_base != pryx_agent::vm_boot::STREAM_PORT_BASE
            || bootstrap.metadata.vm_stream_version != pryx_agent::vm_boot::STREAM_VERSION
            || bootstrap.metadata.vm_files_version != pryx_agent::bootstrap_capabilities::VM_FILES_VERSION
        {
            return Err(error("VM launcher and trusted guest bootstrap are incompatible"));
        }
        Ok(Self { root: root.into(), filesystems, bootstrap })
    }

    pub fn bootstrap(&self) -> &Bootstrap {
        &self.bootstrap
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn filesystems(&self) -> &FilesystemTools {
        &self.filesystems
    }

    pub async fn acquire(cache: &Path, downloads: &Path, artifact: &Artifact) -> Result<Self, RuntimeError> {
        private(cache).map_err(error)?;
        private(downloads).map_err(error)?;
        let root = pryx_agent::process_tools::acquire_artifact(cache.into(), downloads.into(), artifact.clone())
            .await
            .map_err(error)?;
        Self::verified(&root, &artifact.manifest_sha256)
    }
}
