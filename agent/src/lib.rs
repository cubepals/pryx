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
//! - `process_tools.rs`: host-bound native bundle acquisition and private runtime inputs.
//! - `process_tool_policy.rs`: independent native artifact/cache policy types and parser version.
//! - `process_observation.rs`: challenge-bound signed observations of installed process isolation.
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
//! - `field_process.rs`: bounded readiness waits and process supervision conveniences.
//! - `shell_state.rs`: persistent shell registry without router dependencies.
//! - `kernels.rs`: managed Jupyter language kernels and event streaming.
//! - `kernel_slots.rs`: opaque kernel storage owned by the process supervisor.
//! - `tool_slots.rs`: native tool state and non-secret child environment.
//! - `fuse_namespace.rs`: host-approved nested namespaces and mount-only capabilities.
//! - `tools.rs`: established desktop, browser, SSH and object-store adapters.
//! - `shells.rs`: serialized persistent pipe shells with exact stream markers.
//! - `file_search.rs`: confined bounded searches and archive transfers.
//! - `ports.rs`: preallocated forward slots.
//! - `main.rs`: PID 1 bootstrap and embedded shell.
//! - `bootstrap.rs`: entrypoint supervision and termination.
//! - `bootstrap_capabilities.rs`: optional executable bootstrap argument discovery.
//! - `vm_ingress.rs`: host-CID-only virtio-socket ingress to guest-loopback agent ports.
//! - `vm_boot.rs`: private in-memory VM bootstrap contract, separate from fleet protocol and disk records.
//! - `vm_network.rs`: trusted guest network readiness, resolver mounts and child lifetime.
//! - `vm_init.rs`: trusted guest init, mount preparation and orderly non-root agent supervision.
//! - `vm_observation.rs`: boot-scoped signed disk-shutdown observations.
//! - `vm_metrics.rs`: optional read-only guest kernel resource counters.
//! - `vm_shutdown.rs`: authenticated host-CID-only requests to trusted guest init.
//! - `vm_monitor.rs`: host-owned monitor confinement and inherited lifetime lease.

pub mod auth;
#[cfg(feature = "server")]
pub mod bootstrap;
pub mod bootstrap_capabilities;
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
pub mod process_observation;
#[cfg(feature = "server")]
pub mod process_policy;
#[cfg(feature = "server")]
pub mod process_tool_policy;
#[cfg(feature = "server")]
pub mod process_tools;
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
pub mod vm_boot;
#[cfg(feature = "server")]
mod vm_ingress;
#[cfg(feature = "server")]
pub mod vm_init;
#[cfg(feature = "server")]
mod vm_metrics;
#[cfg(feature = "server")]
pub mod vm_monitor;
#[cfg(feature = "server")]
mod vm_network;
#[cfg(feature = "server")]
pub mod vm_observation;
#[cfg(feature = "server")]
pub mod vm_shutdown;

#[cfg(feature = "server")]
mod transfers;

#[cfg(feature = "server")]
pub mod transfer_target;

#[cfg(feature = "server")]
pub mod process_forward;
#[cfg(feature = "server")]
mod process_peer;

#[cfg(feature = "server")]
pub mod field_process;
#[cfg(feature = "server")]
mod file_search;
#[cfg(feature = "server")]
pub mod shells;

#[cfg(feature = "server")]
pub mod shell_state;

#[cfg(feature = "server")]
pub mod fuse_namespace;
#[cfg(feature = "server")]
mod kernel_slots;
#[cfg(feature = "server")]
pub mod kernels;
#[cfg(feature = "server")]
mod tool_slots;
#[cfg(feature = "server")]
pub mod tools;
