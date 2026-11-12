// SPDX-License-Identifier: FSL-1.1-ALv2
//! A closed trusted worker starts only the verified browser broker through maintained
//! bubblewrap. A private proc permits Chromium's sandbox without exposing host proc writes.
use crate::{process_policy::Policy as ProcessPolicy, process_tool_policy::Policy};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
};

#[derive(Clone, Deserialize, Serialize)]
pub struct Launch {
    pub scratch: PathBuf,
    pub address: String,
    pub private_token: String,
}
#[derive(Deserialize, Serialize)]
struct Input {
    policy: ProcessPolicy,
    launch: Launch,
    bundle: PathBuf,
}

pub(super) fn start(policy: &ProcessPolicy, tools: &Policy, launch: Launch) -> io::Result<Child> {
    if !tools.browser_on {
        return Err(io::Error::other(
            "Process browser requires usable unprivileged user namespaces; allow them in the host security policy",
        ));
    }
    let artifact = tools.artifact.as_ref().ok_or_else(|| io::Error::other("Missing pinned process tools"))?;
    let bundle = tools.cache.join(&artifact.sha256);
    if !fs::canonicalize(&launch.scratch)?.starts_with(fs::canonicalize(&tools.scratch)?) {
        return Err(io::Error::other("Browser scratch escapes its private root"));
    }
    for name in ["home", "tmp", "cache"] {
        fs::create_dir_all(launch.scratch.join(name))?;
    }
    let executable = std::env::current_exe()?;
    let mut command = Command::new(bundle.join("helpers/bwrap"));
    command
        .env_clear()
        .env("LD_LIBRARY_PATH", bundle.join("libraries/lib"))
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", launch.scratch.join("home"))
        .env("TMPDIR", launch.scratch.join("tmp"))
        .env("XDG_CACHE_HOME", launch.scratch.join("cache"))
        .env("FONTCONFIG_FILE", bundle.join("etc/fonts/fonts.conf"));
    command
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
        if std::path::Path::new(path).exists() {
            command.args(["--ro-bind", path, path]);
        }
    }
    // Install private mounts first so installations and scratch under /tmp stay visible.
    command.args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--tmpfs", "/dev/shm"]);
    for path in [&bundle, &executable] {
        command.arg("--ro-bind").arg(path).arg(path);
    }
    for path in [&policy.data, &tools.scratch] {
        command.arg("--bind").arg(path).arg(path);
    }
    command
        .arg("--chdir")
        .arg(&policy.data)
        .arg(executable)
        .arg("process-browser-child")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(fs::File::create(launch.scratch.join("browser.stderr"))?);
    let input = serde_json::to_vec(&Input { policy: policy.clone(), launch, bundle })?;
    let mut child = command.spawn()?;
    let result =
        child.stdin.take().ok_or_else(|| io::Error::other("Missing browser bootstrap input"))?.write_all(&input);
    if let Err(problem) = result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(problem);
    }
    Ok(child)
}

pub fn child() -> Result<(), Box<dyn std::error::Error>> {
    const MAX_BOOTSTRAP_BYTES: u64 = 32 * 1024;
    let mut bytes = Vec::new();
    std::io::stdin().take(MAX_BOOTSTRAP_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BOOTSTRAP_BYTES {
        return Err("Oversized browser bootstrap".into());
    }
    let input: Input = serde_json::from_slice(&bytes)?;
    crate::process_policy::browser(&input.policy)?;
    let config = serde_json::json!({"bundle":input.bundle,"scratch":input.launch.scratch,"address":input.launch.address,"privateToken":input.launch.private_token,"timeoutMs":crate::contract::TOOL_STARTUP.as_millis(),"maxRequestBytes":crate::contract::MAX_REQUEST_BYTES});
    let mut broker = Command::new(input.bundle.join("bin/node"))
        .arg(input.bundle.join("browser-client/start.mjs"))
        .stdin(Stdio::piped())
        .spawn()?;
    broker.stdin.take().ok_or("Missing browser broker input")?.write_all(&serde_json::to_vec(&config)?)?;
    std::process::exit(broker.wait()?.code().unwrap_or(1))
}
