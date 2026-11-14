// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Descriptor-relative source traversal, accounting and allocation measurement. This leaf
//! never calls destination-copy code; copy and archive operations consume its descriptors.
use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
use rustix::io::Errno;

pub use crate::protocol::Method;

/// Directories deeper than this are refused rather than walked: the descriptor count must be bounded, and
/// the walk holds a descriptor per level.
pub(crate) const MAX_DEPTH: usize = 128;
/// Entries a walk visits before it gives up: data sets hold thousands.
pub(crate) const MAX_ENTRIES: u64 = 2_000_000;
/// Bytes a tree may come to: what a restore unpacks at most (`tarball::MAX_UNPACKED_BYTES`), and
/// what `copy_tree` and `tarball::pack` take, by the sizes the files state. A data set may be several GB.
/// A sparse file states any size with no blocks behind it, which `disk_usage` doesn't see, and
/// its apparent size still counts against admission.
pub(crate) const MAX_BYTES: u64 = 256 * 1024 * 1024 * 1024;
/// How often `room` is asked within one file, besides after each: a large file, or a sparse one,
/// is stopped at the floor partway through.
pub(crate) const ROOM_STEP: u64 = 1024 * 1024;

/// What the walk saw of an entry, from the descriptor it opened (or, for a link, the link itself).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Meta {
    pub(crate) mode: u32,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) size: u64,
    pub(crate) mtime: i64,
    pub(crate) mtime_nsec: i64,
}

impl From<&rustix::fs::Stat> for Meta {
    #[allow(clippy::unnecessary_cast)] // The field types differ between architectures.
    fn from(s: &rustix::fs::Stat) -> Self {
        Self {
            mode: s.st_mode as u32,
            uid: s.st_uid as u32,
            gid: s.st_gid as u32,
            size: s.st_size.max(0) as u64,
            mtime: s.st_mtime as i64,
            mtime_nsec: s.st_mtime_nsec as i64,
        }
    }
}

pub(crate) enum Kind<'a> {
    Dir,
    /// Opened read-only, at offset 0.
    File(&'a mut File),
    /// The link's target, as stored. Never resolved.
    Link(&'a Path),
}

pub(crate) struct Entry<'a> {
    /// Relative to the root; empty for the root itself, which comes first.
    pub(crate) path: &'a Path,
    pub(crate) meta: Meta,
    pub(crate) kind: Kind<'a>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Walked {
    pub(crate) dirs: u64,
    pub files: u64,
    pub(crate) links: u64,
    /// FIFOs, sockets, devices, files under a lease, and entries that changed while the walk
    /// looked at them.
    pub(crate) skipped: u64,
    /// The regular files' sizes when they were opened.
    pub bytes: u64,
}

impl Walked {
    pub(crate) fn entries(&self) -> u64 {
        self.dirs + self.files + self.links
    }
}

/// Visits `root` and everything under it, parents before children, names in byte order. Names in
/// `exclude` are left out at the top level only. Blocking.
pub(crate) fn walk(
    root: &Path,
    exclude: &[String],
    visit: &mut dyn FnMut(Entry<'_>) -> io::Result<()>,
) -> io::Result<Walked> {
    walk_at_most(root, exclude, MAX_ENTRIES, visit)
}

/// `walk`, giving up past `max_entries`: `MAX_ENTRIES`, but for tests.
pub(super) fn walk_at_most(
    root: &Path,
    exclude: &[String],
    max_entries: u64,
    visit: &mut dyn FnMut(Entry<'_>) -> io::Result<()>,
) -> io::Result<Walked> {
    let fd = rustix::fs::open(root, dir_flags(), Mode::empty())?;
    let meta = Meta::from(&rustix::fs::fstat(&fd)?);
    let mut walked = Walked { dirs: 1, ..Walked::default() };
    visit(Entry { path: Path::new(""), meta, kind: Kind::Dir })?;
    let mut path = PathBuf::new();
    descend(&fd, &mut path, 0, exclude, max_entries, visit, &mut walked)?;
    Ok(walked)
}

fn dir_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

/// The entry changed between being listed and being opened: it is skipped, not an error.
fn vanished(e: Errno) -> bool {
    matches!(e, Errno::NOENT | Errno::LOOP | Errno::NOTDIR | Errno::NXIO)
}

fn descend(
    dir: &OwnedFd,
    path: &mut PathBuf,
    depth: usize,
    exclude: &[String],
    max_entries: u64,
    visit: &mut dyn FnMut(Entry<'_>) -> io::Result<()>,
    walked: &mut Walked,
) -> io::Result<()> {
    if depth >= MAX_DEPTH {
        return Err(io::Error::other(format!("{} nests deeper than {MAX_DEPTH} directories", path.display())));
    }
    let mut names: Vec<CString> = Vec::new();
    for entry in Dir::read_from(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == c"." || name == c".." {
            continue;
        }
        if depth == 0 && exclude.iter().any(|e| e.as_bytes() == name.to_bytes()) {
            continue;
        }
        names.push(name.to_owned());
        // Counted as the names are read, so a directory of millions can't fill memory before the
        // limit is looked at: each name read is one more entry, or skip, once visited.
        if walked.entries() + walked.skipped + names.len() as u64 > max_entries {
            return Err(io::Error::other(format!("more than {max_entries} entries")));
        }
    }
    names.sort();
    for name in names {
        if walked.entries() + walked.skipped >= max_entries {
            return Err(io::Error::other(format!("more than {max_entries} entries")));
        }
        let stat = match rustix::fs::statat(dir, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(e) if vanished(e) => {
                walked.skipped += 1;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        path.push(OsStr::from_bytes(name.to_bytes()));
        let result = match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => match rustix::fs::openat(dir, name.as_c_str(), dir_flags(), Mode::empty()) {
                Ok(child) => {
                    let meta = Meta::from(&rustix::fs::fstat(&child)?);
                    walked.dirs += 1;
                    visit(Entry { path, meta, kind: Kind::Dir })
                        .and_then(|()| descend(&child, path, depth + 1, exclude, max_entries, visit, walked))
                }
                Err(e) if vanished(e) => {
                    walked.skipped += 1;
                    Ok(())
                }
                Err(e) => Err(e.into()),
            },
            FileType::RegularFile => {
                // NONBLOCK: a FIFO swapped in after the stat would otherwise hang the open.
                let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY;
                match rustix::fs::openat(dir, name.as_c_str(), flags, Mode::empty()) {
                    Ok(fd) => {
                        let stat = rustix::fs::fstat(&fd)?;
                        if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile {
                            let meta = Meta::from(&stat);
                            walked.files += 1;
                            walked.bytes = walked.bytes.saturating_add(meta.size);
                            let mut file = File::from(fd);
                            visit(Entry { path, meta, kind: Kind::File(&mut file) })
                        } else {
                            walked.skipped += 1;
                            Ok(())
                        }
                    }
                    // WOULDBLOCK: a lease on the file holds the open off, and NONBLOCK fails it
                    // rather than wait for the lease's holder.
                    Err(e) if vanished(e) || e == Errno::WOULDBLOCK => {
                        walked.skipped += 1;
                        Ok(())
                    }
                    Err(e) => Err(e.into()),
                }
            }
            FileType::Symlink => match rustix::fs::readlinkat(dir, name.as_c_str(), Vec::new()) {
                Ok(target) => {
                    walked.links += 1;
                    let target = PathBuf::from(OsStr::from_bytes(target.as_bytes()));
                    visit(Entry { path, meta: Meta::from(&stat), kind: Kind::Link(&target) })
                }
                Err(e) if vanished(e) || e == Errno::INVAL => {
                    walked.skipped += 1;
                    Ok(())
                }
                Err(e) => Err(e.into()),
            },
            _ => {
                walked.skipped += 1;
                Ok(())
            }
        };
        path.pop();
        result?;
    }
    Ok(())
}

/// Bytes a tree occupies on disk (allocated blocks, not apparent sizes). Each directory is opened
/// relative to its parent with `O_NOFOLLOW`, and anything else is looked at where it is, never
/// opened or followed. Entries on another filesystem (a mount inside the tree) are neither counted
/// nor entered. Blocking.
pub fn disk_usage(root: &Path) -> io::Result<u64> {
    let fd = rustix::fs::open(root, dir_flags(), Mode::empty())?;
    let (mut total, device) = allocated(&rustix::fs::fstat(&fd)?);
    count(fd, device, 0, &mut total)?;
    Ok(total)
}

/// The unit of `st_blocks`, whatever the filesystem's block size (stat(2)).
pub(super) const ST_BLOCK_BYTES: u64 = 512;

/// Bytes allocated to an entry, and the filesystem it is on.
#[allow(clippy::unnecessary_cast)] // The field types differ between architectures.
fn allocated(stat: &rustix::fs::Stat) -> (u64, u64) {
    (stat.st_blocks.max(0) as u64 * ST_BLOCK_BYTES, stat.st_dev as u64)
}

fn count(dir: OwnedFd, device: u64, depth: usize, total: &mut u64) -> io::Result<()> {
    if depth >= MAX_DEPTH {
        return Err(io::Error::other(format!("a tree nests deeper than {MAX_DEPTH} directories")));
    }
    let mut entries = Dir::new(dir)?;
    while let Some(entry) = entries.read() {
        let entry = entry?;
        let name = entry.file_name();
        if name == c"." || name == c".." {
            continue;
        }
        let dir = entries.fd()?;
        let stat = match rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(e) if vanished(e) => continue,
            Err(e) => return Err(e.into()),
        };
        let (bytes, on) = allocated(&stat);
        if on != device {
            continue;
        }
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
            *total += bytes;
            continue;
        }
        // Counted and entered as opened: a directory swapped for a link since the look above
        // doesn't open, and is skipped.
        match rustix::fs::openat(dir, name, dir_flags(), Mode::empty()) {
            Ok(child) => {
                let (bytes, on) = allocated(&rustix::fs::fstat(&child)?);
                if on == device {
                    *total += bytes;
                    count(child, device, depth + 1, total)?;
                }
            }
            Err(e) if vanished(e) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Copied {
    pub walked: Walked,
    pub method: Method,
}

/// A long write stopped before the disk could fill: its caller's `room` refused, the disk being
/// under the floor the caller keeps, or the tree states more than `MAX_BYTES`. It comes back inside
/// the `io::Error` that `copy_tree` and `tarball::pack` return, so the caller can tell it from the
/// disk's own failures (`NoRoom::of`), as `tarball::UnpackError::NoRoom` is told apart.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct NoRoom(pub(crate) String);

impl NoRoom {
    /// Tells `room` the bytes written so far; its refusal stops the write.
    pub(crate) fn ask(room: &mut dyn FnMut(u64) -> Result<(), String>, written: u64) -> io::Result<()> {
        room(written).map_err(|message| io::Error::other(Self(message)))
    }

    /// What `room` said, if `e` is its refusal.
    pub(crate) fn of(e: &io::Error) -> Option<&str> {
        e.get_ref()?.downcast_ref::<Self>().map(|refused| refused.0.as_str())
    }
}

/// The bytes `copy_tree` or `tarball::pack` has taken so far, told to its caller's `room` after
/// each file and every `ROOM_STEP` within one, and held to `MAX_BYTES`.
pub(crate) struct Progress<'a> {
    room: &'a mut dyn FnMut(u64) -> Result<(), String>,
    pub(super) bytes: u64,
    pub(super) next: u64,
}

impl<'a> Progress<'a> {
    pub(crate) fn new(room: &'a mut dyn FnMut(u64) -> Result<(), String>) -> Self {
        Self { room, bytes: 0, next: ROOM_STEP }
    }

    /// Refuses a file of `size` bytes that would take the tree past `MAX_BYTES`, before any of it
    /// is read: a restore would refuse the tree anyway.
    pub(crate) fn admit(&self, path: &Path, size: u64) -> io::Result<()> {
        if self.bytes.saturating_add(size) > MAX_BYTES {
            return Err(io::Error::other(NoRoom(format!(
                "{} takes the data past {MAX_BYTES} bytes, more than a restore unpacks",
                path.display()
            ))));
        }
        Ok(())
    }

    /// Counts `n` more bytes, and asks `room` once another `ROOM_STEP` has gone by.
    pub(super) fn add(&mut self, n: u64) -> io::Result<()> {
        self.bytes = self.bytes.saturating_add(n);
        if self.bytes > MAX_BYTES {
            return Err(io::Error::other(NoRoom(format!("the data grew past {MAX_BYTES} bytes as it was read"))));
        }
        if self.bytes >= self.next { self.ask() } else { Ok(()) }
    }

    /// Tells `room` the bytes so far; its refusal stops the write.
    pub(crate) fn ask(&mut self) -> io::Result<()> {
        self.next = self.bytes.saturating_add(ROOM_STEP);
        NoRoom::ask(self.room, self.bytes)
    }

    /// `inner`, counted as it is read.
    pub(crate) fn reading<R: Read>(&mut self, inner: R) -> impl Read {
        Counted { inner, progress: self }
    }
}

struct Counted<'p, 'a, R> {
    inner: R,
    progress: &'p mut Progress<'a>,
}

impl<R: Read> Read for Counted<'_, '_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.progress.add(n as u64)?;
        Ok(n)
    }
}
