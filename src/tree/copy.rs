// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Reflink or bounded file copies from the descriptor-relative tree walk. Directory traversal
//! remains in the parent module; this module never opens a source again by its path.
use super::*;
use rustix::fs::{SeekFrom, seek};
use std::os::unix::fs::PermissionsExt;

impl Progress<'_> {
    /// Keep descriptor copying inside this module; the walker never calls a child
    /// which imports its progress and traversal types back from the parent.
    fn copy(&mut self, source: &mut File, target: &File) -> io::Result<()> {
        file(self, source, target)
    }
}

/// The filesystem can't share blocks between these two files; copying is the only way.
fn no_reflink(e: Errno) -> bool {
    matches!(e, Errno::OPNOTSUPP | Errno::XDEV | Errno::NOTTY | Errno::NOSYS | Errno::PERM)
}

/// Copies `src` to `dst`, which must not exist yet. Files share blocks with the source where the
/// filesystem can (XFS with reflink, btrfs), and are copied where it can't (ext4). Symlinks are
/// copied as symlinks; set-id bits and others' write are dropped, as an unpack drops them.
/// Everything is given to `owner`, or keeps the source's owner without one. Blocking, at idle
/// priority (`priority::Idle`).
///
/// After each file, and every `ROOM_STEP` bytes within one, `room` is told the bytes copied so far,
/// shared or not, and an `Err` from it stops the copy (`NoRoom`): the caller keeps the disk above
/// its floor this way. A tree that states more than `MAX_BYTES` is stopped the same way, before
/// the file that takes it past is copied. What was copied until then stays for the caller to
/// remove.
///
/// `dst` must lie where the workload can't reach (a snapshot, a restore's staging directory):
/// it is written by path.
pub fn copy_tree(
    src: &Path,
    dst: &Path,
    owner: Option<(u32, u32)>,
    exclude: &[String],
    room: &mut dyn FnMut(u64) -> Result<(), String>,
) -> io::Result<Copied> {
    let _idle = crate::priority::Idle::enter();
    let (mut reflink, mut cloned, mut copied) = (true, false, false);
    let mut progress = Progress::new(room);
    let walked = walk(src, exclude, &mut |entry| {
        let target = dst.join(entry.path);
        let (uid, gid) = owner.unwrap_or((entry.meta.uid, entry.meta.gid));
        match entry.kind {
            Kind::Dir => {
                let mode = if entry.path.as_os_str().is_empty() { 0o750 } else { 0o755 };
                fs::DirBuilder::new().mode(mode).create(&target)?;
                std::os::unix::fs::lchown(&target, Some(uid), Some(gid))
            }
            Kind::File(file) => {
                progress.admit(entry.path, entry.meta.size)?;
                let out = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
                    .mode(0o600)
                    .open(&target)?;
                let mut done = false;
                if reflink {
                    match rustix::fs::ioctl_ficlone(&out, &*file) {
                        Ok(()) => {
                            done = true;
                            cloned = true;
                            progress.add(entry.meta.size)?;
                        }
                        Err(e) if no_reflink(e) => reflink = false,
                        // Some files can't be shared where others can (EINVAL); copy just this one.
                        Err(_) => {}
                    }
                }
                if !done {
                    progress.copy(file, &out)?;
                    copied = true;
                }
                std::os::unix::fs::fchown(&out, Some(uid), Some(gid))?;
                out.set_permissions(fs::Permissions::from_mode(entry.meta.mode & 0o755))?;
                let at = Timespec { tv_sec: entry.meta.mtime, tv_nsec: entry.meta.mtime_nsec as _ };
                rustix::fs::futimens(&out, &Timestamps { last_access: at, last_modification: at })?;
                progress.ask()
            }
            Kind::Link(to) => {
                std::os::unix::fs::symlink(to, &target)?;
                std::os::unix::fs::lchown(&target, Some(uid), Some(gid))
            }
        }
    })?;
    let method = if cloned && !copied { Method::Reflink } else { Method::Copy };
    Ok(Copied { walked, method })
}

/// Whether files in `dir` can share blocks: a probe of two small files. Blocking.
pub fn reflink_supported(dir: &Path) -> bool {
    let probe = dir.join(format!(".reflink-probe-{}", uuid::Uuid::new_v4()));
    let (a, b) = (probe.with_extension("a"), probe.with_extension("b"));
    let result = (|| -> io::Result<bool> {
        fs::write(&a, vec![0x5a; 64 * 1024])?;
        let src = File::open(&a)?;
        let dst = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&b)?;
        Ok(rustix::fs::ioctl_ficlone(&dst, &src).is_ok())
    })();
    let _ = fs::remove_file(&a);
    let _ = fs::remove_file(&b);
    result.unwrap_or(false)
}

/// Copy holes with the kernel's own extent lookup, falling back when it is unsupported.
/// Count apparent bytes in ROOM_STEP increments, including holes, so existing admission
/// bounds and cancellation points still apply even when little disk space is allocated.
fn file(progress: &mut Progress<'_>, source: &mut File, mut target: &File) -> io::Result<()> {
    let size = source.metadata()?.len();
    let first = match seek(&*source, SeekFrom::Data(0)) {
        Ok(position) => Some(position),
        Err(Errno::NXIO) => None,
        Err(Errno::INVAL | Errno::OPNOTSUPP) => {
            seek(&*source, SeekFrom::Start(0))?;
            loop {
                match io::copy(&mut (&mut *source).take(ROOM_STEP), &mut target)? {
                    0 => return Ok(()),
                    n => progress.add(n)?,
                }
            }
        }
        Err(error) => return Err(error.into()),
    };
    let mut position = 0;
    let mut data = first;
    while position < size {
        let next = data.unwrap_or(size).min(size);
        while position < next {
            let hole = (next - position).min(progress.next - progress.bytes);
            position += hole;
            target.set_len(position)?;
            progress.add(hole)?;
        }
        let Some(start) = data.filter(|start| *start < size) else { break };
        let end = seek(&*source, SeekFrom::Hole(start))?.min(size);
        if start < position || end <= start {
            return Err(io::Error::other("File extents changed during copy"));
        }
        seek(&*source, SeekFrom::Start(start))?;
        seek(target, SeekFrom::Start(start))?;
        position = start;
        while position < end {
            let limit = (end - position).min(progress.next - progress.bytes);
            let written = io::copy(&mut (&mut *source).take(limit), &mut target)?;
            if written != limit {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            position += written;
            progress.add(written)?;
        }
        data = if position == size {
            None
        } else {
            match seek(&*source, SeekFrom::Data(position)) {
                Ok(next) if next >= position => Some(next),
                Err(Errno::NXIO) => None,
                Ok(_) => return Err(io::Error::other("File extent moved backwards")),
                Err(error) => return Err(error.into()),
            }
        };
    }
    if source.metadata()?.len() != size {
        return Err(io::Error::other("File size changed during copy"));
    }
    target.set_len(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{FileExt, MetadataExt};

    #[test]
    fn descriptor_copy_preserves_bytes_and_kernel_reported_holes_without_reflink() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("sparse");
        let output = directory.path().join("copy");
        let mut source = OpenOptions::new().read(true).write(true).create_new(true).open(&input).unwrap();
        let size = 16 * ROOM_STEP;
        source.set_len(size).unwrap();
        let middle = (0..=u8::MAX).cycle().take(64 * 1024).collect::<Vec<_>>();
        source.write_all_at(&middle, size / 2).unwrap();
        source.write_all_at(b"last", size - 4).unwrap();
        let first = seek(&source, SeekFrom::Data(0));
        let target = OpenOptions::new().write(true).create_new(true).open(&output).unwrap();
        let mut told = Vec::new();
        let mut room = |bytes| {
            told.push(bytes);
            Ok(())
        };
        {
            let mut progress = Progress::new(&mut room);
            file(&mut progress, &mut source, &target).unwrap();
            progress.ask().unwrap();
        }
        assert_eq!(fs::read(input).unwrap(), fs::read(output).unwrap());
        assert_eq!(told.last(), Some(&size));
        assert!(told.windows(2).all(|pair| pair[1] - pair[0] <= ROOM_STEP));
        if first.is_ok_and(|offset| offset > 0) {
            assert!(
                target.metadata().unwrap().blocks() * ST_BLOCK_BYTES < size / 4,
                "The kernel reports holes, so the fallback must preserve them"
            );
        }
    }
}
