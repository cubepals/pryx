// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The agent executable's bootstrap. Embedded-shell invocations run only in the sandbox's
//! existing isolation and never receive the bootstrap secret. The node remains lifecycle owner.

use brush_builtins::{BuiltinSet, ShellBuilderExt};
use clap::Parser;
use std::io::{self, BufRead, Write};
use std::os::unix::process::CommandExt;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().is_some_and(|a| a == pryx_agent::bootstrap_capabilities::PROBE) {
        println!("{}", serde_json::to_string(&pryx_agent::bootstrap_capabilities::probe())?);
        return Ok(());
    }
    let native_bootstrap = arguments.first().is_some_and(|a| a.starts_with("--") && a != "--version");
    let mappings = if native_bootstrap { pryx_agent::fuse_namespace::prepare()? } else { None };
    // Images run as the same sandbox user. Prevent their processes from reading the agent's
    // bootstrap environment or memory through ptrace/procfs, without adding a privilege.
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    rustix::process::set_child_subreaper(rustix::process::Pid::from_raw(1))?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    if matches!(arguments.first().map(String::as_str), Some("tool-child" | "mount-child")) {
        let mount = arguments[0] == "mount-child";
        pryx_agent::fuse_namespace::child(mount)?;
        let mut command = arguments[1..].to_vec();
        let _directory = if mount { Some(pryx_agent::fuse_namespace::worker(&mut command)?) } else { None };
        if mount {
            let name = command.first().ok_or("Missing mount helper")?;
            let path = pryx_agent::fuse_namespace::helper(name).ok_or("Mount helper is not installed")?;
            if pryx_agent::fuse_namespace::readable(&path) {
                eprintln!(
                    "mount helper {} is readable; it must be execute-only so its credentials stay out of /proc",
                    path.display()
                );
                std::process::exit(pryx_agent::fuse_namespace::READABLE_HELPER);
            }
            command[0] = path.to_string_lossy().into_owned();
        }
        let executable = command.first().ok_or("Missing tool command")?;
        return Err(std::process::Command::new(executable).args(&command[1..]).exec().into());
    }
    if arguments.first().is_some_and(|a| a == "process-probe") {
        println!("{}", serde_json::to_string(&pryx_agent::process_capabilities::probe())?);
        return Ok(());
    }
    if arguments.first().is_some_and(|a| a == "process-child") {
        let supervisor = arguments.get(1).ok_or("Missing supervisor")?.parse()?;
        let network = arguments.get(2).ok_or("Missing network policy")?.parse()?;
        pryx_agent::process_policy::child(supervisor, network)?;
        let executable = arguments.get(3).ok_or("Missing restricted command")?;
        return Err(std::process::Command::new(executable).args(&arguments[4..]).exec().into());
    }
    if arguments.first().is_some_and(|a| a == "network-off") {
        pryx_agent::process_policy::network_off()?;
        let executable = arguments.get(1).ok_or("Missing restricted command")?;
        return Err(std::process::Command::new(executable).args(&arguments[2..]).exec().into());
    }
    // This trusted metadata reader must exist before the main thread installs Landlock.
    // Only the supervisor bootstrap uses it; guest shell/launcher invocations cannot create
    // a less restricted worker from their inherited policy.
    let bootstrap = !matches!(arguments.first().map(String::as_str), Some("shell" | "--version" | "restricted"));
    let forwarding = if bootstrap
        && (std::env::var_os(pryx_agent::process_policy::POLICY_ENV).is_some()
            || arguments.iter().any(|a| a == "--host-bindings" || a.starts_with("--host-bindings=")))
    {
        Some(pryx_agent::process_forward::ProcessForward::start()?)
    } else {
        None
    };
    if let Ok(encoded) = std::env::var(pryx_agent::process_policy::POLICY_ENV) {
        let policy = serde_json::from_str(&encoded)?;
        pryx_agent::process_policy::enter(&policy)?;
        if arguments.first().is_some_and(|a| a == "restricted") {
            let executable = arguments.get(1).ok_or("Missing restricted command")?;
            return Err(std::process::Command::new(executable)
                .args(&arguments[2..])
                .env_remove(pryx_agent::process_policy::POLICY_ENV)
                .exec()
                .into());
        }
    }
    if arguments.first().is_some_and(|a| a == "--version") {
        println!("pryx-agent {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if !matches!(arguments.first().map(String::as_str), Some("shell" | "--version")) {
        pryx_agent::fuse_namespace::enter(mappings)?;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    if arguments.first().is_some_and(|a| a == "shell") {
        let code = runtime.block_on(shell(arguments.get(1).cloned()))?;
        std::process::exit(code.into());
    }
    let code = runtime.block_on(pryx_agent::bootstrap::serve(pryx_agent::bootstrap::Bootstrap::parse(), forwarding))?;
    std::process::exit(code)
}

async fn shell(command: Option<String>) -> Result<u8, Box<dyn std::error::Error>> {
    let mut shell = brush_core::Shell::builder().default_builtins(BuiltinSet::BashMode).build().await?;
    if let Some(command) = command {
        return Ok(shell.run_dash_c_command(command).await?.exit_code.into());
    }
    let input = io::stdin();
    let mut input = input.lock();
    loop {
        print!("$ ");
        io::stdout().flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(shell.last_exit_status());
        }
        let parameters = shell.default_exec_params();
        let result = shell.run_string(line, &brush_core::SourceInfo::from("terminal"), &parameters).await?;
        if !result.is_normal_flow() {
            return Ok(result.exit_code.into());
        }
    }
}
