// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Read-only host capability evidence. Missing permission is unknown, never proof of absence.
//! Child probes change only their private namespaces or disposable KVM state.
//!
//! Parts (`probe/`):
//! - `kernel.rs`: kernel ABI, namespaces, devices and the current cgroup.
//! - `kvm.rs`: a bounded child runs a halt instruction and measures the exit.
//! - `services.rs`: engine sockets, firewall, systemd and user sessions.
mod kernel;
mod kvm;
mod services;

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    On,
    Off,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capability {
    pub state: State,
    pub evidence: String,
    pub action: String,
}
impl Capability {
    fn new(state: State, evidence: impl Into<String>, action: impl Into<String>) -> Self {
        Self { state, evidence: evidence.into(), action: action.into() }
    }
    fn on(evidence: impl Into<String>) -> Self {
        Self::new(State::On, evidence, "No action needed")
    }
    fn error(error: std::io::Error, action: &str) -> Self {
        let state = if error.kind() == std::io::ErrorKind::NotFound { State::Off } else { State::Unknown };
        Self::new(state, error.to_string(), action)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub architecture: String,
    pub c_library: String,
    pub capabilities: BTreeMap<String, Capability>,
    pub landlock_abi: Option<u32>,
}
impl Report {
    pub fn on(&self, name: &str) -> bool {
        self.capabilities.get(name).is_some_and(|c| c.state == State::On)
    }
    pub fn text(&self) -> String {
        let mut text = format!("{} / {}\n", self.architecture, self.c_library);
        for (name, capability) in &self.capabilities {
            let state = match capability.state {
                State::On => "on",
                State::Off => "off",
                State::Unknown => "unknown",
            };
            text.push_str(&format!("{name}: {state} — {}; {}\n", capability.evidence, capability.action));
        }
        text
    }
}

/// Probes this caller's access. This does not need a configuration or acquire a privilege.
pub async fn run() -> Report {
    let mut report = kernel::read(Path::new("/"));
    report.capabilities.insert("user-namespaces".into(), child("probe-user-namespace"));
    let users = if rustix::process::geteuid().is_root() {
        child("probe-separate-users")
    } else {
        Capability::new(
            State::Off,
            "Process runtime uses the caller's uid",
            "Run the server preset as root for a separate uid pool",
        )
    };
    report.capabilities.insert("separate-users".into(), users);
    let kvm = if report.on("kvm-device") { child("probe-kvm") } else { report.capabilities["kvm-device"].clone() };
    report.capabilities.insert("kvm".into(), kvm);
    services::read(&mut report).await;
    report
}

fn child(argument: &str) -> Capability {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let result = (|| -> std::io::Result<Capability> {
        let mut child = Command::new(std::env::current_exe()?)
            .arg(argument)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if child.try_wait()?.is_some() {
                let output = child.wait_with_output()?;
                if output.status.success() {
                    return Ok(Capability::on(String::from_utf8_lossy(&output.stdout).trim()));
                }
                return Ok(Capability::new(
                    State::Unknown,
                    String::from_utf8_lossy(&output.stderr).trim(),
                    if argument == "probe-kvm" {
                        "Grant this user access to /dev/kvm on a KVM host"
                    } else if argument == "probe-separate-users" {
                        "Grant the server's process runtime SETUID and SETGID for its uid pool"
                    } else {
                        "Allow unprivileged user namespaces and the binary's AppArmor namespace policy"
                    },
                ));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(Capability::new(
                    State::Unknown,
                    "Probe exceeded two seconds",
                    "Check host virtualization or namespace policy",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    result.unwrap_or_else(|error| Capability::error(error, "Check executable and host permissions"))
}

/// Only the disposable child invokes these; the daemon never enters another namespace.
pub fn user_namespace_child() -> anyhow::Result<()> {
    kernel::user_namespace_child()
}
pub fn kvm_child() -> anyhow::Result<()> {
    kvm::halt()
}

pub fn separate_users_child() -> anyhow::Result<()> {
    kernel::separate_users_child()
}

pub(crate) fn firewall() -> Capability {
    services::firewall()
}
