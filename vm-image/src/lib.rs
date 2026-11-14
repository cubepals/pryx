// SPDX-License-Identifier: FSL-1.1-ALv2
//! The independent VM image helper wraps the maintained registry client and filesystem tools.
//! It does not register compute or change the daemon's dependency features and logging behavior.
//!
//! Parts (`src/`):
//! - `image.rs`: verified streaming OCI acquisition and immutable EROFS publication.
//! - `main.rs`: bounded single-request command for the daemon’s private pipe contract.
pub use pryxd::runtime::vm::{error, filesystems};
pub mod image;
