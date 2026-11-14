// SPDX-License-Identifier: FSL-1.1-ALv2
//! Trusted guest network setup before any image code runs. Host-owned bootstrap programs
//! configure devices and DHCP; this module owns checked resolver mounts and their readiness,
//! never packet framing, a network stack or image-selected root executables.
use crate::{
    paths::DataDir,
    vm_boot::{self, Network},
};
use rustix::mount;
use std::{fs::File, io, os::fd::AsRawFd, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
};

const DIRECTORY: &str = "run/pryx-network";
const RESOLVER: &str = "run/pryx-network/resolv.conf";

pub(crate) fn prepare(root: &DataDir, network: Option<&Network>) -> io::Result<()> {
    let Some(network) = network else { return Ok(()) };
    if network.interface.is_empty()
        || network.interface.len() >= libc::IFNAMSIZ
        || !network.interface.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        || network.mtu == 0
        || network.vsock_port.is_some_and(|p| p == 0 || p == vm_boot::PORT)
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid host-owned guest network configuration"));
    }
    if let Some(policy) = &network.policy {
        let services = [policy.http.source, policy.tls.source, policy.dns.source];
        let targets = [policy.http.target, policy.tls.target, policy.dns.target];
        if !policy.host.is_private()
            || services.contains(&0)
            || targets.contains(&0)
            || policy.dhcp_client == 0
            || policy.dhcp_server == 0
            || policy.dhcp_client == policy.dhcp_server
            || services.contains(&policy.dhcp_server)
            || services.iter().enumerate().any(|(index, port)| services[index + 1..].contains(port))
            || targets.iter().enumerate().any(|(index, port)| targets[index + 1..].contains(port))
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid host-owned guest network policy"));
        }
    }
    root.mkdir(DIRECTORY)?;
    root.chmod(DIRECTORY, 0o700)?;
    let source = root.write(RESOLVER)?;
    let target = root.write("etc/resolv.conf")?;
    let descriptor = |file: &File| format!("/proc/self/fd/{}", file.as_raw_fd());
    mount::mount_bind(descriptor(&source), descriptor(&target))?;
    Ok(())
}

pub(crate) async fn start(network: Option<&Network>) -> io::Result<Option<Child>> {
    let Some(network) = network else { return Ok(None) };
    let mut command = Command::new(Path::new(vm_boot::BOOTSTRAP_ROOT).join(vm_boot::NETWORK_PROGRAM));
    command.args([
        "--iface",
        &network.interface,
        "--mac",
        &network.mac,
        "--mtu",
        &network.mtu.to_string(),
        "--resolver",
        &format!("/{RESOLVER}"),
    ]);
    if let Some(port) = network.vsock_port {
        command
            .arg("--forwarder")
            .arg(Path::new(vm_boot::BOOTSTRAP_ROOT).join(vm_boot::PACKET_PROGRAM))
            .arg("--vsock-port")
            .arg(port.to_string());
    }
    let mut child = command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;
    let mut output = child.stdout.take().expect("private readiness pipe configured");
    let mut byte = [0u8; 1];
    let count = tokio::time::timeout(Duration::from_secs(30), output.read(&mut byte)).await??;
    if count != 0 || child.try_wait()?.is_some() {
        return Err(io::Error::other("Trusted guest network setup exited before readiness"));
    }
    Ok(Some(child))
}

pub(crate) async fn failure(child: &mut Option<Child>) -> io::Error {
    match child {
        Some(child) => {
            let _ = child.wait().await;
            io::Error::other("Trusted guest network helper exited")
        }
        None => std::future::pending().await,
    }
}

pub(crate) async fn stop(child: Option<Child>) -> io::Result<()> {
    let Some(mut child) = child else { return Ok(()) };
    if let Some(pid) = child.id().and_then(|pid| rustix::process::Pid::from_raw(pid as i32)) {
        rustix::process::kill_process(pid, rustix::process::Signal::TERM)?;
    }
    match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
        Ok(result) => {
            result?;
        }
        Err(_) => {
            child.kill().await?;
        }
    }
    Ok(())
}
