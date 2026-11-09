// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Descriptor-relative file operations shared by the node and sandbox agent. Every parent is
//! opened without following links; files are checked before truncation or streaming. This is
//! confined to one data directory, not a way to address the sandbox's image or host filesystem.

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Component, Path};

use rustix::fs::{self, AtFlags, Dir, FileType, Mode, OFlags};
use serde::Serialize;

const DIRECTORY: OFlags = OFlags::RDONLY.union(OFlags::DIRECTORY).union(OFlags::NOFOLLOW).union(OFlags::CLOEXEC);
const MAX_COMPONENTS: usize = 128;
const MAX_LIST: usize = 100_000;
/// Linux's links to this process's open descriptors, documented by proc_pid_fd(5).
const DESCRIPTORS: &str = "/proc/self/fd";

pub struct DataDir(OwnedFd, Option<(rustix::process::Uid, rustix::process::Gid)>);

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub name: String,
    pub kind: &'static str,
    pub size: u64,
    pub modified: i64,
}

fn components(path: &str) -> io::Result<Vec<&str>> {
    let mut parts = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(name) => parts.push(name.to_str().ok_or(io::ErrorKind::InvalidInput)?),
            Component::CurDir => {}
            _ => return Err(io::ErrorKind::PermissionDenied.into()),
        }
    }
    if parts.len() > MAX_COMPONENTS || path.contains('\0') {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(parts)
}

fn info(name: &str, stat: fs::Stat) -> FileInfo {
    let kind = match FileType::from_raw_mode(stat.st_mode) {
        FileType::RegularFile => "file",
        FileType::Directory => "directory",
        FileType::Symlink => "symlink",
        _ => "other",
    };
    #[allow(clippy::unnecessary_cast)]
    FileInfo { name: name.into(), kind, size: stat.st_size.max(0) as u64, modified: stat.st_mtime as i64 }
}

impl DataDir {
    /// A directory reached through the same no-follow walk, for native in-sandbox mount tools.
    pub fn directory_handle(&self, path: &str, create: bool) -> io::Result<File> {
        Ok(self.directory(&components(path)?, create)?.into())
    }
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Self(fs::open(path, DIRECTORY, Mode::empty())?, None))
    }

    pub fn open_owned(path: &Path, owner: (u32, u32)) -> io::Result<Self> {
        let mut directory = Self::open(path)?;
        directory.1 = Some((rustix::process::Uid::from_raw(owner.0), rustix::process::Gid::from_raw(owner.1)));
        Ok(directory)
    }

    fn own(&self, fd: &impl std::os::fd::AsFd) -> io::Result<()> {
        if let Some((uid, gid)) = self.1 {
            fs::fchown(fd, Some(uid), Some(gid))?;
        }
        Ok(())
    }

    fn directory(&self, parts: &[&str], create: bool) -> io::Result<OwnedFd> {
        let mut dir = rustix::io::dup(&self.0)?;
        for part in parts {
            let mut created = false;
            if create {
                match fs::mkdirat(&dir, *part, Mode::from_raw_mode(0o755)) {
                    Ok(()) => created = true,
                    Err(rustix::io::Errno::EXIST) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            dir = fs::openat(&dir, *part, DIRECTORY, Mode::empty())?;
            if created {
                self.own(&dir)?;
            }
        }
        Ok(dir)
    }

    fn parent<'a>(&self, parts: &'a [&'a str], create: bool) -> io::Result<(OwnedFd, &'a str)> {
        let (name, parents) = parts.split_last().ok_or(io::ErrorKind::PermissionDenied)?;
        Ok((self.directory(parents, create)?, name))
    }

    pub fn read(&self, path: &str) -> io::Result<File> {
        self.file(path, false)
    }

    pub fn write(&self, path: &str) -> io::Result<File> {
        self.file(path, true)
    }

    pub fn append(&self, path: &str) -> io::Result<File> {
        let parts = components(path)?;
        let (dir, name) = self.parent(&parts, true)?;
        let fd = fs::openat(
            dir,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::APPEND | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::from_raw_mode(0o600),
        )?;
        if FileType::from_raw_mode(fs::fstat(&fd)?.st_mode) != FileType::RegularFile {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        self.own(&fd)?;
        Ok(File::from(fd))
    }

    pub fn write_atomic(&self, path: &str, bytes: &[u8], durable: bool) -> io::Result<()> {
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parts = components(path)?;
        let (parent, name) = self.parent(&parts, true)?;
        let (temporary, fd) = loop {
            let temporary = format!(".pryx-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
            match fs::openat(
                &parent,
                temporary.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            ) {
                Ok(fd) => break (temporary, fd),
                Err(rustix::io::Errno::EXIST) => continue,
                Err(error) => return Err(error.into()),
            }
        };
        let result = (|| {
            self.own(&fd)?;
            let mut file = File::from(fd);
            file.write_all(bytes)?;
            if durable {
                file.sync_all()?;
            }
            fs::renameat(&parent, temporary.as_str(), &parent, name)?;
            if durable {
                fs::fsync(&parent)?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::unlinkat(&parent, temporary.as_str(), AtFlags::empty());
        }
        result
    }

    fn file(&self, path: &str, write: bool) -> io::Result<File> {
        let parts = components(path)?;
        let (dir, name) = self.parent(&parts, write)?;
        let flags = OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        let flags = flags | if write { OFlags::WRONLY | OFlags::CREATE } else { OFlags::RDONLY };
        let fd = fs::openat(dir, name, flags, Mode::from_raw_mode(0o644))?;
        if FileType::from_raw_mode(fs::fstat(&fd)?.st_mode) != FileType::RegularFile {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let file = File::from(fd);
        if write {
            self.own(&file)?;
            file.set_len(0)?;
        }
        Ok(file)
    }

    pub fn stat(&self, path: &str) -> io::Result<FileInfo> {
        let parts = components(path)?;
        if parts.is_empty() {
            return Ok(info(".", fs::fstat(&self.0)?));
        }
        let (dir, name) = self.parent(&parts, false)?;
        Ok(info(name, fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)?))
    }

    pub fn list(&self, path: &str) -> io::Result<Vec<FileInfo>> {
        let parts = components(path)?;
        let fd = self.directory(&parts, false)?;
        let mut entries = Vec::new();
        for entry in Dir::read_from(&fd)? {
            let entry = entry?;
            let name = entry.file_name().to_str().map_err(|_| io::ErrorKind::InvalidData)?;
            if name == "." || name == ".." || (parts.is_empty() && name == crate::contract::STATE_DIRECTORY) {
                continue;
            }
            let stat = fs::statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW)?;
            entries.push(info(name, stat));
            if entries.len() > MAX_LIST {
                return Err(io::Error::other("directory has too many entries"));
            }
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    pub fn mkdir(&self, path: &str) -> io::Result<()> {
        self.directory(&components(path)?, true)?;
        Ok(())
    }

    pub fn chmod(&self, path: &str, mode: u32) -> io::Result<()> {
        if mode > 0o777 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let parts = components(path)?;
        if parts.is_empty() {
            fs::fchmod(&self.0, Mode::from_raw_mode(mode))?;
            return Ok(());
        }
        let (parent, name) = self.parent(&parts, false)?;
        let target = fs::openat(parent, name, OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())?;
        if !matches!(FileType::from_raw_mode(fs::fstat(&target)?.st_mode), FileType::RegularFile | FileType::Directory)
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        // O_PATH needs no read access. Following its proc link addresses the pinned inode,
        // never a user path that could have been replaced since openat, on older kernels too.
        let pinned = format!("{DESCRIPTORS}/{}", target.as_raw_fd());
        fs::chmodat(fs::CWD, pinned.as_str(), Mode::from_raw_mode(mode), AtFlags::empty())?;
        Ok(())
    }

    pub fn directory_file(&self, path: &str) -> io::Result<File> {
        Ok(File::from(self.directory(&components(path)?, false)?))
    }

    pub fn move_to(&self, from: &str, to: &str) -> io::Result<()> {
        let from = components(from)?;
        let to = components(to)?;
        let (src, name) = self.parent(&from, false)?;
        let (dst, target) = self.parent(&to, true)?;
        fs::renameat(src, name, dst, target)?;
        Ok(())
    }

    pub fn delete(&self, path: &str) -> io::Result<()> {
        let parts = components(path)?;
        let (dir, name) = self.parent(&parts, false)?;
        let stat = fs::statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW)?;
        let flags = if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
            AtFlags::REMOVEDIR
        } else {
            AtFlags::empty()
        };
        fs::unlinkat(dir, name, flags)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::fs::symlink;

    #[test]
    fn files_remain_under_the_open_root() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::open(dir.path()).unwrap();
        data.write("one/two").unwrap().write_all(b"hello").unwrap();
        let mut read = String::new();
        data.read("one/two").unwrap().read_to_string(&mut read).unwrap();
        assert_eq!(read, "hello");
        assert_eq!(data.list("one").unwrap()[0].size, 5);
        data.move_to("one/two", "three/file").unwrap();
        assert!(data.read("one/two").is_err());
        data.delete("three/file").unwrap();
        data.delete("three").unwrap();
        for path in ["..", "one/../../etc/passwd", "/etc/passwd", "a\0b"] {
            assert!(data.write(path).is_err(), "{path}");
        }
        symlink("/etc", dir.path().join("escape")).unwrap();
        assert!(data.read("escape/passwd").is_err());
        assert!(data.write("escape/passwd").is_err());
        symlink("/etc/passwd", dir.path().join("link")).unwrap();
        assert!(data.read("link").is_err());
        assert_eq!(data.stat("link").unwrap().kind, "symlink");
    }

    #[test]
    fn a_fifo_is_refused_without_waiting_for_a_writer() {
        let dir = tempfile::tempdir().unwrap();
        fs::mknodat(fs::CWD, dir.path().join("fifo"), FileType::Fifo, Mode::from_raw_mode(0o600), 0).unwrap();
        let data = DataDir::open(dir.path()).unwrap();
        assert!(data.read("fifo").is_err());
        assert!(data.write("fifo").is_err());
    }

    #[test]
    fn chmod_restores_unreadable_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::open(dir.path()).unwrap();
        let file = data.write("file").unwrap();
        data.mkdir("directory").unwrap();
        let directory = data.directory_file("directory").unwrap();
        for (name, descriptor) in [("file", file), ("directory", directory)] {
            for restricted in [0o000, 0o200] {
                data.chmod(name, restricted).unwrap();
                let restored = data.chmod(name, 0o644);
                let mode = fs::fstat(&descriptor).unwrap().st_mode & 0o777;
                // Keep failed pre-fix attempts removable without repairing the observed result.
                fs::fchmod(&descriptor, Mode::from_raw_mode(0o755)).unwrap();
                restored.unwrap();
                assert_eq!(mode, 0o644, "{name} after {restricted:o}");
            }
        }
    }

    #[test]
    fn chmod_refuses_links_and_special_files() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("file");
        std::fs::write(&target, b"outside").unwrap();
        let original = fs::statat(fs::CWD, &target, AtFlags::empty()).unwrap().st_mode;
        symlink(&target, dir.path().join("link")).unwrap();
        symlink(outside.path(), dir.path().join("escape")).unwrap();
        fs::mknodat(fs::CWD, dir.path().join("fifo"), FileType::Fifo, Mode::from_raw_mode(0o600), 0).unwrap();
        let data = DataDir::open(dir.path()).unwrap();
        for path in ["link", "escape", "escape/file", "fifo"] {
            assert!(data.chmod(path, 0o600).is_err(), "{path}");
        }
        assert_eq!(fs::statat(fs::CWD, &target, AtFlags::empty()).unwrap().st_mode, original);
        assert_eq!(std::fs::read(&target).unwrap(), b"outside");
    }
}
