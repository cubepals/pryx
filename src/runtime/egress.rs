// SPDX-License-Identifier: FSL-1.1-ALv2
//! Hostname-checked transport gateways, independent of engine network orchestration.
//! Callers supply host-owned listeners, source subnet, capacity, cancellation policy and the
//! upstream addresses a sandbox's own rules refuse.
//!
//! Parts (`egress/`):
//! - `gateway.rs`: HTTP/TLS forwarding and public DNS pinning.
//! - `dns.rs`: the policy-checked resolver.
//! - `listeners.rs`: private fixed/ephemeral listener sets and their shared lifetime.
pub(crate) mod dns;
pub(crate) mod gateway;
pub(in crate::runtime) mod listeners;
use crate::protocol::EgressPolicy;

/// Host-selected policy; requests with explicit rules always use Names.
#[derive(Clone)]
pub(in crate::runtime) enum Policy {
    Names(EgressPolicy),
    Public,
}
impl From<EgressPolicy> for Policy {
    fn from(policy: EgressPolicy) -> Self {
        Self::Names(policy)
    }
}

/// A sandbox's own address rules that refuse (its `egressAddresses.deny`): a gateway never dials
/// an upstream one names, whatever name it was resolved for. Every gateway also refuses
/// non-public addresses and this host's own; this is what a sandbox adds.
#[derive(Clone, Default)]
pub(in crate::runtime) struct Refused(Arc<[Destination]>);
impl Refused {
    pub(in crate::runtime) fn new(rules: &[Destination]) -> Self {
        Self(rules.into())
    }
    /// Whether a rule names `upstream`, on its port or on every port. Addresses compare as the
    /// kernel programs key them, IPv4 as IPv4-mapped IPv6, so either spelling of a rule holds.
    pub(in crate::runtime) fn names(&self, upstream: std::net::SocketAddr) -> bool {
        let address = crate::bpf::rules::mapped(ipnet::IpNet::from(upstream.ip())).network();
        self.0.iter().any(|rule| {
            crate::bpf::rules::mapped(rule.prefix).contains(&address)
                && rule.ports.is_none_or(|(low, high)| (low..=high).contains(&upstream.port()))
        })
    }
}
use crate::bpf::rules::Destination;
use std::{net::IpAddr, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_util::sync::CancellationToken;
