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
    config_path: &Path,
    limits: (u64, u32),
    deadline: Instant,
) -> Result<Configuration, RuntimeError> {
    let mut arguments = vec![
        "--kernel".into(),
        bundle.root().join("kernel/vmlinux").display().to_string(),
        "--root".into(),
        bundle.root().join("bootstrap/bootstrap.erofs").display().to_string(),
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
    let ids = image.layers.iter().enumerate().map(|(i, _)| format!("layer{i}")).collect::<Vec<_>>();
    for (id, layer) in ids.iter().zip(&image.layers) {
        arguments.extend(["--layer".into(), format!("{id}={}", layer.display())]);
    }
    let configurator = bundle.root().join("bin/vm-config");
    let mut command = Command::new(&configurator);
    command.args(&arguments);
    let config = output(command, deadline).await?;
    crate::durable::write_atomic(config_path, &config, 0o600).map_err(error)?;
    let mut command = Command::new(&configurator);
    command.args(&arguments).arg("--identities");
    let identities: BTreeMap<String, String> =
        serde_json::from_slice(&output(command, deadline).await?).map_err(error)?;
    let mut command = Command::new(&configurator);
    command.arg("--metadata");
    let metadata: Value = serde_json::from_slice(&output(command, deadline).await?).map_err(error)?;
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
