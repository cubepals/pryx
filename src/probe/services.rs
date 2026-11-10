// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Read-only service discovery. A socket's presence never establishes that its engine works;
//! successful engine info is required. Failed privileged firewall inspection remains unknown.
use super::{Capability, Report, State};
use crate::runtime::{ContainerRuntime, docker::DockerRuntime};
use std::{path::PathBuf, process::Command, time::Duration};

pub(super) async fn read(report: &mut Report) {
    report.capabilities.insert("host-firewall".into(), firewall());
    let uid = rustix::process::geteuid().as_raw();
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")));
    for (name, path) in [
        ("systemd", PathBuf::from("/run/systemd/system")),
        ("user-session", runtime.join("systemd/private")),
        ("linger", PathBuf::from(format!("/var/lib/systemd/linger/{}", username(uid)))),
    ] {
        let capability = if name == "user-session" {
            user_session(&path).await
        } else {
            match path.try_exists() {
                Ok(true) => Capability::on(format!("{} exists", path.display())),
                Ok(false) => Capability::new(
                    State::Off,
                    format!("{} is absent", path.display()),
                    if name == "linger" {
                        "Run loginctl enable-linger for this user"
                    } else {
                        "Start a systemd user session"
                    },
                ),
                Err(error) => Capability::error(error, "Check the user's systemd session"),
            }
        };
        report.capabilities.insert(name.into(), capability);
    }
    let mut sockets = vec![
        ("docker", PathBuf::from("/var/run/docker.sock")),
        ("docker-user", runtime.join("docker.sock")),
        ("podman-user", runtime.join("podman/podman.sock")),
        ("podman", PathBuf::from("/run/podman/podman.sock")),
    ];
    if let Ok(host) = std::env::var("DOCKER_HOST")
        && let Some(path) = host.strip_prefix("unix://")
    {
        sockets.push(("engine-env", PathBuf::from(path)));
    }
    for (name, socket) in sockets {
        let result = async {
            let runtime = DockerRuntime::new(&socket)?;
            runtime.info().await
        };
        let capability = match tokio::time::timeout(Duration::from_secs(2), result).await {
            Ok(Ok(info)) => Capability::on(format!(
                "{} answered info: version {}; cgroup {}; driver {}; security {}",
                socket.display(),
                info.version.as_deref().unwrap_or("unknown"),
                info.cgroup_version.as_deref().unwrap_or("unknown"),
                info.cgroup_driver.as_deref().unwrap_or("unknown"),
                info.security_options.join(", ")
            )),
            Ok(Err(error)) => Capability::new(
                socket_state(&socket),
                error.to_string(),
                "Start a Docker-compatible engine and grant this user socket access",
            ),
            Err(_) => Capability::new(State::Unknown, "Engine info exceeded two seconds", "Check the engine service"),
        };
        report.capabilities.insert(name.into(), capability);
    }
}

fn socket_state(socket: &std::path::Path) -> State {
    match socket.try_exists() {
        Ok(false) => State::Off,
        _ => State::Unknown,
    }
}

async fn user_session(path: &std::path::Path) -> Capability {
    match tokio::time::timeout(Duration::from_secs(2), tokio::net::UnixStream::connect(path)).await {
        Ok(Ok(_)) => Capability::on(format!("{} accepted a connection", path.display())),
        Ok(Err(error)) => Capability::error(error, "Start a systemd user session"),
        Err(_) => Capability::new(State::Unknown, "User session socket did not answer", "Start a systemd user session"),
    }
}

fn username(uid: u32) -> String {
    std::fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .find_map(|line| {
            let fields = line.split(':').collect::<Vec<_>>();
            (fields.get(2)?.parse::<u32>().ok()? == uid).then(|| fields[0].to_owned())
        })
        .unwrap_or_else(|| uid.to_string())
}

pub(crate) fn firewall() -> Capability {
    let result = Command::new("nft").args(["-j", "list", "tables"]).output();
    match result {
        Ok(output) if output.status.success() => {
            let parsed: Result<serde_json::Value, _> = serde_json::from_slice(&output.stdout);
            let loaded = parsed.ok().and_then(|value| value["nftables"].as_array().cloned()).map(|items| {
                items
                    .iter()
                    .any(|item| item["table"]["name"] == crate::isolation::TABLE && item["table"]["family"] == "inet")
            });
            match loaded {
                Some(true) => Capability::on(format!("nft queried table inet {}", crate::isolation::TABLE)),
                Some(false) => Capability::new(
                    State::Off,
                    "nft listed tables and the pryx table is absent",
                    "Run pryxd isolate as root",
                ),
                None => Capability::new(
                    State::Unknown,
                    "nft returned an invalid table document",
                    "Check nftables and its JSON output",
                ),
            }
        }
        Ok(output) => Capability::new(
            State::Unknown,
            String::from_utf8_lossy(&output.stderr).trim(),
            "Run pryxd isolate as root and permit read-only firewall inspection",
        ),
        Err(error) => Capability::error(error, "Install nftables and run pryxd isolate as root"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn inaccessible_paths_and_dead_session_sockets_are_not_absence() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("session");
        assert_eq!(socket_state(&path), State::Off);
        assert_eq!(user_session(&path).await.state, State::Off);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert_eq!(user_session(&path).await.state, State::On);
        drop(listener);
        assert_eq!(user_session(&path).await.state, State::Unknown);
        let cycle = root.path().join("cycle");
        std::os::unix::fs::symlink(&cycle, &cycle).unwrap();
        assert_eq!(socket_state(&cycle), State::Unknown);
    }
}
