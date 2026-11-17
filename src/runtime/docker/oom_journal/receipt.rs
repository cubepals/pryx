// SPDX-License-Identifier: FSL-1.1-ALv2
//! Private durable generation receipts and cross-instance serialization. A pending attempt
//! hides its previous binding until the one managed mutation has a confirmed outcome.
use super::{RuntimeError, failure, valid_hex};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Bound {
    pub(super) started: String,
    pub(super) boot: String,
    pub(super) invocation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Pending {
    pub(super) prior_started: Option<String>,
    pub(super) boot: Option<String>,
    pub(super) begin: Option<u64>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    pub(super) bound: Option<Bound>,
    pub(super) pending: Option<Pending>,
}

pub(in crate::runtime::docker) struct Held {
    path: PathBuf,
    _lock: File,
    pub(super) receipt: Receipt,
}

fn protected(path: &Path, directory: bool, private: bool) -> Result<(), RuntimeError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| failure("cannot inspect receipt authority"))?;
    let owner = rustix::process::geteuid().as_raw();
    if (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
        || (metadata.uid() != owner && metadata.uid() != 0)
        || (private && (metadata.uid() != owner || metadata.mode() & 0o077 != 0))
        || (!private
            && metadata.mode() & 0o022 != 0
            && !(metadata.is_dir() && metadata.uid() == 0 && metadata.mode() & libc::S_ISVTX != 0))
        || (!directory && metadata.nlink() != 1)
    {
        return Err(failure("receipt storage must be private, owned and free of links"));
    }
    Ok(())
}

fn directory(state: &Path) -> Result<PathBuf, RuntimeError> {
    if !state.is_absolute() {
        return Err(failure("receipt state directory must be absolute"));
    }
    for ancestor in state.ancestors() {
        protected(ancestor, true, false)?;
    }
    let directory = state.join("docker-oom");
    match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {
            File::open(state).and_then(|file| file.sync_all()).map_err(|_| failure("cannot sync receipt directory"))?
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err(failure("cannot create private receipt directory")),
    }
    protected(&directory, true, true)?;
    Ok(directory)
}

fn index_lock(state: &Path) -> Result<File, RuntimeError> {
    let path = state.join(".docker-oom.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|_| failure("cannot open receipt index lock"))?;
    protected(&path, false, true)?;
    // This lock covers only lock-inode acquisition/retirement, never an engine call. Keeping
    // it until the retired inode is closed prevents another backend locking a replacement.
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| RuntimeError::Conflict("Docker OOM receipt index is busy".into()))?;
    Ok(lock)
}

pub(super) fn retire(state: &Path, id: &str) -> Result<(), RuntimeError> {
    if !valid_hex(id, 64) {
        return Err(failure("cannot retire a non-immutable Docker identity"));
    }
    let directory = directory(state)?;
    let _index = index_lock(state)?;
    let lock_path = directory.join(format!("{id}.lock"));
    let lock = match OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOFOLLOW).open(&lock_path) {
        Ok(lock) => lock,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(failure("cannot open retired receipt lock")),
    };
    protected(&lock_path, false, true)?;
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| RuntimeError::Conflict("Docker start receipt is still in use".into()))?;
    let receipt = directory.join(format!("{id}.json"));
    match std::fs::remove_file(receipt) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(_) => return Err(failure("cannot retire start receipt")),
    }
    std::fs::remove_file(lock_path).map_err(|_| failure("cannot retire receipt lock"))?;
    drop(lock);
    File::open(directory).and_then(|file| file.sync_all()).map_err(|_| failure("cannot sync retired receipts"))
}

impl Held {
    pub(super) fn acquire(state: &Path, id: &str) -> Result<Self, RuntimeError> {
        if !valid_hex(id, 64) {
            return Err(failure("requires an immutable Docker ID"));
        }
        let directory = directory(state)?;
        let index = index_lock(state)?;
        let lock_path = directory.join(format!("{id}.lock"));
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&lock_path)
            .map_err(|_| failure("cannot open receipt lock"))?;
        protected(&lock_path, false, true)?;
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| RuntimeError::Conflict("Docker start receipt is already in use".into()))?;
        drop(index);
        let path = directory.join(format!("{id}.json"));
        let receipt = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(&path) {
            Ok(file) => {
                protected(&path, false, true)?;
                let mut bytes = Vec::new();
                file.take(4097).read_to_end(&mut bytes).map_err(|_| failure("cannot read receipt"))?;
                if bytes.len() > 4096 {
                    return Err(failure("receipt exceeded its bound"));
                }
                serde_json::from_slice::<Receipt>(&bytes).map_err(|_| failure("invalid start receipt"))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Receipt::default(),
            Err(_) => return Err(failure("cannot open start receipt")),
        };
        if receipt.bound.as_ref().is_some_and(|bound| {
            !valid_hex(&bound.boot, 32) || !valid_hex(&bound.invocation, 32) || bound.started.len() > 64
        }) || receipt.pending.as_ref().is_some_and(|pending| {
            pending.boot.as_ref().is_some_and(|boot| !valid_hex(boot, 32))
                || pending.boot.is_some() != pending.begin.is_some()
                || pending.prior_started.as_ref().is_some_and(|started| started.len() > 64)
        }) {
            return Err(failure("invalid receipt generation identity"));
        }
        Ok(Self { path, _lock: lock, receipt })
    }

    pub(super) fn save(&self) -> Result<(), RuntimeError> {
        let bytes = serde_json::to_vec(&self.receipt).map_err(|_| failure("cannot encode start receipt"))?;
        crate::durable::write_atomic(&self.path, &bytes, 0o600)
            .map_err(|_| failure("cannot durably save start receipt"))
    }
}
