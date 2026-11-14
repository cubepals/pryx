// SPDX-License-Identifier: FSL-1.1-ALv2
//! Preserve one writable disk through root-jailer staging and recovery. The kernel owns
//! bind mounts, procfs owns mountinfo parsing, and a private record binds cleanup to exact
//! inodes after monitor retirement. UID allocation, TAP policy and monitor launch belong elsewhere.
use super::{disk, error};
use crate::runtime::RuntimeError;
use rustix::{fs::FlockOperation, mount};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

const VERSION: u16 = 1;
// Firecracker 1.18 jailer Env::new documents BASE/EXECUTABLE/ID/root; no API exports this basename.
const ROOT: &str = "root";
const RECORDS: &str = "records";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
}
impl Identity {
    fn of(info: &fs::Metadata) -> Self {
        Self { device: info.dev(), inode: info.ino() }
    }
    fn matches(&self, info: &fs::Metadata) -> bool {
        self.device == info.dev() && self.inode == info.ino()
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u16,
    id: String,
    monitor: String,
    disk: PathBuf,
    lease: PathBuf,
    identity: Identity,
    original_gid: u32,
    owner: (u32, u32),
    inputs: Vec<Identity>,
}

pub struct Request<'a> {
    pub base: &'a Path,
    pub id: &'a str,
    pub monitor: &'a Path,
    pub disk: &'a Path,
    pub lease: &'a Path,
    pub owner: (u32, u32),
    pub readonly: &'a BTreeMap<PathBuf, PathBuf>,
}
pub struct Jail {
    base: PathBuf,
    id: String,
    root: PathBuf,
    retired: bool,
}

fn root_only() -> Result<(), RuntimeError> {
    if !rustix::process::geteuid().is_root() {
        return Err(RuntimeError::Unavailable(
            "Root VM jail staging requires root mount and ownership authority".into(),
        ));
    }
    Ok(())
}
fn private(path: &Path) -> Result<(), RuntimeError> {
    super::bundle::ancestors(path).map_err(error)?;
    let info = fs::symlink_metadata(path).map_err(error)?;
    if !info.is_dir() || info.uid() != rustix::process::Uid::ROOT.as_raw() || info.mode() & 0o077 != 0 {
        return Err(error("Root jail state requires a private root-owned directory"));
    }
    Ok(())
}
fn directory(path: &Path) -> Result<(), RuntimeError> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(problem) if problem.kind() == io::ErrorKind::AlreadyExists => (),
        Err(problem) => return Err(error(problem)),
    }
    private(path)
}
fn regular(path: &Path) -> Result<File, RuntimeError> {
    super::bundle::ancestors(path.parent().ok_or_else(|| error("Missing root input parent"))?).map_err(error)?;
    let file =
        OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).map_err(error)?;
    if !file.metadata().map_err(error)?.is_file() {
        return Err(error("Root jail input must be a regular file"));
    }
    Ok(file)
}
fn lease(path: &Path) -> Result<File, RuntimeError> {
    private(path.parent().ok_or_else(|| error("Missing root lease parent"))?)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(error)?;
    let info = file.metadata().map_err(error)?;
    if !info.is_file()
        || info.uid() != rustix::process::Uid::ROOT.as_raw()
        || info.mode() & 0o077 != 0
        || info.nlink() != 1
    {
        return Err(error("Root jail lease must be one private root-owned file"));
    }
    rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|problem| {
        if problem == rustix::io::Errno::WOULDBLOCK {
            RuntimeError::Conflict("Root VM monitor still holds its lifetime lease".into())
        } else {
            error(problem)
        }
    })?;
    Ok(file)
}
fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|value| value.simple().to_string() == id)
}
fn root(base: &Path, monitor: &str, id: &str) -> Result<PathBuf, RuntimeError> {
    if !valid_id(id)
        || monitor.is_empty()
        || Path::new(monitor).components().count() != 1
        || Path::new(monitor).components().any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(error("Invalid root jail identity"));
    }
    Ok(base.join(monitor).join(id).join(ROOT))
}
fn record_path(base: &Path, id: &str) -> Result<PathBuf, RuntimeError> {
    if !valid_id(id) {
        return Err(error("Invalid root jail record identity"));
    }
    Ok(base.join(RECORDS).join(format!("{id}.json")))
}
fn save(base: &Path, record: &Record) -> Result<(), RuntimeError> {
    let bytes = serde_json::to_vec(record).map_err(error)?;
    if bytes.len() > pryx_agent::contract::MAX_REQUEST_BYTES {
        return Err(error("Root jail record exceeds its bound"));
    }
    crate::durable::write_atomic(&record_path(base, &record.id)?, &bytes, 0o600).map_err(error)
}
fn load(base: &Path, id: &str) -> Result<Record, RuntimeError> {
    private(base)?;
    private(&base.join(RECORDS))?;
    let file = regular(&record_path(base, id)?)?;
    let info = file.metadata().map_err(error)?;
    if info.uid() != rustix::process::Uid::ROOT.as_raw() || info.mode() & 0o077 != 0 || info.nlink() != 1 {
        return Err(error("Unsafe root jail record"));
    }
    let mut bytes = Vec::new();
    file.take(pryx_agent::contract::MAX_REQUEST_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(error)?;
    if bytes.len() > pryx_agent::contract::MAX_REQUEST_BYTES {
        return Err(error("Oversized root jail record"));
    }
    let record: Record = serde_json::from_slice(&bytes).map_err(|_| error("Invalid root jail record"))?;
    if record.version != VERSION
        || record.id != id
        || record.owner.0 == rustix::process::Uid::ROOT.as_raw()
        || record.owner.1 == rustix::process::Gid::ROOT.as_raw()
        || record.owner.0 == u32::MAX
        || record.owner.1 == u32::MAX
    {
        return Err(error("Invalid root jail ownership record"));
    }
    Ok(record)
}

impl Jail {
    pub fn prepare(request: Request<'_>) -> Result<Self, RuntimeError> {
        root_only()?;
        let Request { base, id, monitor, disk: source, lease: lifetime, owner, readonly } = request;
        private(base)?;
        if owner.0 == rustix::process::Uid::ROOT.as_raw()
            || owner.1 == rustix::process::Gid::ROOT.as_raw()
            || owner.0 == u32::MAX
            || owner.1 == u32::MAX
        {
            return Err(error("Root jailer requires fixed non-root monitor identities"));
        }
        let monitor =
            monitor.file_name().and_then(|name| name.to_str()).ok_or_else(|| error("Invalid monitor name"))?;
        let root = root(base, monitor, id)?;
        directory(&base.join(RECORDS))?;
        match fs::symlink_metadata(record_path(base, id)?) {
            Ok(_) => return Err(error("Root jail record already exists")),
            Err(problem) if problem.kind() == io::ErrorKind::NotFound => (),
            Err(problem) => return Err(error(problem)),
        }
        let _lease = lease(lifetime)?;
        let disk = regular(source)?;
        let info = disk.metadata().map_err(error)?;
        if info.uid() != rustix::process::Uid::ROOT.as_raw() || info.mode() & 0o077 != 0 || info.nlink() != 1 {
            return Err(error("Root jail requires one private root-owned writable disk"));
        }
        let inputs = readonly
            .values()
            .map(|path| {
                let file = regular(path)?;
                let info = file.metadata().map_err(error)?;
                if info.uid() != rustix::process::Uid::ROOT.as_raw() || info.mode() & 0o222 != 0 {
                    return Err(error("Root jail read-only input is mutable or not root-owned"));
                }
                Ok(Identity::of(&info))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let record = Record {
            version: VERSION,
            id: id.into(),
            monitor: monitor.into(),
            disk: source.into(),
            lease: lifetime.into(),
            identity: Identity::of(&info),
            original_gid: info.gid(),
            owner,
            inputs,
        };
        directory(&base.join(monitor))?;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(root.parent().ok_or_else(|| error("Missing jail parent"))?)
            .map_err(error)?;
        directory(&root)?;
        save(base, &record)?;
        if let Err(problem) = stage(&root, &record, readonly, &disk) {
            if cleanup(base, &record).is_err() {
                return Err(error(format!("Root jail staging failed; recovery is retained: {problem}")));
            }
            return Err(problem);
        }
        Ok(Self { base: base.into(), id: id.into(), root, retired: false })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Seal generated configuration after staged paths have supplied real disk identities.
    /// This runs before the jailer changes root ownership or starts the monitor.
    pub fn add_readonly(&mut self, relative: &Path, source: &Path) -> Result<(), RuntimeError> {
        root_only()?;
        private(&self.root)?;
        let mut record = load(&self.base, &self.id)?;
        let _lease = lease(&record.lease)?;
        let target = destination(&self.root, relative)?;
        match fs::symlink_metadata(&target) {
            Ok(_) => return Err(error("Root jail input destination already exists")),
            Err(problem) if problem.kind() == io::ErrorKind::NotFound => (),
            Err(problem) => return Err(error(problem)),
        }
        let file = regular(source)?;
        let info = file.metadata().map_err(error)?;
        if info.uid() != rustix::process::Uid::ROOT.as_raw() || info.mode() & 0o222 != 0 {
            return Err(error("Root jail read-only input is mutable or not root-owned"));
        }
        record.inputs.push(Identity::of(&info));
        // Recovery must already know the input if a mount or remount fails.
        save(&self.base, &record)?;
        bind(source, &target, true)
    }
    /// No Drop cleanup: the actual monitor lease, rather than object lifetime, authorizes retirement.
    pub fn retire(&mut self) -> Result<(), RuntimeError> {
        if !self.retired {
            Self::recover(&self.base, &self.id)?;
            self.retired = true;
        }
        Ok(())
    }
    pub fn recover(base: &Path, id: &str) -> Result<(), RuntimeError> {
        root_only()?;
        let record = load(base, id)?;
        let _lease = lease(&record.lease)?;
        cleanup(base, &record)
    }
}

fn bind(source: &Path, target: &Path, readonly: bool) -> Result<(), RuntimeError> {
    let parent = target.parent().ok_or_else(|| error("Missing jail input destination"))?;
    fs::create_dir_all(parent).map_err(error)?;
    OpenOptions::new().write(true).create_new(true).mode(0o600).open(target).map_err(error)?;
    mount::mount_bind(source, target).map_err(error)?;
    let mut flags =
        mount::MountFlags::BIND | mount::MountFlags::NODEV | mount::MountFlags::NOSUID | mount::MountFlags::NOEXEC;
    if readonly {
        flags |= mount::MountFlags::RDONLY;
    }
    mount::mount_remount(target, flags, "").map_err(error)
}
fn destination(root: &Path, relative: &Path) -> Result<PathBuf, RuntimeError> {
    if relative.as_os_str().is_empty()
        || relative.components().any(|part| !matches!(part, Component::Normal(_)))
        || relative == Path::new(disk::FILE)
    {
        return Err(error("Invalid root jail read-only destination"));
    }
    Ok(root.join(relative))
}
fn stage(root: &Path, record: &Record, readonly: &BTreeMap<PathBuf, PathBuf>, disk: &File) -> Result<(), RuntimeError> {
    for (relative, source) in readonly {
        bind(source, &destination(root, relative)?, true)?;
    }
    bind(&record.disk, &root.join(disk::FILE), false)?;
    std::os::unix::fs::fchown(disk, Some(record.owner.0), Some(record.owner.1)).map_err(error)
}
fn cleanup(base: &Path, record: &Record) -> Result<(), RuntimeError> {
    let root = root(base, &record.monitor, &record.id)?;
    let parent = root.parent().ok_or_else(|| error("Missing jail parent"))?;
    match fs::symlink_metadata(parent) {
        Ok(_) => private(parent)?,
        Err(problem) if problem.kind() == io::ErrorKind::NotFound => (),
        Err(problem) => return Err(error(problem)),
    }
    let disk = regular(&record.disk)?;
    let info = disk.metadata().map_err(error)?;
    if !record.identity.matches(&info)
        || info.nlink() != 1
        || info.mode() & 0o077 != 0
        || (info.uid() != rustix::process::Uid::ROOT.as_raw() && info.uid() != record.owner.0)
    {
        return Err(error("Root jail disk identity changed; ownership is not restored"));
    }
    let mut mounts = procfs::process::Process::myself().and_then(|process| process.mountinfo()).map_err(error)?.0;
    mounts.retain(|entry| entry.mount_point.starts_with(&root));
    for entry in &mounts {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&entry.mount_point)
            .map_err(error)?;
        let info = file.metadata().map_err(error)?;
        if !record.identity.matches(&info) && !record.inputs.iter().any(|input| input.matches(&info)) {
            return Err(error("Foreign mount in root jail; cleanup is refused"));
        }
    }
    mounts.sort_by_key(|entry| std::cmp::Reverse(entry.mount_point.components().count()));
    for entry in mounts {
        mount::unmount(&entry.mount_point, mount::UnmountFlags::NOFOLLOW).map_err(error)?;
    }
    match fs::remove_dir_all(parent) {
        Ok(()) => (),
        Err(problem) if problem.kind() == io::ErrorKind::NotFound => (),
        Err(problem) => return Err(error(problem)),
    }
    std::os::unix::fs::fchown(&disk, Some(rustix::process::Uid::ROOT.as_raw()), Some(record.original_gid))
        .map_err(error)?;
    disk.sync_all().map_err(error)?;
    fs::remove_file(record_path(base, &record.id)?).map_err(error)?;
    File::open(base.join(RECORDS)).and_then(|directory| directory.sync_all()).map_err(error)
}

#[cfg(test)]
#[path = "jail_tests.rs"]
mod tests;
