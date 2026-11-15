// SPDX-License-Identifier: FSL-1.1-ALv2
//! Kernel network policy as the process runtime starts: proved on this host, then the programs of
//! supervisors that outlived the last daemon found, verified and served again. What a sandbox's
//! policy is and how it is attached is `network.rs`.
use super::network::{Network, Served, error};
use super::*;

impl ProcessRuntime {
    /// Turns kernel network policy on where the host proves it can attach the programs, and
    /// finds those of supervisors that outlived the last daemon. A sandbox whose address or
    /// hostname policy can't be verified again is stopped rather than left running without it.
    pub(crate) async fn with_network(mut self, served: Served) -> Self {
        let found = crate::bpf::probe();
        self.network_capability = Some(crate::bpf::capability(&found));
        let Ok(host) = found else { return self };
        let network = Arc::new(Network::new(host, served, &self.state.root));
        for name in self.names() {
            let Ok(entry) = self.entry(&name) else { continue };
            if super::observe::matching_pid(entry.pid, entry.ticks).is_none() {
                continue;
            }
            let adopted = match &entry.network {
                Some(record) => network.adopt(&entry.spec, record).await,
                None => Err(error("the sandbox started without it")),
            };
            if let Err(problem) = adopted {
                tracing::warn!(sandbox = %entry.spec.name, %problem, "process sandbox has no verified kernel network policy");
                if entry.network.is_some() && Network::needs_programs(&entry.spec) {
                    let _ = self.terminate_tree(&name).await;
                }
            }
        }
        self.network = Some(network);
        self
    }
}
