// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The sandbox agent and its small node-facing token contract. Host lifecycle remains in pryxd.
//!
//! Parts (`./`):
//! - `context.rs`: HTTP state and service errors.
//! - `auth.rs`: scoped, expiring tokens without a shared verification secret.
//! - `contract.rs`: shared bootstrap names and limits.
//! - `schema.rs`: development-only agent wire schema.
//! - `state.rs`: bounded, non-secret persistent checkpoints.
//! - `telemetry.rs`: in-memory activity and stream ownership.
//! - `process.rs`: supervised process trees, streams, stdin and completion.
//! - `pty.rs`: terminal ownership through the platform PTY library.
//! - `server.rs`: the authenticated HTTP and websocket surface.
//! - `files.rs`: streamed data and filesystem watches.
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
#[cfg(any(feature = "server", feature = "fs"))]
#[path = "../../src/tree/path.rs"]
pub mod paths;
#[cfg(feature = "server")]
pub mod ports;
#[cfg(feature = "server")]
pub mod process;
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
