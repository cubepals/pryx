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
pub const CAPTURE_BYTES: usize = 1024 * 1024;
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
