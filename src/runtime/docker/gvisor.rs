// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The gvisor tier's part of the Docker backend: the runtime name it starts containers with, why
//! an engine can't serve it, which containers and events of the one engine are whose, and the
//! resolvers its sandboxes ask.
use crate::runtime::RuntimeInfo;

/// The name gVisor's own installer (`runsc install`) registers its runtime under in the engine.
pub const GVISOR_RUNTIME: &str = "runsc";
pub const GVISOR_PODMAN_PROBLEM: &str =
    "gvisor is unavailable on Podman: pryx starts gVisor through the Docker Engine API's runtime field";
pub const GVISOR_MISSING_PROBLEM: &str =
    "gvisor is unavailable: the engine has no runsc runtime registered; install gVisor and run runsc install";

/// A node exec that outlives its deadline under gVisor: Docker can't stop an exec, and its
/// processes live in gVisor's own kernel, where no host PID names them.
pub const EXEC_TIMEOUT_PROBLEM: &str = "exec-timeout is unavailable on the gvisor tier: Docker cannot stop an exec and \
     gVisor's processes have no host PID; run commands through the sandbox agent, which ends its own";
/// Docker's embedded resolver answers only through rules gVisor's own network stack never sees.
pub(crate) const HOSTNAME_EGRESS_PROBLEM: &str = "egress-hostnames is unavailable on the gvisor tier: Docker's embedded resolver is unreachable from gVisor's network stack";

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

/// Whether host-side policy (the FUSE device, the egress gateways' resolver) holds for this
/// backend: a rootful Docker engine, and runc's containers, since gVisor's kernel and network
/// stack are not the host's.
pub(super) fn host_policy(own: Option<&str>, info: Option<&RuntimeInfo>) -> bool {
    own.is_none() && info.is_some_and(|info| !info.rootless && !info.podman)
}

/// Docker stops a paused container by signalling it frozen; gVisor's frozen sandbox can't run
/// the handler before the grace runs out, so the gvisor tier thaws it first, as a wake would.
pub(super) async fn thaw_for_stop(
    runtime: &impl crate::runtime::ContainerRuntime,
    own: Option<&str>,
    name: &str,
) -> Result<(), crate::runtime::RuntimeError> {
    if own.is_some()
        && runtime.inspect(name).await?.is_some_and(|info| info.status == crate::runtime::ContainerStatus::Paused)
    {
        runtime.unpause(name).await?;
    }
    Ok(())
}

/// systemd-resolved's list of the host's own upstream servers: /etc/resolv.conf names its stub.
const RESOLVED_UPSTREAM: &str = "/run/systemd/resolve/resolv.conf";
const HOST_RESOLV_CONF: &str = "/etc/resolv.conf";
/// resolv.conf(5): the resolver uses at most this many nameservers (MAXNS).
const MAXNS: usize = 3;
pub(crate) const RESOLVER_PROBLEM: &str = "gvisor needs a resolver its sandboxes can reach: the host names only loopback resolvers; name a reachable upstream in /etc/resolv.conf";

/// The resolv.conf a gvisor sandbox on a bridge gets, from the host's: Docker's embedded resolver
/// (127.0.0.11) answers only through rules gVisor's network stack never sees, so the sandbox asks
/// the host's upstream resolvers directly, as Docker's default bridge does. Loopback servers are
/// the host's own and unreachable from the sandbox; IPv4 servers go first. None left, none.
fn sandbox_resolv_conf(host: &str) -> Option<String> {
    let mut servers: Vec<std::net::IpAddr> = host
        .lines()
        .filter_map(|line| line.trim().strip_prefix("nameserver"))
        .filter_map(|server| server.trim().parse().ok())
        .filter(|server: &std::net::IpAddr| !server.is_loopback())
        .collect();
    servers.sort_by_key(std::net::IpAddr::is_ipv6);
    servers.truncate(MAXNS);
    if servers.is_empty() {
        return None;
    }
    let kept = host.lines().filter(|line| line.starts_with("search ") || line.starts_with("options "));
    let lines = servers.iter().map(|server| format!("nameserver {server}")).chain(kept.map(str::to_owned));
    Some(lines.map(|line| line + "\n").collect())
}

/// Writes the sandbox's resolv.conf beside its data, in its workload's directory, and returns it
/// for a read-only bind at /etc/resolv.conf, which Docker then leaves to it.
pub(super) fn resolv_conf_for(data_dir: &std::path::Path) -> Result<std::path::PathBuf, crate::runtime::RuntimeError> {
    let conf = [RESOLVED_UPSTREAM, HOST_RESOLV_CONF]
        .into_iter()
        .find_map(|path| std::fs::read_to_string(path).ok().and_then(|host| sandbox_resolv_conf(&host)))
        .ok_or_else(|| crate::runtime::RuntimeError::Other(RESOLVER_PROBLEM.into()))?;
    let workload = data_dir
        .parent()
        .ok_or_else(|| crate::runtime::RuntimeError::Other("data has no workload directory".into()))?;
    let path = workload.join("resolv.conf");
    crate::durable::write_atomic(&path, conf.as_bytes(), 0o644)
        .map_err(|error| crate::runtime::RuntimeError::Other(format!("gvisor resolv.conf: {error}")))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sandbox_asks_the_hosts_reachable_upstream_resolvers() {
        let resolved = "# systemd-resolved\nnameserver 2001:db8::53\nnameserver 10.0.0.2\nnameserver 10.0.0.3\nnameserver 10.0.0.4\nsearch internal.example\noptions edns0\n";
        assert_eq!(
            sandbox_resolv_conf(resolved).as_deref(),
            Some(
                "nameserver 10.0.0.2\nnameserver 10.0.0.3\nnameserver 10.0.0.4\nsearch internal.example\noptions edns0\n"
            )
        );
        assert_eq!(sandbox_resolv_conf("nameserver 127.0.0.53\nnameserver ::1\noptions trust-ad\n"), None);
        assert_eq!(
            sandbox_resolv_conf("nameserver 127.0.0.53\nnameserver 192.0.2.1\n").as_deref(),
            Some("nameserver 192.0.2.1\n")
        );
    }

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
