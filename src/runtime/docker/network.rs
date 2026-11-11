// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Engine bridge construction and isolation observation. Host firewall rules and hostname
//! egress stay in their existing modules; this owns no workload placement or data policy.
use super::*;

pub(super) async fn internal(
    runtime: &DockerRuntime,
    name: &str,
    labels: &BTreeMap<String, String>,
) -> Result<(), RuntimeError> {
    let inspect =
        quick("inspect internal network", runtime.docker()?.inspect_network(name, None::<InspectNetworkOptions>)).await;
    match inspect {
        Ok(network) => {
            if network.internal == Some(true)
                && network.options.unwrap_or_default().get(ICC_OPTION).map(String::as_str) == Some("false")
            {
                return Ok(());
            }
            return Err(RuntimeError::Other("The internal network does not enforce host policy".into()));
        }
        Err(RuntimeError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    let request = NetworkCreateRequest {
        name: name.into(),
        driver: Some("bridge".into()),
        internal: Some(true),
        options: Some(HashMap::from([
            (ICC_OPTION.into(), "false".into()),
            (BRIDGE_NAME_OPTION.into(), bridge_name(name)),
        ])),
        labels: Some(labels.clone().into_iter().collect()),
        ..Default::default()
    };
    quick("create internal network", runtime.docker()?.create_network(request)).await?;
    Ok(())
}

pub(super) async fn ensure(
    runtime: &DockerRuntime,
    name: &str,
    labels: &BTreeMap<String, String>,
) -> Result<NetworkReport, RuntimeError> {
    if let Some(isolated) = runtime.network_isolated(name).await? {
        return Ok(NetworkReport { isolated });
    }
    let request = NetworkCreateRequest {
        name: name.to_owned(),
        driver: Some("bridge".into()),
        // Workloads can't reach each other directly; each reaches the outside network and is reached
        // through its published ports only, which the host's rules keep the others off.
        options: Some(HashMap::from([
            (ICC_OPTION.to_owned(), "false".to_owned()),
            (BRIDGE_NAME_OPTION.to_owned(), bridge_name(name)),
        ])),
        labels: Some(labels.clone().into_iter().collect()),
        ..Default::default()
    };
    match quick("create network", runtime.docker()?.create_network(request)).await {
        Ok(_) => Ok(NetworkReport { isolated: true }),
        // Made by someone else in between, maybe without isolation: look again.
        Err(RuntimeError::Conflict(message)) => match runtime.network_isolated(name).await? {
            Some(isolated) => Ok(NetworkReport { isolated }),
            None => Err(RuntimeError::Conflict(message)),
        },
        Err(e) => Err(e),
    }
}

pub(super) async fn isolated(runtime: &DockerRuntime, name: &str) -> Result<Option<bool>, RuntimeError> {
    match quick("inspect network", runtime.docker()?.inspect_network(name, None::<InspectNetworkOptions>)).await {
        Ok(network) => {
            Ok(Some(network.options.unwrap_or_default().get(ICC_OPTION).map(String::as_str) == Some("false")))
        }
        Err(RuntimeError::NotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}
