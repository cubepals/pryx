// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The sandbox agent and its small node-facing token contract. Host lifecycle remains in pryxd.
//!
//! Parts (`./`):
//! - `guest.rs`: virtual process controls and private guest ingress adapters.
//! - `context.rs`: HTTP state and service errors.
//! - `auth.rs`: scoped, expiring tokens without a shared verification secret.
//! - `contract.rs`: shared bootstrap names and limits.
//! - `schema.rs`: development-only agent wire schema.
//! - `state.rs`: bounded, non-secret persistent checkpoints.
//! - `telemetry.rs`: in-memory activity and stream ownership.
//! - `process.rs`: supervised process trees, streams, stdin and completion.
//! - `process_policy.rs`: process-tier filesystem, syscall and sampled resource restrictions.
//! - `process_forward.rs`: ownership-verified process-tier forwarding channel.
//! - `process_peer.rs`: kernel process and connected-peer ownership evidence.
//! - `process_capabilities.rs`: non-mutating kernel support queries.
//! - `pty.rs`: terminal ownership through the platform PTY library.
//! - `server.rs`: the authenticated HTTP and websocket surface.
//! - `files.rs`: streamed data and filesystem watches.
//! - `transfers.rs`: remote file transfer requests and streaming.
//! - `transfer_target.rs`: public-address validation and pinned transfer DNS.
//! - `logbook.rs`: bounded persistent entrypoint output.
//! - `mcp.rs`: optional authenticated sandbox MCP tools and resources.
//! - `ports.rs`: preallocated forward slots.
//! - `main.rs`: PID 1 bootstrap and embedded shell.
//! - `bootstrap.rs`: entrypoint supervision and termination.

pub mod auth;
#[cfg(feature = "server")]
pub mod bootstrap;
#[cfg(feature = "server")]
pub mod context;
pub mod contract;
#[cfg(feature = "server")]
pub mod files;
#[cfg(feature = "server")]
pub mod guest;
#[cfg(any(feature = "server", feature = "fs"))]
pub mod logbook;
#[cfg(feature = "mcp")]
mod mcp;
#[cfg(any(feature = "server", feature = "fs"))]
#[path = "../../src/tree/path.rs"]
pub mod paths;
#[cfg(feature = "server")]
pub mod ports;
#[cfg(feature = "server")]
pub mod process;
#[cfg(feature = "fs")]
pub mod process_capabilities;
#[cfg(feature = "server")]
pub mod process_policy;
#[cfg(feature = "server")]
pub mod pty;
#[cfg(all(feature = "schema", feature = "server"))]
pub mod schema;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "fs")]
pub mod state;
#[cfg(feature = "server")]
pub mod telemetry;

#[cfg(feature = "server")]
mod transfers;

#[cfg(feature = "server")]
mod transfer_target;

#[cfg(feature = "server")]
pub mod process_forward;
#[cfg(feature = "server")]
mod process_peer;
