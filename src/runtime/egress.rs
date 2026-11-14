// SPDX-License-Identifier: FSL-1.1-ALv2
//! Hostname-checked transport gateways, independent of engine network orchestration.
//! Callers supply host-owned listeners, source subnet, capacity and cancellation policy.
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
use std::{net::IpAddr, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_util::sync::CancellationToken;
