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

use crate::capability::child;
pub use crate::capability::{Capability, State};

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
