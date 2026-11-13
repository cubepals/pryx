// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Bootstrap names belong to the agent contract. None of these constants contains a credential.

pub const AGENT_PORT: u16 = 5280;
pub const MAX_SPARE_PORTS: u16 = 32;
pub const BINARY_PATH: &str = "/.pryx-agent";
pub const KEY_LABEL: &str = "blocklyd.agent-key-name";
pub const SECRET_ENV_PREFIX: &str = "PRYX_AGENT_SECRET_";
pub const INTERNAL_PORT_PREFIX: &str = "__pryx_";
pub const AGENT_PORT_NAME: &str = "__pryx_agent";
pub const DEFAULT_BINARY: &str = "/usr/local/lib/pryx/pryx-agent";
pub const FORWARD_HEADER: &str = "x-pryx-target-port";
pub const IDEMPOTENCY_HEADER: &str = "idempotency-key";
pub const CAPTURE_BYTES: usize = 1024 * 1024;
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const STATE_DIRECTORY: &str = ".pryx-agent";

/// Convenience routes require this version in the authenticated health response.
pub const API_VERSION: u16 = 2;

/// Bounded convenience operations retain no more than these filesystem resources.
pub const ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
pub const SEARCH_BYTES: u64 = 64 * 1024 * 1024;
pub const SEARCH_ENTRIES: usize = 100_000;
pub const ARCHIVE_CONCURRENCY: usize = 2;

/// Maximum client wait bounds detached readiness and shell response work.
pub const MAX_WAIT: std::time::Duration = std::time::Duration::from_hours(1);

/// Set only by the native host's opt-in FUSE policy; never grants a host-namespace capability.
pub const FUSE_ENV: &str = "PRYX_NATIVE_FUSE";
pub const FUSE_FEATURE: &str = "native-fuse-mounts";
pub const MOUNTS_ENV: &str = "PRYX_BOOTSTRAP_MOUNTS";
pub const TOOL_KIND_ENV: &str = "PRYX_TOOL_TEMPLATE";
pub const SSH_FORWARD_HEADER: &str = "x-pryx-ssh-session";
pub const WEBSOCKET_PROTOCOL: &str = "pryx";

/// Native browser proxy and bounded service/image budgets; templates retain these values.
pub const BROWSER_PORT: u16 = 9223;
pub const TOOL_STARTUP: std::time::Duration = std::time::Duration::from_secs(30);
pub const MAX_IMAGE_BYTES: usize = 4 * CAPTURE_BYTES;
// PNG specification section 5.2.
pub(crate) const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
