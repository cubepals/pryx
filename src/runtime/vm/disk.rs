// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Writable ext4 images are prepared only while the monitor's kernel lease is held. Checks
//! never modify the current image; preen repairs a private whole copy and publishes it only
//! after independent read-only verification and fsync. No host mount or filesystem parser lives here;
//! e2fsprogs reads a guest-written disk only under the launcher's filesystem-tool confinement.
use super::{
    error,
    filesystems::{FilesystemTools, Run},
};
use crate::runtime::RuntimeError;
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::ExitStatus,
    time::{Duration, Instant},
};

pub const FILE: &str = "vm.ext4";
// e2fsprogs 1.47.4 e2fsck/e2fsck.h: FSCK_NONDESTRUCT, documented in e2fsck(8).
const ERRORS_CORRECTED: i32 = 1;

pub struct Prepared {
    pub path: PathBuf,
    pub created: bool,
    pub recovered: bool,
}

fn owned(path: &Path) -> Result<File, RuntimeError> {
    let file =
        OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).map_err(error)?;
    let info = file.metadata().map_err(error)?;
    if !info.is_file()
        || info.uid() != rustix::process::geteuid().as_raw()
        || info.mode() & 0o077 != 0
        || info.nlink() != 1
    {
        return Err(error("VM disk must be one private owned regular file"));
    }
    Ok(file)
}

// A bound on a stuck program, not an expected time: the slowest shared disk's sequential floor.
const WHOLE_DISK_FLOOR_BYTES_PER_SECOND: u64 = 16 * 1024 * 1024;
// Check, staged copy, repair and recheck each read the whole disk once.
const WHOLE_DISK_PASSES: u64 = 4;

/// Recovery's own bound, from the disk's size: a caller's boot deadline cannot end a repair
/// that a large disk needs, and the transaction keeps the longer of the two.
pub fn recovery_time(bytes: u64) -> Duration {
    Duration::from_secs(bytes.div_ceil(WHOLE_DISK_FLOOR_BYTES_PER_SECOND).saturating_mul(WHOLE_DISK_PASSES))
}

/// Free space a staged copy needs: the host's floor plus the blocks the disk actually holds.
fn staging_room(original: &File, free_floor: u64) -> Result<u64, RuntimeError> {
    // st_blocks counts 512-byte units whatever the filesystem's block size (stat(2)).
    const STAT_BLOCK_BYTES: u64 = 512;
    let allocated = original.metadata().map_err(error)?.blocks().saturating_mul(STAT_BLOCK_BYTES);
    Ok(free_floor.saturating_add(allocated))
}

fn remaining(deadline: Instant) -> Result<Duration, RuntimeError> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        Err(RuntimeError::Timeout("VM disk preparation exceeded its deadline".into()))
    } else {
        Ok(left)
    }
}

fn room(path: &Path, floor: u64) -> Result<(), RuntimeError> {
    match crate::host::disk(path) {
        Some((_, available)) if available >= floor => Ok(()),
        Some(_) => Err(error("VM disk preparation reached the host's free-space floor")),
        None => Err(error("VM disk preparation cannot measure available host storage")),
    }
}

// e2fsprogs' own diagnostics are bounded and discarded; the exit status decides.
const DIAGNOSTIC_BYTES: u64 = 64 * 1024;

fn text(path: &Path) -> Result<String, RuntimeError> {
    path.to_str().map(String::from).ok_or_else(|| error("VM disk path is not text"))
}

/// A guest wrote this disk: every e2fsprogs run is confined to it and its staged copy.
async fn tool(
    tools: &FilesystemTools,
    run: Run<'_>,
    disk: &Path,
    deadline: Instant,
) -> Result<ExitStatus, RuntimeError> {
    // Working files live beside the data directory, never inside it.
    let scratch =
        disk.parent().and_then(Path::parent).ok_or_else(|| error("VM disk lacks a private scratch parent"))?;
    remaining(deadline)?;
    let deadline = tokio::time::Instant::from_std(deadline);
    Ok(tools.run(run, scratch, Some(deadline), DIAGNOSTIC_BYTES).await?.status)
}

async fn check(tools: &FilesystemTools, disk: &Path, deadline: Instant) -> Result<bool, RuntimeError> {
    let run = Run {
        program: "e2fsck",
        arguments: vec!["-f".into(), "-n".into(), text(disk)?],
        read: vec![disk.to_owned()],
        write: Vec::new(),
        // It has no write grant; its written-file bound is the disk's own size, as a repair's.
        file_bytes: fs::metadata(disk).map_err(error)?.len().max(1),
    };
    Ok(tool(tools, run, disk, deadline).await?.success())
}

/// Caller holds the monitor's lifetime lease while verifying and copying this image.
pub async fn verify(tools: &FilesystemTools, path: &Path, timeout: Duration) -> Result<(), RuntimeError> {
    let _file = owned(path)?;
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| error("Invalid VM disk deadline"))?;
    if check(tools, path, deadline).await? {
        Ok(())
    } else {
        Err(error("VM copy requires an independently verified stopped disk"))
    }
}

/// Own the transaction until completion even if the requesting future is cancelled. Paths,
/// sizes and tool identity are host-owned inputs admitted before this call, never guest paths.
pub async fn prepare(
    tools: &FilesystemTools,
    directory: &Path,
    lease: &Path,
    bytes: u64,
    timeout: Duration,
    free_floor: u64,
) -> Result<Prepared, RuntimeError> {
    let (tools, directory, lease) = (tools.clone(), directory.to_owned(), lease.to_owned());
    tokio::spawn(async move { transaction(&tools, &directory, &lease, bytes, timeout, free_floor).await })
        .await
        .map_err(error)?
}

async fn transaction(
    tools: &FilesystemTools,
    directory: &Path,
    lease: &Path,
    bytes: u64,
    timeout: Duration,
    free_floor: u64,
) -> Result<Prepared, RuntimeError> {
    let deadline = Instant::now()
        .checked_add(timeout.max(recovery_time(bytes)))
        .ok_or_else(|| error("Invalid VM disk deadline"))?;
    remaining(deadline)?;
    if bytes == 0 || bytes > crate::tree::MAX_BYTES {
        return Err(error("VM disk size exceeds the existing data archive bound"));
    }
    super::bundle::ancestors(directory).map_err(error)?;
    let parent = directory.parent().ok_or_else(|| error("VM disk lacks a staging parent"))?;
    super::bundle::ancestors(lease.parent().ok_or_else(|| error("VM monitor lease lacks its private directory"))?)
        .map_err(error)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(lease)
        .map_err(error)?;
    let info = lock.metadata().map_err(error)?;
    if !info.is_file()
        || info.uid() != rustix::process::geteuid().as_raw()
        || info.mode() & 0o077 != 0
        || info.nlink() != 1
    {
        return Err(error("Unsafe VM monitor lease"));
    }
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).map_err(|problem| {
        if problem == rustix::io::Errno::WOULDBLOCK {
            RuntimeError::Conflict("VM monitor still owns its writable disk".into())
        } else {
            error(problem)
        }
    })?;
    for entry in fs::read_dir(directory).map_err(error)? {
        if entry.map_err(error)?.file_name() != FILE {
            return Err(error("VM data directory contains an unrecognized file"));
        }
    }
    room(directory, free_floor)?;
    let path = directory.join(FILE);
    let original = match fs::symlink_metadata(&path) {
        Ok(_) => Some(owned(&path)?),
        Err(problem) if problem.kind() == io::ErrorKind::NotFound => None,
        Err(problem) => return Err(error(problem)),
    };
    if let Some(original) = &original {
        if original.metadata().map_err(error)?.len() != bytes {
            return Err(error("VM disk resizing requires an explicit verified resize transaction"));
        }
        if check(tools, &path, deadline).await? {
            return Ok(Prepared { path, created: false, recovered: false });
        }
    }
    if let Some(original) = &original {
        // Refused before copying, never by filling the host part-way through the copy.
        room(directory, staging_room(original, free_floor)?)?;
    }
    let staging = tempfile::Builder::new().prefix(".vm-disk-stage-").tempdir_in(parent).map_err(error)?;
    let candidate = staging.path().join("disk");
    if original.is_some() {
        let source = directory.to_owned();
        let destination = staging.path().join("copy");
        let copied_path = destination.clone();
        let copied = tokio::task::spawn_blocking(move || {
            crate::tree::copy_tree(&source, &destination, None, &[], &mut |_| {
                remaining(deadline).and_then(|_| room(&source, free_floor)).map_err(|problem| problem.to_string())
            })
        })
        .await
        .map_err(error)?;
        remaining(deadline)?;
        let copied = copied.map_err(error)?;
        if copied.walked.skipped != 0 || copied.walked.files != 1 || copied.walked.links != 0 {
            return Err(error("VM disk recovery could not copy the complete image"));
        }
        fs::rename(copied_path.join(FILE), &candidate).map_err(error)?;
        let repair = Run {
            program: "e2fsck",
            arguments: vec!["-f".into(), "-p".into(), text(&candidate)?],
            read: Vec::new(),
            write: vec![candidate.clone()],
            file_bytes: bytes,
        };
        let status = tool(tools, repair, &candidate, deadline).await?;
        if !status.success() && status.code() != Some(ERRORS_CORRECTED) {
            return Err(error("VM disk needs recovery beyond safe automatic preen; current data is untouched"));
        }
    } else {
        create(tools, &candidate, bytes, deadline).await?;
    }
    if !check(tools, &candidate, deadline).await? {
        return Err(error("Prepared VM disk failed independent read-only integrity verification"));
    }
    room(directory, free_floor)?;
    publish(original.as_ref(), &candidate, &path, directory, bytes, deadline)?;
    Ok(Prepared { path, created: original.is_none(), recovered: original.is_some() })
}

/// A new sparse file of the admitted size, formatted in place by the confined mke2fs.
async fn create(tools: &FilesystemTools, candidate: &Path, bytes: u64, deadline: Instant) -> Result<(), RuntimeError> {
    let file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(candidate).map_err(error)?;
    file.set_len(bytes).map_err(error)?;
    let arguments = ["-q", "-F", "-t", "ext4", "-E", "lazy_itable_init=0,lazy_journal_init=0"];
    let create = Run {
        program: "mke2fs",
        arguments: arguments.into_iter().map(String::from).chain([text(candidate)?]).collect(),
        read: Vec::new(),
        write: vec![candidate.to_owned()],
        file_bytes: bytes,
    };
    if !tool(tools, create, candidate, deadline).await?.success() {
        return Err(error("VM writable filesystem creation failed"));
    }
    Ok(())
}

fn publish(
    original: Option<&File>,
    candidate: &Path,
    path: &Path,
    directory: &Path,
    bytes: u64,
    deadline: Instant,
) -> Result<(), RuntimeError> {
    let candidate_file = owned(candidate)?;
    if candidate_file.metadata().map_err(error)?.len() != bytes {
        return Err(error("Prepared VM disk changed its admitted size"));
    }
    candidate_file.sync_all().map_err(error)?;
    remaining(deadline)?;
    if let Some(original) = original {
        let before = original.metadata().map_err(error)?;
        let current = fs::symlink_metadata(path).map_err(error)?;
        if before.dev() != current.dev() || before.ino() != current.ino() {
            return Err(RuntimeError::Conflict("VM disk changed during recovery".into()));
        }
        fs::rename(candidate, path).map_err(error)?;
    } else {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            candidate,
            rustix::fs::CWD,
            path,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(error)?;
    }
    File::open(directory).and_then(|directory| directory.sync_all()).map_err(error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn recovery_outlasts_a_boot_deadline_on_a_large_disk_and_reserves_its_copy() {
        let large = 100 * 1024 * 1024 * 1024;
        assert!(recovery_time(large) > Duration::from_hours(1));
        assert!(recovery_time(large) > recovery_time(large / 100));
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(FILE);
        let mut file = File::create(&path).unwrap();
        file.write_all(&[7; 1024 * 1024]).unwrap();
        file.sync_all().unwrap();
        // A sparse tail holds no blocks, so it needs no staging room.
        file.set_len(64 * 1024 * 1024).unwrap();
        let needed = staging_room(&File::open(&path).unwrap(), 4096).unwrap();
        assert!((4096 + 1024 * 1024..4096 + 2 * 1024 * 1024).contains(&needed), "{needed}");
    }
}
