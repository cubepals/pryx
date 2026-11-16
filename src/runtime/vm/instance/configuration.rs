// SPDX-License-Identifier: FSL-1.1-ALv2
//! Generate monitor configuration and backing-file identities through the pinned owner SDK.
//! This module prepares host files; it does not launch compute or handle guest credentials.
use super::super::{bundle::Bundle, error, image::Prepared};
use crate::runtime::RuntimeError;
use pryx_agent::vm_boot;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, process::Stdio};
use tokio::{io::AsyncReadExt, process::Command, time::Instant};

pub(super) struct Configuration {
    pub cid: u32,
    pub ids: Vec<String>,
    pub identities: BTreeMap<String, String>,
    pub write_id: String,
}
pub(super) struct Destination<'a> {
    pub output: &'a Path,
    pub jail: Option<&'a Path>,
    pub tap: Option<(&'a str, &'a str)>,
    /// The monitor's working directory, which relative disk and socket paths are resolved in.
    pub working: Option<&'a Path>,
}
/// The writable disk's name in an ordinary monitor's boot directory, a link to the sandbox's own
/// disk. A snapshot records it so; each restore resolves it in that sandbox's directory.
pub(super) const DISK_LINK: &str = "disk";
/// The monitor's vsock socket, relative to its boot directory in the same way.
pub(super) const CHANNEL: &str = "vsock";
async fn output(mut command: Command, deadline: Instant) -> Result<Vec<u8>, RuntimeError> {
    command.env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    tokio::time::timeout_at(deadline, async {
        let mut child = command.spawn().map_err(error)?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| error("Missing VM configuration output"))?
            .take(pryx_agent::vm_boot::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(error)?;
        if bytes.len() > pryx_agent::vm_boot::MAX_BYTES || !child.wait().await.map_err(error)?.success() {
            return Err(error("VM configurator refused its guest resources or drive paths"));
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| RuntimeError::Timeout("VM configuration exceeded its deadline".into()))?
}

pub(super) async fn configure(
    bundle: &Bundle,
    image: &Prepared,
    disk: &Path,
    channel: &Path,
    destination: Destination<'_>,
    limits: (u64, u32),
    deadline: Instant,
) -> Result<Configuration, RuntimeError> {
    let configurator = bundle.root().join("bin/vm-config");
    let configurator_in = |arguments: &[String]| {
        let mut command = Command::new(&configurator);
        command.args(arguments);
        if let Some(working) = destination.working {
            command.current_dir(working);
        }
        command
    };
    let mut command = Command::new(&configurator);
    command.arg("--metadata");
    let metadata: Value = serde_json::from_slice(&output(command, deadline).await?).map_err(error)?;
    if destination.jail.is_some() && metadata["jailPaths"] != true {
        return Err(error("VM configurator does not advertise staged jail paths"));
    }
    if destination.tap.is_some() && metadata["tapConfiguration"] != true {
        return Err(error("VM configurator does not advertise host-owned TAP configuration"));
    }
    let root = destination.jail.unwrap_or(bundle.root());
    let mut arguments = vec![
        "--kernel".into(),
        root.join("kernel/vmlinux").display().to_string(),
        "--root".into(),
        root.join("bootstrap/bootstrap.erofs").display().to_string(),
        "--writes".into(),
        disk.display().to_string(),
        "--socket".into(),
        channel.display().to_string(),
        "--init-mode".into(),
        bundle.bootstrap().metadata.vm_init_mode.clone(),
        "--memory-mib".into(),
        limits.0.to_string(),
        "--vcpus".into(),
        limits.1.to_string(),
    ];
    if let Some(jail) = destination.jail {
        arguments.extend(["--jail-root".into(), jail.display().to_string()]);
    }
    if let Some((tap, mac)) = destination.tap {
        arguments.extend(["--tap".into(), tap.into(), "--guest-mac".into(), mac.into()]);
    }
    let ids = image.layers.iter().enumerate().map(|(i, _)| format!("layer{i}")).collect::<Vec<_>>();
    for (id, layer) in ids.iter().zip(&image.layers) {
        arguments.extend(["--layer".into(), format!("{id}={}", layer.display())]);
    }
    let config = output(configurator_in(&arguments), deadline).await?;
    crate::durable::write_atomic(destination.output, &config, 0o600).map_err(error)?;
    let mut command = configurator_in(&arguments);
    command.arg("--identities");
    let identities: BTreeMap<String, String> =
        serde_json::from_slice(&output(command, deadline).await?).map_err(error)?;
    let cid = metadata["guestCid"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|cid| vm_boot::guest_cid(*cid))
        .ok_or_else(|| error("VM configurator omitted its guest CID"))?;
    let write_id = metadata["writeDrive"]
        .as_str()
        .ok_or_else(|| error("VM configurator omitted its writable drive ID"))?
        .to_owned();
    Ok(Configuration { cid, ids, identities, write_id })
}
