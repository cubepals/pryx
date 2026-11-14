// SPDX-License-Identifier: FSL-1.1-ALv2
//! Tool-family metadata is owned here and emitted into the agent schema for SDK/template use.
pub const VERSION: u16 = 1;
pub use crate::contract::TOOL_KIND_ENV as KIND_ENV;
pub const DIRECTORY_ENV: &str = "PRYX_TOOL_DIRECTORY";
// Conventional noVNC/VNC/CDP ports stay inside the private sandbox network namespace.
pub const DESKTOP_PORT: u16 = 6080;
pub const VNC_PORT: u16 = 5900;
pub const CDP_PORT: u16 = 9222;
pub(crate) use crate::contract::PNG;
pub use crate::contract::{BROWSER_PORT, MAX_IMAGE_BYTES};
