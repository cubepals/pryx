// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Walking a directory a workload can write to, without being led anywhere else.
//!
//! A sandbox's data directory belongs to whatever runs in its container: plugins and mods can
//! create, rename and replace anything in it, while pryxd reads it as root. A walk by path
//! follows a directory that was swapped for a symlink between looking and opening, and would read
//! the host's files (this node's key, say) into a backup its owner can download. So every step
//! here is taken relative to a directory already open, with `O_NOFOLLOW`:
//!
//! - a symlink is reported as a symlink and never followed;
//! - a directory or file replaced while the walk looks at it is skipped;
//! - a file under a lease (`F_SETLEASE`, which the file's owner may take) is skipped, not waited
//!   for;
//! - a FIFO, socket or device is never opened;
//! - a file is read through the descriptor that was opened and checked.
//!
//! `copy_tree` uses it for local snapshots, sharing blocks with the source (`FICLONE`) where the
//! filesystem can, and `tarball::pack` for archives. `disk_usage` measures a tree by the same
//! rules, opening only its directories.
//!
//! Parts (`tree/`):
//! - `path.rs`: descriptor-relative single-file operations for node and agent.
//! - `walk.rs`: descriptor-relative traversal and progress accounting.
//! - `copy.rs`: reflink and bounded copying through the same no-follow walk.
//! - `tests.rs`: real-filesystem traversal, admission and copy regressions.

mod copy;
#[cfg(test)]
mod tests;
mod walk;
pub use copy::{copy_tree, reflink_supported};
pub use walk::{Copied, Method, Walked, disk_usage};
pub(crate) use walk::{Kind, MAX_BYTES, MAX_ENTRIES, NoRoom, Progress, walk};

#[cfg(test)]
pub(crate) use walk::ROOM_STEP;
