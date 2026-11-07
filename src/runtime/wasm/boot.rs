// SPDX-License-Identifier: FSL-1.1-ALv2
//! Build a VM from host policy and verified packages. The root is SQLite-backed; only a separate
//! persistent content directory enters the guest as its data mount. Runtime records stay outside.

use agentos_client::{
    AgentOsConfig, AgentOsLimits, AgentOsSidecarConfig, JsRuntimeLimits, MountConfig, MountPlugin, PackageRef,
    PatternPermissions, PermissionMode, Permissions, PythonLimits, ResourceLimits, RootFilesystemConfig,
    RootFilesystemKind, WasmLimits,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use super::{ContainerSpec, RuntimeError};

pub(super) const CONTENT_DIRECTORY: &str = "content";
const DISTRIBUTION: &str = include_str!("../../../deploy/agentos.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Distribution {
    pub version: String,
    pub binary_sha256: String,
    pub default_packages: Vec<Package>,
    #[serde(default)]
    pub optional_packages: Vec<OptionalPackage>,
}

#[derive(Deserialize)]
pub(super) struct Package {
    pub name: String,
    pub sha256: String,
}

#[derive(Deserialize)]
pub(super) struct OptionalPackage {
    pub name: String,
    pub digest: String,
}

pub(super) fn distribution() -> Result<Distribution, RuntimeError> {
    serde_json::from_str(DISTRIBUTION).map_err(super::error)
}

pub(super) fn binary(root: &Path) -> PathBuf {
    root.join("agentos-runtime-sidecar-linux-x64-gnu/agentos-native-sidecar")
}

pub(super) fn verify(path: &Path, expected: &str) -> Result<(), RuntimeError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    for parent in path.ancestors().skip(1).collect::<Vec<_>>().into_iter().rev() {
        let metadata = std::fs::symlink_metadata(parent).map_err(super::error)?;
        let sticky_root = metadata.uid() == 0 && metadata.mode() & libc::S_ISVTX != 0;
        if !metadata.is_dir()
            || (!sticky_root && metadata.mode() & 0o022 != 0)
            || (metadata.uid() != rustix::process::geteuid().as_raw() && metadata.uid() != 0)
        {
            return Err(super::error("Unsafe agentOS distribution ancestor"));
        }
    }
    let mut file =
        std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path).map_err(super::error)?;
    let metadata = file.metadata().map_err(super::error)?;
    if !metadata.is_file()
        || metadata.mode() & 0o022 != 0
        || (metadata.uid() != rustix::process::geteuid().as_raw() && metadata.uid() != 0)
    {
        return Err(super::error("Unsafe agentOS distribution file"));
    }
    let mut digest = Sha256::new();
    use std::io::Read;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(super::error)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if hex::encode(digest.finalize()) != expected {
        return Err(super::error("agentOS distribution checksum mismatch"));
    }
    Ok(())
}

pub(super) fn package_path(root: &Path, name: &str) -> PathBuf {
    root.join("agentos-core/dist/default-software").join(format!("{name}.aospkg"))
}

pub(super) fn options(
    spec: Option<&ContainerSpec>,
    root: &Path,
    launcher: &Path,
    pool: &str,
    owner: (u32, u32),
) -> Result<AgentOsConfig, RuntimeError> {
    let distribution = distribution()?;
    let mut options = AgentOsConfig {
        sidecar_binary_path: Some(launcher.to_string_lossy().into_owned()),
        sidecar: Some(AgentOsSidecarConfig::Shared { pool: Some(pool.into()) }),
        packages: distribution
            .default_packages
            .iter()
            .map(|p| PackageRef { path: package_path(root, &p.name).to_string_lossy().into_owned() })
            .collect(),
        permissions: Some(Permissions {
            network: Some(PatternPermissions::Mode(PermissionMode::Deny)),
            host_function: Some(PatternPermissions::Mode(PermissionMode::Deny)),
            ..Default::default()
        }),
        ..Default::default()
    };
    if let Some(spec) = spec {
        workload_options(&mut options, spec, owner)?;
    }
    Ok(options)
}

fn workload_options(options: &mut AgentOsConfig, spec: &ContainerSpec, owner: (u32, u32)) -> Result<(), RuntimeError> {
    options.user = Some(
        serde_json::from_value(serde_json::json!({
            "uid": owner.0, "gid": owner.1, "euid": owner.0, "egid": owner.1
        }))
        .map_err(super::error)?,
    );
    options.database = Some(
        serde_json::from_value(serde_json::json!({
            "type":"sqlite_file", "path":spec.data_dir.join("root.sqlite")
        }))
        .map_err(super::error)?,
    );
    options.root_filesystem = RootFilesystemConfig {
        kind: RootFilesystemKind::Native,
        native_plugin: Some(MountPlugin { id: "chunked_sqlite".into(), config: None }),
        ..Default::default()
    };
    options.mounts.push(MountConfig::Native {
        path: spec.mount_path.clone(),
        plugin: MountPlugin {
            id: "host_dir".into(),
            config: Some(serde_json::json!({
                "hostPath": spec.data_dir.join(CONTENT_DIRECTORY)
            })),
        },
        guest_source: None,
        guest_fstype: None,
        read_only: false,
    });
    options.permissions.as_mut().expect("host policy exists").network =
        Some(PatternPermissions::Mode(if spec.internal_network {
            PermissionMode::Deny
        } else {
            PermissionMode::Allow
        }));
    let memory_mb = u64::try_from(spec.memory_bytes.max(0)).map_err(super::error)? / (1024 * 1024);
    options.limits = Some(AgentOsLimits {
        resources: Some(ResourceLimits {
            max_processes: Some(u64::try_from(spec.pids_limit).map_err(super::error)?),
            ..Default::default()
        }),
        js_runtime: Some(JsRuntimeLimits { v8_heap_limit_mb: Some(memory_mb), ..Default::default() }),
        python: Some(PythonLimits { max_old_space_mb: Some(memory_mb), ..Default::default() }),
        wasm: Some(WasmLimits { runner_heap_limit_mb: Some(memory_mb), ..Default::default() }),
        ..Default::default()
    });
    Ok(())
}

pub(super) async fn packages(options: &mut AgentOsConfig, root: &Path, names: &[String]) -> Result<(), RuntimeError> {
    let distribution = distribution()?;
    for name in names {
        if distribution.default_packages.iter().any(|package| package.name == *name) {
            continue;
        }
        if let Some(package) = distribution.optional_packages.iter().find(|package| package.name == *name) {
            let path = root.join("agentos-core/dist/optional-software").join(format!("{name}.aospkg"));
            verify(
                &path,
                package
                    .digest
                    .strip_prefix("sha256:")
                    .ok_or_else(|| super::error("Invalid optional package digest"))?,
            )?;
            options.packages.push(PackageRef { path: path.to_string_lossy().into_owned() });
            continue;
        }
        let package = agentos_client::resolve_registry_software(name, Some(&distribution.version), None)
            .await
            .map_err(super::error)?;
        options.software_sources.push(agentos_client::SoftwareSource::Package(agentos_client::PackageSource::Url {
            url: package.url,
            expected_digest: Some(package.digest),
        }));
    }
    Ok(())
}
