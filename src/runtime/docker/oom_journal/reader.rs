// SPDX-License-Identifier: FSL-1.1-ALv2
//! Fixed systemd records from the native host journal, never message text. The maintained
//! journalctl synchronizer drains queued input before acknowledging the causal barrier.
use super::{RuntimeError, failure, valid_hex};
use async_trait::async_trait;
use std::{path::Path, process::Stdio, sync::Arc, time::Duration};
use tokio::io::AsyncReadExt;

// systemd v255 src/systemd/sd-messages.h, UNIT_STARTED and UNIT_OUT_OF_MEMORY.
pub(super) const STARTED: &str = "39f53479d3a045ac8e11786248231fbf";
pub(super) const OOM: &str = "fe6faa94e7774663a0da52717891d8ef";
const FIELDS: &str = "MESSAGE_ID,UNIT,INVOCATION_ID,_BOOT_ID,_PID,_UID,_TRANSPORT,__MONOTONIC_TIMESTAMP";
const BYTES: u64 = 16 * 1024;
const BUDGET: Duration = Duration::from_secs(3);

#[async_trait]
pub(super) trait Source: Send + Sync {
    async fn run(&self, args: &[String]) -> Result<Vec<u8>, RuntimeError>;
    async fn available(&self) -> Result<(), RuntimeError>;
}

pub(super) struct Native;

fn binary() -> Result<std::path::PathBuf, RuntimeError> {
    use std::os::unix::fs::MetadataExt;
    if !rustix::process::geteuid().is_root() {
        return Err(failure("requires the native root system journal"));
    }
    let init = std::fs::read_link("/proc/1/exe").map_err(|_| failure("cannot authenticate host PID 1"))?;
    if init.file_name().is_none_or(|name| name != "systemd") || crate::runtime::protected_agent(&init, true).is_none() {
        return Err(failure("requires native systemd PID 1"));
    }
    // A container or journal namespace must not substitute its own trusted-field authority.
    let ours = std::fs::metadata("/proc/self/ns/pid").map_err(|_| failure("cannot inspect PID namespace"))?;
    let host = std::fs::metadata("/proc/1/ns/pid").map_err(|_| failure("cannot inspect PID 1 namespace"))?;
    if ours.dev() != host.dev() || ours.ino() != host.ino() {
        return Err(failure("requires the system manager PID namespace"));
    }
    for directory in ["/run/log", "/var/log"] {
        let path = Path::new(directory).join("journal");
        if path.exists() {
            for ancestor in path.ancestors() {
                let metadata = std::fs::symlink_metadata(ancestor)
                    .map_err(|_| failure("cannot authenticate system journal directory"))?;
                if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
                    return Err(failure("system journal directory is not protected"));
                }
            }
        }
    }
    ["/usr/bin/journalctl", "/bin/journalctl"]
        .iter()
        .find_map(|path| crate::runtime::protected_agent(Path::new(path), true))
        .ok_or_else(|| failure("requires a protected journalctl executable"))
}

async fn native_version() -> Result<std::path::PathBuf, RuntimeError> {
    let binary = binary()?;
    let output = execute(&binary, &["--version".into()]).await?;
    let version = std::str::from_utf8(&output)
        .ok()
        .and_then(|text| text.lines().next())
        .and_then(|line| line.strip_prefix("systemd "))
        .and_then(|line| line.split_whitespace().next())
        .and_then(|version| version.parse::<u32>().ok());
    if version.is_none_or(|version| version < 255) {
        return Err(failure("requires journalctl 255 or newer with the native Synchronize method"));
    }
    Ok(binary)
}

#[async_trait]
impl Source for Native {
    async fn available(&self) -> Result<(), RuntimeError> {
        let binary = native_version().await?;
        execute(&binary, &["--system".into(), "--sync".into()]).await.map(|_| ())
    }

    async fn run(&self, args: &[String]) -> Result<Vec<u8>, RuntimeError> {
        execute(&native_version().await?, args).await
    }
}

async fn execute(binary: &Path, args: &[String]) -> Result<Vec<u8>, RuntimeError> {
    let mut child = tokio::process::Command::new(binary)
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| failure("cannot start journalctl"))?;
    let mut stdout = child.stdout.take().expect("piped journal stdout").take(BYTES + 1);
    let mut stderr = child.stderr.take().expect("piped journal stderr").take(BYTES + 1);
    let read = async {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let (out, err) = tokio::join!(stdout.read_to_end(&mut output), stderr.read_to_end(&mut errors));
        out.map_err(|_| failure("cannot read journalctl output"))?;
        err.map_err(|_| failure("cannot read journalctl status"))?;
        if output.len() as u64 > BYTES || errors.len() as u64 > BYTES {
            return Err(failure("journalctl output exceeded its bound"));
        }
        let status = child.wait().await.map_err(|_| failure("cannot reap journalctl"))?;
        if !status.success() || !errors.is_empty() {
            return Err(failure("journalctl did not complete a clean bounded read"));
        }
        Ok(output)
    };
    match tokio::time::timeout(BUDGET, read).await {
        Ok(Ok(output)) => Ok(output),
        result => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            match result {
                Ok(Err(error)) => Err(error),
                _ => Err(failure("journalctl exceeded its deadline")),
            }
        }
    }
}

pub(super) async fn sync(source: &Arc<dyn Source>) -> Result<(), RuntimeError> {
    // v255 journalctl.c action_sync uses io.systemd.Journal.Synchronize. journald's
    // deferred +15 request drains all queued input (+5), syncs, then replies.
    let output = source.run(&["--system".into(), "--sync".into()]).await?;
    if !output.is_empty() {
        return Err(failure("unexpected journal synchronization output"));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Record {
    pub(super) invocation: String,
    pub(super) monotonic: u64,
}

pub(super) async fn query(
    source: &Arc<dyn Source>,
    boot: &str,
    id: &str,
    message: &str,
    invocation: Option<&str>,
) -> Result<Vec<Record>, RuntimeError> {
    let scope = format!("docker-{id}.scope");
    let mut args = vec![
        "--system".into(),
        "--no-pager".into(),
        "--quiet".into(),
        "--output=json".into(),
        format!("--output-fields={FIELDS}"),
        "--all".into(),
        "--lines=2".into(),
        format!("_BOOT_ID={boot}"),
        "_PID=1".into(),
        "_UID=0".into(),
        "_TRANSPORT=journal".into(),
        format!("UNIT={scope}"),
        format!("MESSAGE_ID={message}"),
    ];
    if let Some(invocation) = invocation {
        args.push(format!("INVOCATION_ID={invocation}"));
    }
    let output = source.run(&args).await?;
    parse(&output, boot, &scope, message, invocation)
}

pub(super) fn parse(
    output: &[u8],
    boot: &str,
    scope: &str,
    message: &str,
    invocation: Option<&str>,
) -> Result<Vec<Record>, RuntimeError> {
    let mut records = Vec::new();
    for line in output.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()) {
        if records.len() == 2 {
            return Err(failure("journal query returned too many records"));
        }
        let value: serde_json::Value = serde_json::from_slice(line).map_err(|_| failure("malformed journal record"))?;
        let field = |name: &str| value.get(name).and_then(serde_json::Value::as_str);
        if field("MESSAGE_ID") != Some(message)
            || field("UNIT") != Some(scope)
            || field("_BOOT_ID") != Some(boot)
            || field("_PID") != Some("1")
            || field("_UID") != Some("0")
            || field("_TRANSPORT") != Some("journal")
        {
            return Err(failure("journal record authority or scope changed"));
        }
        let id = field("INVOCATION_ID")
            .filter(|id| valid_hex(id, 32))
            .ok_or_else(|| failure("journal invocation identity is unavailable"))?;
        if invocation.is_some_and(|expected| expected != id) {
            return Err(failure("journal invocation identity changed"));
        }
        let monotonic = field("__MONOTONIC_TIMESTAMP")
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| failure("journal monotonic timestamp is unavailable"))?;
        records.push(Record { invocation: id.into(), monotonic });
    }
    Ok(records)
}
