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
    pub(crate) async fn with_network(self, served: Served) -> Self {
        self.with_network_programs(served, crate::bpf::object::OBJECT).await
    }

    /// As `with_network`, proving the host with `programs` in place of the embedded object. Only
    /// tests hand it another: one whose programs the verifier refuses.
    #[doc(hidden)]
    pub async fn with_network_programs(mut self, served: Served, programs: &[u8]) -> Self {
        let found = crate::bpf::probe_object(programs);
        let capability = crate::bpf::capability(&found);
        // What an operator, and the installed acceptance, read of this start's proof.
        let on = capability.state == crate::capability::State::On;
        tracing::info!(on, evidence = %capability.evidence, "kernel network policy");
        self.network_capability = Some(capability);
        let Ok(host) = found else { return self };
        let network = Arc::new(Network::new(host, served, &self.state.root));
        // Heard from before any sandbox is adopted, so no change of the host's addresses is missed.
        let watched = crate::bpf::host::subscribe().and_then(|changes| {
            let network = Arc::downgrade(&network);
            std::thread::Builder::new().name("pryx-addresses".into()).spawn(move || Network::watch(&network, changes))
        });
        if let Err(problem) = watched {
            let evidence = format!("this daemon can't hear of changes to the host's addresses: {problem}");
            tracing::warn!(%evidence, "kernel network policy");
            self.network_capability = Some(crate::capability::Capability::new(
                crate::capability::State::Off,
                evidence,
                crate::bpf::host::WATCH_ACTION,
            ));
            return self;
        }
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
