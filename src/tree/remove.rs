// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Removing a tree that sealed caches or a workload's own writes may have left read-only, without
//! being led anywhere else. Not for data a workload still uses: whoever calls it has already
//! moved the tree out of every workload's reach.

use super::walk::{Kind, walk};
use std::io;
use std::path::Path;

/// Gives every directory under `root` back to its owner to change. The descriptor-relative walk
/// never follows a link, and the chmod refuses one, so nothing outside `root` changes.
pub(crate) fn unseal(root: &Path) -> io::Result<()> {
    let directory = match pryx_agent::paths::DataDir::open(root) {
        Ok(directory) => directory,
        Err(problem) if problem.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(problem) => return Err(problem),
    };
    walk(root, &[], &mut |entry| {
        if matches!(entry.kind, Kind::Dir) {
            directory.chmod(entry.path.to_str().ok_or(io::ErrorKind::InvalidData)?, 0o700)?;
        }
        Ok(())
    })?;
    Ok(())
}

/// Removes `root` and everything under it; missing is removed. std's `remove_dir_all` works from
/// open directories and unlinks a link itself, never its target. A tree too large or too odd for
/// the walk to open is still removed where its permissions already allow.
pub(crate) fn remove(root: &Path) -> io::Result<()> {
    let _ = unseal(root);
    match std::fs::remove_dir_all(root) {
        Err(problem) if problem.kind() != io::ErrorKind::NotFound => Err(problem),
        _ => Ok(()),
    }
}
