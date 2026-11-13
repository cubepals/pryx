// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Reflink or bounded file copies from the descriptor-relative tree walk. Directory traversal
//! remains in the parent module; this module never opens a source again by its path.
use super::*;
use std::os::unix::fs::PermissionsExt;

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
