// SPDX-License-Identifier: FSL-1.1-ALv2
//! Separate host-owned VM authority. Its private Unix transport authenticates kernel peers;
//! this foundation does not register a backend or grant the main daemon more privileges.
//!
//! Parts (`worker/`):
//! - `transport.rs`: private socket ownership, single server lease and kernel peer admission.
pub mod transport;
