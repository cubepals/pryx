// SPDX-License-Identifier: FSL-1.1-ALv2
//! Hostname-checked transport gateways, independent of engine network orchestration.
//! Callers supply host-owned listeners, source subnet, capacity and cancellation policy.
//!
//! Parts (`egress/`):
//! - `gateway.rs`: HTTP/TLS forwarding and public DNS pinning.
//! - `dns.rs`: the policy-checked resolver.
pub(crate) mod dns;
pub(crate) mod gateway;
use crate::protocol::EgressPolicy;
use std::{net::IpAddr, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_util::sync::CancellationToken;
