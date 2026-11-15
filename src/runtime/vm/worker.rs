// SPDX-License-Identifier: FSL-1.1-ALv2
//! Separate host-owned VM authority. Its private Unix transport authenticates kernel peers;
//! this foundation does not register a backend or grant the main daemon more privileges.
//!
//! Parts (`worker/`):
//! - `commands.rs`: typed runtime commands and cancellation-owned unary dispatch.
//! - `http.rs`: maintained HTTP framing, bounded control calls and one submission per operation.
//! - `transport.rs`: private socket ownership, single server lease and kernel peer admission.
pub mod commands;
pub mod http;
pub mod transport;
