// SPDX-License-Identifier: FSL-1.1-ALv2
//! Bounded metadata from the actual protected agent executable. Parser support never asserts
//! host capability, isolation enforcement or native service readiness.
use pryx_agent::bootstrap_capabilities::{Capabilities, PROBE};
use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::AsyncReadExt;

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const PROBE_BYTES: u64 = 4096;

#[derive(Debug, thiserror::Error)]
enum ProbeFailure {
    #[error("metadata {0}: {1}")]
    Io(&'static str, #[source] std::io::Error),
    #[error("metadata output exceeds its bound")]
    Oversized,
    #[error("metadata process exited: {0}")]
    Exit(std::process::ExitStatus),
    #[error("metadata parse: {0}")]
    Parse(#[from] serde_json::Error),
}

async fn observed(binary: &Path) -> Result<Capabilities, ProbeFailure> {
    let mut child = tokio::process::Command::new(binary)
        .arg(PROBE)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| ProbeFailure::Io("spawn", error))?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .expect("metadata stdout is piped")
        .take(PROBE_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| ProbeFailure::Io("read", error))?;
    if bytes.len() as u64 > PROBE_BYTES {
        return Err(ProbeFailure::Oversized);
    }
    let exit = child.wait().await.map_err(|error| ProbeFailure::Io("wait", error))?;
    if !exit.success() {
        return Err(ProbeFailure::Exit(exit));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub(crate) async fn query(binary: &Path) -> Option<Capabilities> {
    let result = tokio::time::timeout(PROBE_TIMEOUT, observed(binary)).await;
    #[cfg(test)]
    match &result {
        Err(problem) => eprintln!("Agent metadata timeout: {problem}"),
        Ok(Err(problem)) => eprintln!("Agent metadata failure: {problem}"),
        Ok(Ok(_)) => (),
    }
    result.ok()?.ok()
}

pub(crate) async fn supported(binary: &Path) -> bool {
    query(binary).await.is_some_and(|value| value.browser_policy)
}

/// The installed executable and every ancestor must retain the host ownership policy.
pub(crate) fn protected(path: &Path, allow_root: bool) -> Option<PathBuf> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file()
        || metadata.mode() & 0o022 != 0
        || metadata.mode() & 0o111 == 0
        || (metadata.uid() != rustix::process::geteuid().as_raw() && !(allow_root && metadata.uid() == 0))
    {
        return None;
    }
    // Writable ancestors would let another user replace a checked executable by path.
    for parent in path.ancestors().skip(1) {
        let metadata = std::fs::symlink_metadata(parent).ok()?;
        if !metadata.is_dir()
            || metadata.mode() & 0o022 != 0
            || (metadata.uid() != rustix::process::geteuid().as_raw() && metadata.uid() != 0)
        {
            return None;
        }
    }
    Some(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("agent");
        std::os::unix::fs::symlink(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agent-metadata"),
            &binary,
        )
        .unwrap();
        (directory, binary)
    }

    #[tokio::test]
    async fn actual_executable_metadata_gates_optional_bootstrap_arguments() {
        let (_directory, binary) = fixture();
        let response = binary.with_extension("json");
        for (answer, expected) in [
            (serde_json::to_string(&pryx_agent::bootstrap_capabilities::probe()).unwrap(), true),
            ("{}".into(), false),
            ("not-json".into(), false),
            ("x".repeat(PROBE_BYTES as usize + 1), false),
        ] {
            std::fs::write(&response, answer).unwrap();
            assert_eq!(supported(&binary).await, expected);
        }
        std::fs::remove_file(response).unwrap();
        assert!(!supported(&binary).await, "an executable's nonzero exit cannot enable arguments");
    }

    #[tokio::test]
    async fn a_stalled_executable_cannot_stall_container_creation() {
        let (_directory, binary) = fixture();
        // Opening the unpaired FIFO stalls the real probe executable without a writer.
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            binary.with_extension("json"),
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .unwrap();
        assert!(!supported(&binary).await);
    }
}
