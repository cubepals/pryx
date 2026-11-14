// SPDX-License-Identifier: FSL-1.1-ALv2
//! Inner-browser policy is passed only to an agent whose actual executable announces support.
//! Legacy agents remain strict; the container's outer isolation is never modified here.
use super::{ContainerSpec, DockerRuntime, RuntimeError};
use crate::runtime::agent_metadata::supported;
use pryx_agent::bootstrap_capabilities::BROWSER_ARGUMENT;
use std::collections::BTreeMap;

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

pub(super) fn bind(mode: Option<bool>, entrypoint: &mut Vec<String>, labels: &mut BTreeMap<String, String>) {
    let Some(mode) = mode else { return };
    // The original image command follows `--`; bootstrap options must precede it.
    let index = entrypoint.iter().position(|value| value == "--").unwrap_or(entrypoint.len());
    entrypoint.splice(index..index, [format!("--{BROWSER_ARGUMENT}"), mode.to_string()]);
    labels.insert(crate::labels::LABEL_BROWSER_SANDBOX.into(), mode.to_string());
}
