// SPDX-License-Identifier: FSL-1.1-ALv2
//! A closed native OpenSSH worker serves only accepted private Unix descriptors. Bubblewrap
//! supplies private accounts/proc/tmp; the ordinary guest policy precedes every server/shell.
use crate::process_policy::Policy;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Seek, Write},
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

const MAX_BOOTSTRAP_BYTES: usize = 32 * 1024;

pub const PRIVATE_DIRECTORY: &str = "/tmp/pryx-ssh";
pub const PRIVATE_SOCKET: &str = "/tmp/pryx-ssh/server.sock";
#[derive(Clone, Deserialize, Serialize)]
pub struct Launch {
    pub directory: PathBuf,
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
    // These descriptors are created outside every guest filesystem grant. Bubblewrap copies
    // their bytes before mounting private accounts, without reopening guest-writable paths.
    let mut passwd = tempfile::tempfile_in(&tools.downloads)?;
    passwd.write_all(format!("pryx:x:{uid}:{gid}:Pryx sandbox:{home}:{}\n", shell.display()).as_bytes())?;
    passwd.rewind()?;
    let mut group = tempfile::tempfile_in(&tools.downloads)?;
    group.write_all(format!("pryx:x:{gid}:\n").as_bytes())?;
    group.rewind()?;
    let encoded =
        serde_json::to_string(&Input { policy: policy.clone(), launch: launch.clone(), bundle: bundle.into() })?;
    if encoded.len() > MAX_BOOTSTRAP_BYTES {
        return Err(io::Error::other("Oversized SSH bootstrap"));
    }
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
    command.args(["--ro-bind-data", "0", "/etc/passwd", "--ro-bind-data", "1", "/etc/group"]);
    command
        .arg("--ro-bind")
        .arg(&tools.scratch)
        .arg(crate::process_tool_policy::SSH_AUTH_DIRECTORY)
        .arg("--chdir")
        .arg(&policy.data)
        .arg(executable)
        .arg("process-ssh-child")
        .env(crate::process_tool_policy::SSH_BOOTSTRAP_ENV, encoded)
        .stdin(Stdio::from(passwd))
        .stdout(Stdio::from(group))
        .stderr(tools.scratch_output(&launch.directory.join("server.stderr"))?);
    let child = command.spawn()?;
    Ok(child)
}

pub fn child() -> Result<(), Box<dyn std::error::Error>> {
    let encoded = std::env::var(crate::process_tool_policy::SSH_BOOTSTRAP_ENV)?;
    if encoded.len() > MAX_BOOTSTRAP_BYTES {
        return Err("Oversized SSH bootstrap".into());
    }
    let input: Input = serde_json::from_str(&encoded)?;
    let tools = input.policy.tools.as_ref().ok_or("Missing native SSH policy")?;
    let relative = input.launch.directory.strip_prefix(&tools.scratch)?;
    if relative.components().any(|component| !matches!(component, std::path::Component::Normal(_))) {
        return Err("Native SSH directory escapes its private root".into());
    }
    let config = Path::new(crate::process_tool_policy::SSH_AUTH_DIRECTORY).join(relative).join("config");
    fs::create_dir_all(PRIVATE_DIRECTORY)?;
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
                        .args(["-i","-e","-f"]).arg(&config)
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
