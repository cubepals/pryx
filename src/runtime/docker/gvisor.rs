// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The gvisor tier's part of the Docker backend: the runtime name it starts containers with, why
//! an engine can't serve it, and which containers and events of the one engine are whose.
use crate::runtime::RuntimeInfo;

/// The name gVisor's own installer (`runsc install`) registers its runtime under in the engine.
pub const GVISOR_RUNTIME: &str = "runsc";
pub(crate) const GVISOR_PODMAN_PROBLEM: &str =
    "gvisor is unavailable on Podman: pryx starts gVisor through the Docker Engine API's runtime field";
pub const GVISOR_MISSING_PROBLEM: &str =
    "gvisor is unavailable: the engine has no runsc runtime registered; install gVisor and run runsc install";

/// Why an engine that serves containers can't serve the gvisor tier, from its own `info`.
pub fn problem(info: &RuntimeInfo) -> Option<String> {
    if info.podman {
        Some(GVISOR_PODMAN_PROBLEM.into())
    } else if !info.runtimes.iter().any(|name| name == GVISOR_RUNTIME) {
        Some(GVISOR_MISSING_PROBLEM.into())
    } else {
        None
    }
}

/// Whether a container made with `runtime` (its inspected `HostConfig.Runtime`, or the label an
/// event carries) belongs to the backend whose own runtime is `own`: one engine serves both
/// tiers, and each sees only its own containers.
pub(super) fn owns(own: Option<&str>, runtime: Option<&str>) -> bool {
    match own {
        Some(own) => runtime == Some(own),
        None => runtime != Some(GVISOR_RUNTIME),
    }
}

/// Whether an inspected container is the backend's whose own runtime is `own`.
pub(super) fn owns_container(own: Option<&str>, inspect: &bollard::models::ContainerInspectResponse) -> bool {
    owns(own, inspect.host_config.as_ref().and_then(|host| host.runtime.as_deref()))
}

/// Whether an event is the backend's: an event names its container's labels, and pryxd labels
/// every container it makes with a runtime with that runtime's name. A failed read is everyone's.
pub(super) fn owns_event(
    own: Option<&str>,
    item: &Result<bollard::models::EventMessage, bollard::errors::Error>,
) -> bool {
    let Ok(message) = item else { return true };
    let labels = message.actor.as_ref().and_then(|actor| actor.attributes.as_ref());
    owns(own, labels.and_then(|labels| labels.get(crate::labels::LABEL_OCI_RUNTIME)).map(String::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_backend_owns_only_its_runtime() {
        assert!(owns(None, None) && owns(None, Some("runc")));
        assert!(!owns(None, Some(GVISOR_RUNTIME)));
        assert!(owns(Some(GVISOR_RUNTIME), Some(GVISOR_RUNTIME)));
        assert!(!owns(Some(GVISOR_RUNTIME), None) && !owns(Some(GVISOR_RUNTIME), Some("runc")));
    }

    #[test]
    fn an_engine_serves_gvisor_only_with_runsc_registered() {
        let mut info = RuntimeInfo { runtimes: vec!["runc".into()], ..Default::default() };
        assert_eq!(problem(&info).as_deref(), Some(GVISOR_MISSING_PROBLEM));
        info.runtimes.push(GVISOR_RUNTIME.into());
        assert_eq!(problem(&info), None);
        info.podman = true;
        assert_eq!(problem(&info).as_deref(), Some(GVISOR_PODMAN_PROBLEM));
    }
}
