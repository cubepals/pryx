// SPDX-License-Identifier: FSL-1.1-ALv2
//! Hold a node-private kernel lock across exec of the verified sidecar. A replacement daemon
//! cannot create VMs while its predecessor's sidecar can still run, even after an abrupt exit.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::os::unix::{
    fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    process::CommandExt,
};
use std::path::{Path, PathBuf};

const NAME: &str = "pryx-agentos-sidecar";
const CONFIG: &str = "launcher.json";

#[derive(Serialize, Deserialize)]
struct Config {
    binary: PathBuf,
    lock: PathBuf,
    home: PathBuf,
    owner: (u32, u32),
}

pub(super) fn private_directory(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        std::fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

pub(super) fn prepare(
    root: &Path,
    binary: &Path,
    executable: Option<&Path>,
    owner: (u32, u32),
) -> Result<PathBuf, super::RuntimeError> {
    private_directory(root).map_err(super::error)?;
    let home = root.parent().ok_or_else(|| super::error("Missing wasm state parent"))?.join("wasm-worker-home");
    worker_directory(&home, owner).map_err(super::error)?;
    worker_directory(&home.join("tmp"), owner).map_err(super::error)?;
    let config = Config { binary: binary.into(), lock: root.join("sidecar.lock"), home, owner };
    crate::durable::write_atomic(&root.join(CONFIG), &serde_json::to_vec(&config).map_err(super::error)?, 0o600)
        .map_err(super::error)?;
    let executable =
        executable.map(Path::to_path_buf).map(Ok).unwrap_or_else(std::env::current_exe).map_err(super::error)?;
    let path = root.join(NAME);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata)
            if metadata.file_type().is_symlink() && std::fs::read_link(&path).map_err(super::error)? == executable => {}
        Ok(_) => return Err(super::error("Unsafe agentOS launcher path")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::os::unix::fs::symlink(executable, &path).map_err(super::error)?;
            File::open(root).and_then(|f| f.sync_all()).map_err(super::error)?;
        }
        Err(e) => return Err(super::error(e)),
    }
    Ok(path)
}

pub(super) fn invoked() -> bool {
    std::env::args_os().next().and_then(|p| PathBuf::from(p).file_name().map(|s| s == NAME)).unwrap_or(false)
}

pub(super) fn execute() -> std::io::Result<std::convert::Infallible> {
    let parent = rustix::process::getppid();
    let invoked = PathBuf::from(std::env::args_os().next().ok_or(std::io::ErrorKind::InvalidInput)?);
    let root = invoked.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    private_directory(root)?;
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(root.join(CONFIG))?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.len() > 8192
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    let config: Config = serde_json::from_reader(file)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&config.lock)?;
    let metadata = lock.metadata()?;
    if !metadata.is_file() || metadata.mode() & 0o077 != 0 || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)?;
    let flags = rustix::io::fcntl_getfd(&lock)?;
    rustix::io::fcntl_setfd(&lock, flags.difference(rustix::io::FdFlags::CLOEXEC))?;
    if rustix::process::geteuid().is_root() {
        rustix::thread::set_thread_groups(&[])?;
        rustix::thread::set_thread_gid(rustix::process::Gid::from_raw(config.owner.1))?;
        rustix::thread::set_thread_uid(rustix::process::Uid::from_raw(config.owner.0))?;
    }
    rustix::thread::set_no_new_privs(true)?;
    rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL))?;
    if parent.is_none() || rustix::process::getppid() != parent {
        return Err(std::io::ErrorKind::Interrupted.into());
    }
    let mut command = std::process::Command::new(config.binary);
    let environment = [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "NODE_EXTRA_CA_CERTS",
    ];
    let selected =
        environment.into_iter().filter_map(|key| std::env::var_os(key).map(|value| (key, value))).collect::<Vec<_>>();
    command.env_clear().envs(selected).env("HOME", &config.home).env("TMPDIR", config.home.join("tmp"));
    Err(command.args(std::env::args_os().skip(1)).exec())
}

pub(super) fn worker_directory(path: &Path, owner: (u32, u32)) -> std::io::Result<()> {
    let created = match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(e) => return Err(e),
    };
    let directory = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    if created {
        rustix::fs::fchown(
            &directory,
            Some(rustix::process::Uid::from_raw(owner.0)),
            Some(rustix::process::Gid::from_raw(owner.1)),
        )?;
    }
    let metadata = rustix::fs::fstat(&directory)?;
    if metadata.st_uid != owner.0 || metadata.st_gid != owner.1 || metadata.st_mode & 0o077 != 0 {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

pub(super) fn traverse(path: &Path, state: &Path, owner: (u32, u32)) -> std::io::Result<()> {
    if owner.0 == rustix::process::geteuid().as_raw() {
        return Ok(());
    }
    for parent in path.ancestors() {
        let metadata = std::fs::symlink_metadata(parent)?;
        if !metadata.is_dir() {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        if parent.starts_with(state) {
            if metadata.uid() != owner.0 {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o711))?;
            }
        } else if metadata.mode() & 0o001 == 0 {
            return Err(std::io::Error::other("wasm state parents must allow worker traversal"));
        }
    }
    Ok(())
}
