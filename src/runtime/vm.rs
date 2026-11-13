// SPDX-License-Identifier: FSL-1.1-ALv2
//! Verified inputs for the VM backend. Image preparation does not register a compute tier,
//! choose an alternative isolation, or announce that a monitor is confined and ready.
//! The independent vm-image/ helper wraps OCI acquisition without changing daemon dependency features.
//!
//! Parts (`vm/`):
//! - `filesystems.rs`: verify the host-owned filesystem component before external execution.
//! - `image.rs`: bounded private-pipe contract with the independent OCI helper.
pub mod filesystems;
pub mod image;

pub fn error(problem: impl std::fmt::Display) -> super::RuntimeError {
    super::RuntimeError::Other(problem.to_string())
}
