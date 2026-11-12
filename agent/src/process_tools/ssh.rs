// SPDX-License-Identifier: FSL-1.1-ALv2
//! A closed native OpenSSH worker serves only accepted private Unix descriptors. Bubblewrap
//! supplies private accounts/proc/tmp; the ordinary guest policy precedes every server/shell.
use crate::process_policy::Policy;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    os::{
        linux::net::SocketAddrExt,
        unix::{
            fs::PermissionsExt,
            net::{SocketAddr, UnixListener},
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

pub const PRIVATE_DIRECTORY: &str = "/tmp/pryx-ssh";
pub const PRIVATE_SOCKET: &str = "/tmp/pryx-ssh/server.sock";
#[derive(Clone, Deserialize, Serialize)]
pub struct Launch {
    pub directory: PathBuf,
    pub home_mount: PathBuf,
    pub address: String,
}
#[derive(Deserialize, Serialize)]
struct Input {
    policy: Policy,
    launch: Launch,
    bundle: PathBuf,
}

pub fn start_at(policy: &Policy, launch: Launch, bundle: &Path, executable: &Path) -> io::Result<Child> {
    let tools = policy.tools.as_ref().ok_or_else(|| io::Error::other("Missing native SSH policy"))?;
    if !tools.browser_on {
        return Err(io::Error::other("Native SSH requires usable unprivileged user namespaces"));
    }
    if rustix::process::geteuid().is_root() {
        return Err(io::Error::other("Native SSH requires a non-root sandbox user"));
    }
    if !fs::canonicalize(&launch.directory)?.starts_with(fs::canonicalize(&tools.scratch)?)
        || !fs::canonicalize(bundle)?.starts_with(fs::canonicalize(&tools.cache)?)
        || !fs::canonicalize(&launch.home_mount)?.starts_with(fs::canonicalize(&policy.data)?)
    {
        return Err(io::Error::other("Native SSH paths escape their host-owned roots"));
    }
    let home = policy.data.to_string_lossy();
    let shell = bundle.join("desktop/adapter/ssh-shell");
    if home.contains([':', '\r', '\n']) || shell.to_string_lossy().contains([':', '\r', '\n']) {
        return Err(io::Error::other("Native SSH paths cannot be represented in a private account record"));
    }
    let uid = rustix::process::geteuid().as_raw();
    let gid = rustix::process::getegid().as_raw();
    fs::write(
        launch.directory.join("passwd"),
        format!("pryx:x:{uid}:{gid}:Pryx sandbox:{home}:{}\n", shell.display()),
    )?;
    fs::write(launch.directory.join("group"), format!("pryx:x:{gid}:\n"))?;
    let mut command = Command::new(bundle.join("helpers/bwrap"));
    command
        .env_clear()
        .env("LD_LIBRARY_PATH", bundle.join("libraries/lib"))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
        ])
        .args(["--uid", &uid.to_string(), "--gid", &gid.to_string()]);
    for path in ["/usr", "/bin", "/lib", "/lib64", "/etc"] {
        if Path::new(path).exists() {
            command.args(["--ro-bind", path, path]);
        }
    }
    command.args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--tmpfs", "/dev/shm"]);
    for path in [bundle, executable] {
        command.arg("--ro-bind").arg(path).arg(path);
    }
    for path in [&policy.data, &tools.scratch] {
        command.arg("--bind").arg(path).arg(path);
    }
    for name in ["passwd", "group"] {
        command.arg("--ro-bind").arg(launch.directory.join(name)).arg(format!("/etc/{name}"));
    }
    command
        .arg("--ro-bind")
        .arg(&launch.directory)
        .arg(&launch.home_mount)
        .arg("--chdir")
        .arg(&policy.data)
        .arg(executable)
        .arg("process-ssh-child")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(fs::File::create(launch.directory.join("server.stderr"))?);
    let encoded = serde_json::to_vec(&Input { policy: policy.clone(), launch, bundle: bundle.into() })?;
    let mut child = command.spawn()?;
    if let Err(error) = child.stdin.take().ok_or_else(|| io::Error::other("Missing SSH bootstrap"))?.write_all(&encoded)
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok(child)
}

pub fn child() -> Result<(), Box<dyn std::error::Error>> {
    const MAX_BOOTSTRAP_BYTES: u64 = 32 * 1024;
    let mut bytes = Vec::new();
    std::io::stdin().take(MAX_BOOTSTRAP_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BOOTSTRAP_BYTES {
        return Err("Oversized SSH bootstrap".into());
    }
    let input: Input = serde_json::from_slice(&bytes)?;
    fs::create_dir(PRIVATE_DIRECTORY)?;
    fs::set_permissions(PRIVATE_DIRECTORY, fs::Permissions::from_mode(0o700))?;
    let private = UnixListener::bind(PRIVATE_SOCKET)?;
    fs::set_permissions(PRIVATE_SOCKET, fs::Permissions::from_mode(0o600))?;
    let scoped = UnixListener::bind_addr(&SocketAddr::from_abstract_name(&input.launch.address)?)?;
    private.set_nonblocking(true)?;
    scoped.set_nonblocking(true)?;
    crate::process_policy::inherited(&input.policy)?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let result: io::Result<()> = runtime.block_on(async {
        let private = tokio::net::UnixListener::from_std(private)?;
        let scoped = tokio::net::UnixListener::from_std(scoped)?;
        fs::write(input.launch.directory.join("ready"), b"private Unix OpenSSH worker\n")?;
        loop {
            tokio::select! {
                accepted=scoped.accept()=>{
                    let (mut downstream,_)=accepted?;
                    tokio::spawn(async move {
                        if let Ok(mut upstream)=tokio::net::UnixStream::connect(PRIVATE_SOCKET).await {
                            let _=tokio::io::copy_bidirectional(&mut downstream,&mut upstream).await;
                        }
                    });
                }
                accepted=private.accept()=>{
                    let (stream,_)=accepted?;
                    let stream=stream.into_std()?;stream.set_nonblocking(false)?;
                    let stdin:std::os::fd::OwnedFd=stream.try_clone()?.into();
                    let stdout:std::os::fd::OwnedFd=stream.into();
                    let child=tokio::process::Command::new(input.bundle.join("desktop/bin/sshd"))
                        .args(["-i","-e","-f"]).arg(input.launch.home_mount.join("config"))
                        .env_clear().env("PATH","/usr/bin:/bin")
                        .stdin(Stdio::from(stdin)).stdout(Stdio::from(stdout))
                        .stderr(fs::OpenOptions::new().append(true).open(input.launch.directory.join("server.stderr"))?)
                        .kill_on_drop(true).spawn()?;
                    tokio::spawn(async move { let mut child=child;let _=child.wait().await; });
                }
            }
        }
    });
    result?;
    Ok(())
}
