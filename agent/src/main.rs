// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The agent executable's bootstrap. Embedded-shell invocations run only in the sandbox's
//! existing isolation and never receive the bootstrap secret. The node remains lifecycle owner.

use brush_builtins::{BuiltinSet, ShellBuilderExt};
use clap::Parser;
use std::io::{self, BufRead, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Images run as the same sandbox user. Prevent their processes from reading the agent's
    // bootstrap environment or memory through ptrace/procfs, without adding a privilege.
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().is_some_and(|a| a == "--version") {
        println!("pryx-agent {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    if arguments.first().is_some_and(|a| a == "shell") {
        let code = runtime.block_on(shell(arguments.get(1).cloned()))?;
        std::process::exit(code.into());
    }
    let code = runtime.block_on(pryx_agent::bootstrap::serve(pryx_agent::bootstrap::Bootstrap::parse()))?;
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
