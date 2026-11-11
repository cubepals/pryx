// SPDX-License-Identifier: FSL-1.1-ALv2
//! Inner-browser policy is passed only to an agent whose actual executable announces support.
//! Legacy agents remain strict; the container's outer isolation is never modified here.
use super::{ContainerSpec, DockerRuntime, RuntimeError};
use pryx_agent::bootstrap_capabilities::{BROWSER_ARGUMENT, Capabilities, PROBE};
use std::{collections::BTreeMap, path::Path, process::Stdio, time::Duration};
use tokio::io::AsyncReadExt;

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const PROBE_BYTES: u64 = 4096;

pub(super) async fn mode(
    runtime: &DockerRuntime,
    spec: &ContainerSpec,
    image_environment: Option<&[String]>,
) -> Result<Option<bool>, RuntimeError> {
    let browser = image_environment.is_some_and(|environment| {
        environment.iter().any(|value| value == &format!("{}=browser", pryx_agent::contract::TOOL_KIND_ENV))
    });
    let required = spec.require_browser_sandbox == Some(true);
    if !browser && !required {
        return Ok(None);
    }
    let Some(agent) = &spec.agent else {
        return if required {
            Err(RuntimeError::Other("browser-sandbox requires a supported browser agent".into()))
        } else {
            Ok(None)
        };
    };
    let on = runtime.native_tools.browser_profile()
        && tokio::task::spawn_blocking(|| crate::capability::child("probe-user-namespace"))
            .await
            .is_ok_and(|capability| capability.state == crate::capability::State::On);
    if required && !on {
        return Err(RuntimeError::Other(
            "browser-sandbox is unavailable: Chromium needs the host native-tool profile and usable user namespaces"
                .into(),
        ));
    }
    Ok(supported(&agent.binary).await.then_some(on))
}

async fn supported(binary: &Path) -> bool {
    let query = async {
        let mut child = tokio::process::Command::new(binary)
            .arg(PROBE)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .ok()?;
        let mut bytes = Vec::new();
        child.stdout.take()?.take(PROBE_BYTES + 1).read_to_end(&mut bytes).await.ok()?;
        if bytes.len() as u64 > PROBE_BYTES || !child.wait().await.ok()?.success() {
            return None;
        }
        serde_json::from_slice::<Capabilities>(&bytes).ok()
    };
    tokio::time::timeout(PROBE_TIMEOUT, query).await.ok().flatten().is_some_and(|value| value.browser_policy)
}

pub(super) fn bind(mode: Option<bool>, entrypoint: &mut Vec<String>, labels: &mut BTreeMap<String, String>) {
    let Some(mode) = mode else { return };
    // The original image command follows `--`; bootstrap options must precede it.
    let index = entrypoint.iter().position(|value| value == "--").unwrap_or(entrypoint.len());
    entrypoint.splice(index..index, [format!("--{BROWSER_ARGUMENT}"), mode.to_string()]);
    labels.insert(crate::labels::LABEL_BROWSER_SANDBOX.into(), mode.to_string());
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
