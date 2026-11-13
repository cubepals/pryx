// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Writable ext4 images are prepared only while the monitor's kernel lease is held. Checks
//! never modify the current image; preen repairs a private whole copy and publishes it only
//! after independent read-only verification and fsync. No host mount or filesystem parser lives here.
use super::{error, filesystems::FilesystemTools};
use crate::runtime::RuntimeError;
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
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

async fn command(mut command: std::process::Command, deadline: Instant) -> Result<ExitStatus, RuntimeError> {
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut command = tokio::process::Command::from(command);
    command.kill_on_drop(true);
    tokio::time::timeout(remaining(deadline)?, command.status())
        .await
        .map_err(|_| RuntimeError::Timeout("VM filesystem tool exceeded its deadline".into()))?
        .map_err(error)
}

async fn check(tools: &FilesystemTools, disk: &Path, deadline: Instant) -> Result<bool, RuntimeError> {
    let mut check = tools.builder("e2fsck");
    check.args(["-f", "-n"]).arg(disk);
    Ok(command(check, deadline).await?.success())
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
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| error("Invalid VM disk deadline"))?;
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
        let mut repair = tools.builder("e2fsck");
        repair.args(["-f", "-p"]).arg(&candidate);
        let status = command(repair, deadline).await?;
        if !status.success() && status.code() != Some(ERRORS_CORRECTED) {
            return Err(error("VM disk needs recovery beyond safe automatic preen; current data is untouched"));
        }
    } else {
        let file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&candidate).map_err(error)?;
        file.set_len(bytes).map_err(error)?;
        let mut create = tools.builder("mke2fs");
        create.args(["-q", "-F", "-t", "ext4", "-E", "lazy_itable_init=0,lazy_journal_init=0"]).arg(&candidate);
        if !command(create, deadline).await?.success() {
            return Err(error("VM writable filesystem creation failed"));
        }
    }
    if !check(tools, &candidate, deadline).await? {
        return Err(error("Prepared VM disk failed independent read-only integrity verification"));
    }
    room(directory, free_floor)?;
    let candidate_file = owned(&candidate)?;
    if candidate_file.metadata().map_err(error)?.len() != bytes {
        return Err(error("Prepared VM disk changed its admitted size"));
    }
    candidate_file.sync_all().map_err(error)?;
    remaining(deadline)?;
    if let Some(original) = &original {
        let before = original.metadata().map_err(error)?;
        let current = fs::symlink_metadata(&path).map_err(error)?;
        if before.dev() != current.dev() || before.ino() != current.ino() {
            return Err(RuntimeError::Conflict("VM disk changed during recovery".into()));
        }
        fs::rename(&candidate, &path).map_err(error)?;
    } else {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            &candidate,
            rustix::fs::CWD,
            &path,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(error)?;
    }
    File::open(directory).and_then(|directory| directory.sync_all()).map_err(error)?;
    Ok(Prepared { path, created: original.is_none(), recovered: original.is_some() })
}
