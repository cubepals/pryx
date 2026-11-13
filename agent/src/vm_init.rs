// SPDX-License-Identifier: FSL-1.1-ALv2
//! Trusted guest PID 1 mounts verified block layers, keeps bootstrap secrets in memory, and
//! supervises an unprivileged agent. Only the guest init may mount, manage its private cgroup,
//! flush disks or power off; image commands never inherit these privileges.
use crate::{
    paths::DataDir,
    vm_boot::{self, Boot},
};
use clap::Parser;
use nix::unistd::{Gid, Uid, setgid, setgroups, sethostname, setuid};
use rustix::mount::{self, MountFlags, MountPropagationFlags};
use rustix::process::{Pid, Signal};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs::{self, File},
    io,
    os::{fd::AsRawFd, unix::process::CommandExt},
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::AsyncReadExt;
use tokio_vsock::{VMADDR_CID_HOST, VsockAddr, VsockStream};

const CGROUP: &str = "/sys/fs/cgroup/pryx";
const WORK: &str = "/run/pryx-init";
// Linux admin-guide/cgroup-v2.rst: populated=0 means no live process in this subtree.
const EMPTY_CGROUP: &str = "populated 0";

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn init_only() -> io::Result<()> {
    if rustix::process::getpid() != Pid::INIT || !rustix::process::geteuid().is_root() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "VM init requires the trusted guest PID 1"));
    }
    if fs::read(vm_boot::MARKER)? != vm_boot::marker() {
        return Err(invalid("VM init requires its pinned bootstrap image"));
    }
    Ok(())
}

fn filesystem(source: &str, target: &Path, kind: &str, flags: MountFlags, data: &str) -> io::Result<()> {
    let data = CString::new(data).map_err(|_| invalid("Invalid VM mount configuration"))?;
    mount::mount(source, target, kind, flags, Some(data.as_c_str())).map_err(Into::into)
}

fn mountpoint(root: &DataDir, name: &str) -> io::Result<File> {
    root.directory_handle(name, true)
}

fn descriptor(file: &File) -> PathBuf {
    // proc_pid_fd(5): the guest's already mounted procfs resolves an open directory.
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

fn validate(boot: &Boot) -> io::Result<()> {
    if boot.version != vm_boot::VERSION || !vm_boot::guest_cid(boot.cid) || boot.layers.len() > vm_boot::MAX_LAYERS {
        return Err(invalid("Unsupported VM bootstrap version, CID or layer count"));
    }
    if !crate::process_observation::nonce_valid(&boot.nonce) {
        return Err(invalid("VM bootstrap needs a fresh boot observation nonce"));
    }
    if boot.uid == 0 || boot.gid == 0 || boot.pids_limit == 0 || boot.temporary_bytes == 0 {
        return Err(invalid("VM workloads require non-root guest IDs and positive host limits"));
    }
    if boot.network_arguments.is_some() {
        return Err(invalid("VM guest networking is not ready in this implementation checkpoint"));
    }
    if boot.hostname.is_empty()
        || !boot.hostname.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(invalid("Invalid VM hostname"));
    }
    let data = Path::new(&boot.data);
    if !data.is_absolute()
        || data == Path::new("/")
        || data.components().any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
    {
        return Err(invalid("VM data mount must be an absolute normal path"));
    }
    for reserved in ["/dev", "/proc", "/sys", "/run", "/tmp", vm_boot::BOOTSTRAP_ROOT] {
        if data.starts_with(reserved) {
            return Err(invalid("VM data mount overlaps a trusted guest mount"));
        }
    }
    for id in boot.layers.iter().chain(std::iter::once(&boot.writes)) {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')) {
            return Err(invalid("Invalid VM block drive identity"));
        }
    }
    if boot.layers.iter().chain(std::iter::once(&boot.writes)).collect::<std::collections::BTreeSet<_>>().len()
        != boot.layers.len() + 1
    {
        return Err(invalid("Repeated VM block drive identity"));
    }
    Ok(())
}

fn devices() -> io::Result<BTreeMap<String, PathBuf>> {
    let mut devices = BTreeMap::new();
    // Linux drivers/block/virtio_blk.c installs serial on the disk's sysfs device.
    for entry in fs::read_dir("/sys/block")? {
        let entry = entry?;
        let serial = match fs::read_to_string(entry.path().join("serial")) {
            Ok(value) => value.trim().to_owned(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if devices.insert(serial, Path::new("/dev").join(entry.file_name())).is_some() {
            return Err(invalid("Repeated VM kernel block serial"));
        }
    }
    Ok(devices)
}

fn prepare(boot: &Boot) -> io::Result<File> {
    validate(boot)?;
    mount::mount_change("/", MountPropagationFlags::PRIVATE | MountPropagationFlags::REC)?;
    filesystem("tmpfs", Path::new("/run"), "tmpfs", MountFlags::NOSUID | MountFlags::NODEV, "mode=0755,size=16m")?;
    let work = DataDir::open(Path::new("/run"))?;
    work.mkdir("pryx-init")?;
    let work = DataDir::open(Path::new(WORK))?;
    let block = devices()?;
    let device = |id: &str| {
        let mut matches = block.iter().filter(|(serial, _)| !serial.is_empty() && id.starts_with(serial.as_str()));
        let path = matches.next().map(|(_, path)| path);
        if matches.next().is_some() {
            return Err(invalid("VM kernel block serial is ambiguous"));
        }
        path.and_then(|path| path.to_str()).ok_or_else(|| invalid("VM kernel lacks a configured block serial"))
    };
    filesystem(device(&boot.writes)?, Path::new("/write"), "ext4", MountFlags::NOSUID | MountFlags::NODEV, "")?;
    let sync = File::open("/write")?;
    let writes = DataDir::open(Path::new("/write"))?;
    for name in ["upper", "work", "data"] {
        writes.mkdir(name)?;
    }
    let data = writes.directory_file("data")?;
    rustix::fs::fchown(
        &data,
        Some(rustix::process::Uid::from_raw(boot.uid)),
        Some(rustix::process::Gid::from_raw(boot.gid)),
    )?;
    let mut lowers = Vec::new();
    for (index, id) in boot.layers.iter().enumerate() {
        let name = format!("layers/{index}");
        let target = mountpoint(&work, &name)?;
        filesystem(
            device(id)?,
            &descriptor(&target),
            "erofs",
            MountFlags::RDONLY | MountFlags::NODEV | MountFlags::NOSUID,
            "",
        )?;
        lowers.push(format!("{WORK}/{name}"));
    }
    if lowers.is_empty() {
        work.mkdir("empty")?;
        lowers.push(format!("{WORK}/empty"));
    }
    lowers.reverse();
    let merged = mountpoint(&work, "merged")?;
    let options = format!("lowerdir={},upperdir=/write/upper,workdir=/write/work", lowers.join(":"));
    filesystem("overlay", &descriptor(&merged), "overlay", MountFlags::NOSUID | MountFlags::NODEV, &options)?;
    let root_path = Path::new(WORK).join("merged");
    let root = DataDir::open(&root_path)?;
    let data_target = mountpoint(&root, boot.data.trim_start_matches('/'))?;
    mount::mount_bind(descriptor(&data), descriptor(&data_target))?;
    for name in ["run", "tmp"] {
        let target = mountpoint(&root, name)?;
        let mode = if name == "tmp" { "1777" } else { "0755" };
        filesystem(
            "tmpfs",
            &descriptor(&target),
            "tmpfs",
            MountFlags::NODEV | MountFlags::NOSUID,
            &format!("mode={mode},size={}", boot.temporary_bytes),
        )?;
    }
    let oldroot = mountpoint(&root, vm_boot::BOOTSTRAP_ROOT.trim_start_matches('/'))?;
    drop(oldroot);
    // All targets were opened through the no-follow directory walk. No image
    // process exists during mount preparation; proc moves last, before pivot.
    for name in ["sys", "dev", "proc"] {
        let target = mountpoint(&root, name)?;
        mount::mount_move(format!("/{name}"), descriptor(&target))?;
    }
    rustix::process::pivot_root(&root_path, root_path.join(vm_boot::BOOTSTRAP_ROOT.trim_start_matches('/')))?;
    std::env::set_current_dir("/")?;
    sethostname(&boot.hostname).map_err(io::Error::from)?;
    let dev = DataDir::open(Path::new("/dev"))?;
    let pts = mountpoint(&dev, "pts")?;
    filesystem(
        "devpts",
        &descriptor(&pts),
        "devpts",
        MountFlags::NOSUID | MountFlags::NOEXEC,
        "mode=0620,ptmxmode=0666",
    )?;
    let shared = mountpoint(&dev, "shm")?;
    filesystem(
        "tmpfs",
        &descriptor(&shared),
        "tmpfs",
        MountFlags::NOSUID | MountFlags::NODEV,
        &format!("mode=1777,size={}", boot.temporary_bytes),
    )?;
    let sys = DataDir::open(Path::new("/sys"))?;
    let cgroup = mountpoint(&sys, "fs/cgroup")?;
    filesystem(
        "cgroup2",
        &descriptor(&cgroup),
        "cgroup2",
        MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC,
        "",
    )?;
    // Linux admin-guide/cgroup-v2.rst defines these guest-owned controller files.
    fs::write("/sys/fs/cgroup/cgroup.subtree_control", "+pids")?;
    fs::create_dir(CGROUP)?;
    fs::write(Path::new(CGROUP).join("pids.max"), boot.pids_limit.to_string())?;
    crate::vm_network::prepare(&DataDir::open(Path::new("/"))?, boot.network.as_ref())?;
    if boot.read_only_root {
        mount::mount_remount("/", MountFlags::RDONLY | MountFlags::NODEV | MountFlags::NOSUID, "")?;
    }
    Ok(sync)
}

async fn bootstrap() -> io::Result<Boot> {
    let cid = vsock::get_local_cid()?;
    let mut socket = VsockStream::connect(VsockAddr::new(VMADDR_CID_HOST, vm_boot::PORT)).await?;
    if socket.peer_addr()?.cid() != VMADDR_CID_HOST || !vm_boot::guest_cid(cid) {
        return Err(invalid("VM bootstrap requires its real host-CID connection"));
    }
    let mut bytes = Vec::new();
    (&mut socket).take(vm_boot::MAX_BYTES as u64 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > vm_boot::MAX_BYTES {
        return Err(invalid("VM bootstrap exceeds its bound"));
    }
    let boot: Boot = serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid VM bootstrap configuration"))?;
    if boot.cid != cid {
        return Err(invalid("VM bootstrap CID differs from its real transport"));
    }
    Ok(boot)
}

/// Fixed launcher before any image process exists, still single-threaded and root.
pub fn agent(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if rustix::process::getppid() != Some(Pid::INIT)
        || !rustix::process::geteuid().is_root()
        || fs::read(Path::new(vm_boot::BOOTSTRAP_ROOT).join(vm_boot::MARKER.trim_start_matches('/')))?
            != vm_boot::marker()
    {
        return Err("VM agent launch requires its trusted guest init".into());
    }
    let uid: u32 = arguments.first().ok_or("Missing VM guest UID")?.parse()?;
    let gid: u32 = arguments.get(1).ok_or("Missing VM guest GID")?.parse()?;
    if uid == 0 || gid == 0 {
        return Err("VM agent requires non-root guest IDs".into());
    }
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    fs::write(Path::new(CGROUP).join("cgroup.procs"), std::process::id().to_string())?;
    setgroups(&[])?;
    setgid(Gid::from_raw(gid))?;
    setuid(Uid::from_raw(uid))?;
    rustix::thread::set_no_new_privs(true)?;
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    let executable = Path::new(vm_boot::BOOTSTRAP_ROOT).join("init");
    Err(std::process::Command::new(executable).args(&arguments[2..]).exec().into())
}

async fn supervise(boot: Boot, writes: File) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::process::ExitStatusExt;
    use tokio::signal::unix::{SignalKind, signal};
    let arguments = crate::bootstrap::Bootstrap::try_parse_from(
        std::iter::once("pryx-agent").chain(boot.arguments.iter().map(String::as_str)),
    )?;
    let (sandbox, epoch, secret_env) = arguments.vm_identity();
    let encoded = boot
        .environment
        .iter()
        .rev()
        .find(|(name, _)| name == secret_env)
        .map(|(_, value)| value)
        .ok_or("VM agent signing material is missing")?;
    let key = crate::auth::SigningKey::from_encoded(encoded)?;
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")?.trim().to_owned();
    let mut network = crate::vm_network::start(boot.network.as_ref()).await?;
    let mut term = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    rustix::system::reboot(rustix::system::RebootCommand::CadOff)?;
    let mut child = tokio::process::Command::new(Path::new(vm_boot::BOOTSTRAP_ROOT).join("init"))
        .arg(vm_boot::AGENT_MODE)
        .arg(boot.uid.to_string())
        .arg(boot.gid.to_string())
        .arg("--vm-vsock")
        .arg("--vm-cid")
        .arg(boot.cid.to_string())
        .args(&boot.arguments)
        .env_clear()
        .envs(boot.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()?;
    let mut result = tokio::select! { result = child.wait() => Some(result?),
    _ = term.recv() => None, _ = interrupt.recv() => None,
    error = crate::vm_network::failure(&mut network) => return Err(error.into()) };
    if let Some(pid) = child.id().and_then(|pid| Pid::from_raw(pid as i32)) {
        let _ = rustix::process::kill_process_group(pid, Signal::TERM);
        if let Ok(status) = tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
            result = Some(status?);
        }
    }
    fs::write(Path::new(CGROUP).join("cgroup.kill"), "1")?;
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if fs::read_to_string(Path::new(CGROUP).join("cgroup.events"))?.lines().any(|line| line == EMPTY_CGROUP) {
            break;
        }
        if tokio::time::Instant::now() >= until {
            return Err("VM command cgroup did not empty after termination".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let result = match result {
        Some(status) => status,
        None => child.wait().await?,
    };
    crate::vm_network::stop(network).await?;
    rustix::fs::syncfs(&writes)?;
    let proof = crate::vm_observation::sign(
        &key,
        crate::vm_observation::Observation {
            nonce: boot.nonce,
            sandbox: sandbox.to_owned(),
            epoch,
            kernel,
            code: result.code(),
            signal: result.signal(),
            disk_synced: true,
        },
    );
    eprintln!("\n{}{}", crate::vm_observation::PREFIX, serde_json::to_string(&proof)?);
    rustix::system::reboot(rustix::system::RebootCommand::PowerOff)?;
    Err("VM guest power-off returned unexpectedly".into())
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_only()?;
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    filesystem("proc", Path::new("/proc"), "proc", MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC, "")?;
    filesystem("sysfs", Path::new("/sys"), "sysfs", MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC, "")?;
    if !std::process::Command::new(Path::new("/").join(vm_boot::NETWORK_PROGRAM))
        .arg("--loopback-only")
        .env_clear()
        .stdin(Stdio::null())
        .status()?
        .success()
    {
        return Err("Trusted guest loopback configuration failed".into());
    }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let boot = runtime.block_on(async { tokio::time::timeout(Duration::from_secs(30), bootstrap()).await })??;
    drop(runtime);
    let writes = prepare(&boot)?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(supervise(boot, writes))
}

#[cfg(test)]
mod tests {
    #[test]
    fn init_refuses_an_ordinary_host_before_any_mount() {
        assert_ne!(rustix::process::getpid(), rustix::process::Pid::INIT);
        assert_eq!(super::init_only().unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
    }
}
