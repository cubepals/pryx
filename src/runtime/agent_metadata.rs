// SPDX-License-Identifier: FSL-1.1-ALv2
//! Bounded metadata from the actual protected agent executable. Parser support never asserts
//! host capability, isolation enforcement or native service readiness.
use pryx_agent::bootstrap_capabilities::{Capabilities, PROBE};
use std::{path::Path, process::Stdio, time::Duration};
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn actual_executable_metadata_gates_optional_bootstrap_arguments() {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("agent");
        for (answer, expected) in [
            (serde_json::to_string(&pryx_agent::bootstrap_capabilities::probe()).unwrap(), true),
            ("{}".into(), false),
            ("not-json".into(), false),
            ("x".repeat(PROBE_BYTES as usize + 1), false),
        ] {
            std::fs::write(&binary, format!("#!/bin/sh\nprintf '%s' '{answer}'\n")).unwrap();
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(supported(&binary).await, expected);
        }
        std::fs::write(&binary, "#!/bin/sh\nexit 2\n").unwrap();
        assert!(!supported(&binary).await, "a legacy parser rejection cannot enable arguments");
    }

    #[tokio::test]
    async fn a_stalled_executable_cannot_stall_container_creation() {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("agent");
        std::fs::write(&binary, "#!/bin/sh\nexec /bin/sleep 30\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!supported(&binary).await);
    }
}
