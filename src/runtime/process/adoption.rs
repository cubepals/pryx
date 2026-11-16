// SPDX-License-Identifier: FSL-1.1-ALv2
//! Kernel network policy as the process runtime starts: proved on this host, then the programs of
//! supervisors that outlived the last daemon found, verified and served again. What a sandbox's
//! policy is and how it is attached is `network.rs`.
use super::network::{Network, Served, error};
use super::*;

impl ProcessRuntime {
    /// Turns kernel network policy on where the host proves it can attach the programs, and
    /// finds those of supervisors that outlived the last daemon. A sandbox that had programs and
    /// can't be shown to have them again is stopped rather than left running without them.
    pub(crate) async fn with_network(self, served: Served) -> Self {
        self.with_network_programs(served, crate::bpf::object::OBJECT).await
    }

    /// As `with_network`, proving the host with `programs` in place of the embedded object. Only
    /// tests hand it another: one whose programs the verifier refuses.
    #[doc(hidden)]
    pub async fn with_network_programs(mut self, served: Served, programs: &[u8]) -> Self {
        let found = self
            .legacy_history()
            .map_err(|problem| crate::bpf::host::Unavailable {
                evidence: problem.to_string(),
                action: "Restore access to this daemon's private process history before enabling kernel network policy",
            })
            .and_then(|()| crate::bpf::probe_object(programs));
        let capability = crate::bpf::capability(&found);
        // What an operator, and the installed acceptance, read of this start's proof.
        let on = capability.state == crate::capability::State::On;
        tracing::info!(on, evidence = %capability.evidence, "kernel network policy");
        self.network_capability = Some(capability);
        let Ok(host) = found else {
            // A failed probe cannot adopt a previously policed sandbox either. Locate whatever
            // cleanup the host still permits without treating that location as a capability proof.
            let cleanup = crate::bpf::host::Host::find().ok().map(|host| Network::new(host, served, &self.state.root));
            self.stop_recorded(cleanup.as_ref()).await;
            return self;
        };
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
            self.stop_recorded(Some(&network)).await;
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
                // Its programs were its only defence against this host, whatever its policy; one
                // started before the host had them has none to lose.
                if entry.network.is_some() {
                    self.stop_unverified(&entry, Some(&network)).await;
                }
            }
        }
        self.network = Some(network);
        let unpoliced = self.unpoliced();
        if unpoliced > 0 {
            tracing::warn!(
                unpoliced,
                "process sandboxes run without kernel network policy; host-firewall is off on the process tier until they stop or start again"
            );
        }
        self
    }

    /// A startup that can't verify programs stops every supervisor that previously had them;
    /// legacy supervisors without a record keep their existing, explicitly weaker mode.
    async fn stop_recorded(&self, network: Option<&Network>) {
        for name in self.names() {
            let Ok(entry) = self.entry(&name) else { continue };
            if entry.network.is_some() {
                self.stop_unverified(&entry, network).await;
            }
        }
    }

    async fn stop_unverified(&self, entry: &Entry, network: Option<&Network>) {
        if let Err(problem) = self.terminate_tree(&entry.spec.name).await {
            tracing::warn!(sandbox = %entry.spec.name, %problem, "unverified process sandbox's tree did not stop");
            let _ = self.signal_tree(&entry.spec.name, rustix::process::Signal::KILL);
        }
        if let Some(network) = network
            && let Err(problem) = network.release(&entry.spec.name)
        {
            tracing::warn!(sandbox = %entry.spec.name, %problem, "unverified process sandbox's programs retained until it stops");
        }
        self.gone(entry).await;
    }

    /// Waits, a bounded time, until a supervisor it stopped has exited: with shared UIDs there is
    /// no orphan sweep that waits for it, and a stopped sandbox must no longer count as running.
    async fn gone(&self, entry: &Entry) {
        const GONE: Duration = Duration::from_secs(5);
        const POLL: Duration = Duration::from_millis(10);
        let deadline = tokio::time::Instant::now() + GONE;
        while super::observe::matching_pid(entry.pid, entry.ticks).is_some() {
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(sandbox = %entry.spec.name, "a stopped process sandbox has not exited");
                return;
            }
            tokio::time::sleep(POLL).await;
        }
    }
}
