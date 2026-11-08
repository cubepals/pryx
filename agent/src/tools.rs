// SPDX-License-Identifier: FSL-1.1-ALv2
//! Native tools adapt established desktop, SSH, object-store and browser programs. Their
//! credentials and runtime files stay outside persistent data; host lifecycle remains external.
//!
//! Parts (`tools/`):
//! - `state.rs`: private runtime directories, capability metadata and service readiness.
//! - `command.rs`: bounded invocations through the existing process supervisor.
//! - `desktop.rs`: xdotool/scrot computer use and token-protected noVNC access.
//! - `browser.rs`: Playwright CDP access and screenshots.
//! - `ssh.rs`: one-time client keys and supervised OpenSSH sessions.
//! - `mounts.rs`: native FUSE adapters and public mount descriptions.
//! - `api.rs`: scoped HTTP routes for these tool families.
//! - `types.rs`: Rust-owned request/response schemas and tool defaults.
//! - `defaults.rs`: schema-emitted native tool constants.
mod api;
mod browser;
mod command;
pub mod defaults;
mod desktop;
mod mounts;
mod ssh;
mod state;
pub mod types;
#[cfg(feature = "mcp")]
pub(crate) use api::mcp;
pub(crate) use api::routes;
pub(crate) use state::prepare;

pub use defaults::VERSION;

pub(crate) use state::wait_mounts;

pub(crate) use state::mounts_pending;
