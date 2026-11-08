// SPDX-License-Identifier: FSL-1.1-ALv2
//! Tool-family metadata is owned here and emitted into the agent schema for SDK/template use.
pub const VERSION: u16 = 1;
pub const KIND_ENV: &str = "PRYX_TOOL_TEMPLATE";
pub const DIRECTORY_ENV: &str = "PRYX_TOOL_DIRECTORY";
// Conventional noVNC/VNC/CDP ports stay inside the private sandbox network namespace.
pub const DESKTOP_PORT: u16 = 6080;
pub const VNC_PORT: u16 = 5900;
pub const CDP_PORT: u16 = 9222;
pub const BROWSER_PORT: u16 = 9223;
pub const MAX_IMAGE_BYTES: usize = 4 * crate::contract::CAPTURE_BYTES;
// PNG specification section 5.2.
pub(crate) const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
