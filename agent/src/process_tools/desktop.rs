// SPDX-License-Identifier: FSL-1.1-ALv2
//! Closed desktop worker in maintained private namespaces. Only the trusted bootstrap can
//! launch it before ordinary guest restrictions; all X11/VNC endpoints stay private.
use crate::process_policy::Policy;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

#[derive(Clone, Deserialize, Serialize)]
pub struct Launch {
    pub scratch: PathBuf,
    pub address: String,
    pub private_token: String,
    pub application_url: Option<String>,
}
#[derive(Deserialize, Serialize)]
struct Input {
    policy: Policy,
    launch: Launch,
    bundle: PathBuf,
}

pub fn start_at(policy: &Policy, launch: Launch, bundle: &Path, executable: &Path) -> io::Result<Child> {
    let tools = policy.tools.as_ref().ok_or_else(|| io::Error::other("Missing native desktop policy"))?;
    if !tools.browser_on {
        return Err(io::Error::other("Process desktop requires usable unprivileged user namespaces"));
    }
    if !fs::canonicalize(&launch.scratch)?.starts_with(fs::canonicalize(&tools.scratch)?)
        || !fs::canonicalize(bundle)?.starts_with(fs::canonicalize(&tools.cache)?)
    {
        return Err(io::Error::other("Desktop paths escape host-owned native tool roots"));
    }
    for name in ["home", "cache"] {
        fs::create_dir_all(launch.scratch.join(name))?;
    }
    let mut command = Command::new(bundle.join("helpers/bwrap"));
    command
        .env_clear()
        .env("LD_LIBRARY_PATH", bundle.join("libraries/lib"))
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", launch.scratch.join("home"))
        .env("XDG_CACHE_HOME", launch.scratch.join("cache"))
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
        .args([
            "--uid",
            &rustix::process::geteuid().as_raw().to_string(),
            "--gid",
            &rustix::process::getegid().as_raw().to_string(),
        ]);
    for path in ["/usr", "/bin", "/lib", "/lib64", "/etc", "/sys/devices/system/cpu"] {
        if Path::new(path).exists() {
            command.args(["--ro-bind", path, path]);
        }
    }
    command.args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--tmpfs", "/dev/shm", "--tmpfs", "/usr/bin"]);
    command.args(["--ro-bind", "/usr/bin/dash", "/usr/bin/sh"]);
    command.arg("--ro-bind").arg(bundle.join("desktop/payload/usr/bin/xkbcomp")).arg("/usr/bin/xkbcomp");
    for path in [bundle, executable] {
        command.arg("--ro-bind").arg(path).arg(path);
    }
    for path in [&policy.data, &tools.scratch] {
        command.arg("--bind").arg(path).arg(path);
    }
    command
        .arg("--chdir")
        .arg(&policy.data)
        .arg(executable)
        .arg("process-desktop-child")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(fs::File::create(launch.scratch.join("desktop.stderr"))?);
    let encoded = serde_json::to_vec(&Input { policy: policy.clone(), launch, bundle: bundle.into() })?;
    let mut child = command.spawn()?;
    if let Err(error) =
        child.stdin.take().ok_or_else(|| io::Error::other("Missing desktop bootstrap"))?.write_all(&encoded)
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
        return Err("Oversized desktop bootstrap".into());
    }
    let input: Input = serde_json::from_slice(&bytes)?;
    fs::create_dir("/tmp/.X11-unix")?;
    crate::process_policy::desktop_browser(&input.policy)?;
    let config = serde_json::json!({"bundle":input.bundle,"scratch":input.launch.scratch,"address":input.launch.address,"privateToken":input.launch.private_token,"applicationUrl":input.launch.application_url,"timeoutMs":crate::contract::TOOL_STARTUP.as_millis(),"maxRequestBytes":crate::contract::MAX_REQUEST_BYTES});
    let mut broker = Command::new(input.bundle.join("bin/node"))
        .arg(input.bundle.join("desktop/adapter/start.mjs"))
        .stdin(Stdio::piped())
        .spawn()?;
    broker.stdin.take().ok_or("Missing desktop broker input")?.write_all(&serde_json::to_vec(&config)?)?;
    std::process::exit(broker.wait()?.code().unwrap_or(1))
}
