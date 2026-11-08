// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! PID 1 starts the original entrypoint, serves the agent and forwards termination. Adopted
//! descendants are reaped separately from the process API's managed children.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::{io, path::PathBuf};

use axum::serve::ListenerExt;
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::Parser;
use tokio::io::AsyncWriteExt;
use tokio::signal::unix::{SignalKind, signal};

use crate::auth::SigningKey;
use crate::paths::DataDir;
use crate::ports::Ports;
use crate::process::{Command, Event, Processes, Start};
use crate::server::{Agent, router};

#[derive(Parser)]
#[command(name = "pryx-agent", version)]
pub struct Bootstrap {
    #[arg(long)]
    sandbox: String,
    #[arg(long)]
    epoch: u64,
    #[arg(long)]
    data: PathBuf,
    #[arg(long)]
    domain: String,
    #[arg(long)]
    public_origin: Option<String>,
    #[arg(long)]
    port: u16,
    #[arg(long, value_delimiter = ',')]
    spare_ports: Vec<u16>,
    #[arg(long)]
    secret_env: String,
    #[arg(long)]
    host_bindings: Option<String>,
    #[arg(long)]
    keep_alive: bool,
    #[arg(long, default_value = "/")]
    entrypoint_cwd: String,
    #[arg(last = true)]
    entrypoint: Vec<String>,
}

impl Bootstrap {
    fn bindings(&self, process_tier: bool) -> io::Result<BTreeMap<u16, Vec<IpAddr>>> {
        let expected = std::iter::once(self.port).chain(self.spare_ports.iter().copied()).collect::<BTreeSet<_>>();
        if expected.contains(&0) || expected.len() != self.spare_ports.len() + 1 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let Some(encoded) = &self.host_bindings else {
            if process_tier {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "Process host bindings are required"));
            }
            // Container agents listen inside their private network namespace.
            return Ok(expected.into_iter().map(|port| (port, vec![Ipv4Addr::UNSPECIFIED.into()])).collect());
        };
        let bindings: BTreeMap<u16, Vec<IpAddr>> = serde_json::from_str(encoded).map_err(io::Error::other)?;
        if bindings.len() != expected.len()
            || expected.iter().any(|port| {
                bindings.get(port).is_none_or(|ips| ips.is_empty() || ips.iter().any(IpAddr::is_unspecified))
            })
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid process host bindings"));
        }
        Ok(bindings)
    }
}

pub async fn serve(
    args: Bootstrap,
    forward: Option<Arc<dyn crate::guest::Forward>>,
) -> Result<i32, Box<dyn std::error::Error>> {
    if args.host_bindings.is_some() && forward.is_none() {
        return Err("Process forwarding ownership channel is required".into());
    }
    let bindings = args.bindings(std::env::var_os(crate::process_policy::POLICY_ENV).is_some())?;
    let encoded = std::env::var(&args.secret_env).map_err(|_| "Agent signing material is missing")?;
    let key = SigningKey::from_encoded(&encoded)?;
    let logbook = Arc::new(crate::logbook::Logbook::open(&args.data)?);
    let telemetry = Arc::new(crate::telemetry::Telemetry::default());
    let mut listeners = Vec::new();
    for ip in &bindings[&args.port] {
        listeners.push(tokio::net::TcpListener::bind((*ip, args.port)).await?);
    }
    let mut spares = Vec::new();
    for port in args.spare_ports {
        let mut held = Vec::new();
        for ip in &bindings[&port] {
            held.push(tokio::net::TcpListener::bind((*ip, port)).await?);
        }
        spares.push((port, held));
    }
    let ports = Ports::bind_listeners(
        args.sandbox.clone(),
        args.domain,
        args.port,
        spares,
        Some(DataDir::open(&args.data)?),
        telemetry.clone(),
        forward,
    )
    .await?;
    ports.set_public_origin(args.public_origin);
    let agent = Arc::new(Agent {
        file_tasks: Default::default(),
        shells: Default::default(),
        public_key: key.public_key(),
        key,
        sandbox: args.sandbox.clone(),
        epoch: args.epoch,
        data: DataDir::open(&args.data)?,
        processes: Processes::new(args.data, args.secret_env),
        ports,
        telemetry,
        logs: logbook.clone(),
    });
    crate::tools::prepare(&agent).map_err(|_| "Native tool runtime directory failed")?;
    let mut servers = Vec::new();
    for listener in listeners {
        let listener = listener.tap_io(|socket| {
            let _ = socket.set_nodelay(true);
        });
        servers.push(tokio::spawn(axum::serve(listener, router(agent.clone())).into_future()));
    }
    let mut term = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mount_gate = tokio::select! {
        result=crate::tools::wait_mounts(&agent)=>result.map_err(|_|"Initial sandbox mounts failed"),
        _=term.recv()=>Err("Terminated while preparing mounts"),
        _=interrupt.recv()=>Err("Interrupted while preparing mounts"),
    };
    if let Err(error) = mount_gate {
        terminate(&agent, libc::SIGKILL).await;
        for server in servers {
            server.abort()
        }
        return Err(error.into());
    }
    let entrypoint = if args.entrypoint.is_empty() {
        None
    } else {
        Some(agent.processes.start(
            Start {
                command: Command::Argv(args.entrypoint),
                env: Default::default(),
                cwd: Some(args.entrypoint_cwd),
                stdin: false,
                pty: false,
                cols: None,
                rows: None,
                timeout: None,
            },
            "entrypoint",
            None,
        )?)
    };
    let logs = if let Some(process) = &entrypoint {
        let (replay, mut output) = process.subscribe().await;
        Some(tokio::spawn(async move {
            for event in replay {
                log_output(event, &logbook).await;
            }
            while let Some(event) = output.recv().await {
                log_output(event, &logbook).await;
            }
        }))
    } else {
        None
    };
    let reaper = agent.clone();
    let reaper = tokio::spawn(async move {
        let mut checkpoint = tokio::time::Instant::now();
        loop {
            reaper.processes.reap_orphans();
            if checkpoint.elapsed() >= std::time::Duration::from_secs(1) {
                let _ =
                    crate::state::write(&reaper.data, crate::state::ACTIVITY_PATH, &reaper.telemetry.snapshot(), false);
                checkpoint = tokio::time::Instant::now();
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });
    let exited = async {
        match &entrypoint {
            Some(process) if !args.keep_alive => process.wait().await.code.unwrap_or(1),
            _ => std::future::pending::<i32>().await,
        }
    };
    // Serve scoped health while scientific imports warm. Kernel open/reuse waits on its own
    // readiness, so a CPU-limited import cannot consume the control token's health deadline.
    let warming = async {
        crate::kernels::prestart(&agent).await.map_err(|_| "Prestarted language kernel failed")?;
        std::future::pending::<Result<i32, Box<dyn std::error::Error>>>().await
    };
    let result = tokio::select! {
        code = exited => Ok(code),
        result = warming => result,
        _ = term.recv() => { terminate(&agent, libc::SIGTERM).await; Ok(0) },
        _ = interrupt.recv() => { terminate(&agent, libc::SIGINT).await; Ok(0) },
    };
    if result.is_err() {
        terminate(&agent, libc::SIGKILL).await;
    }
    for server in servers {
        server.abort();
    }
    reaper.abort();
    if let Some(logs) = logs {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), logs).await;
    }
    result
}

async fn log_output(event: Event, logbook: &crate::logbook::Logbook) {
    if let Event::Output { stream, bytes } = event
        && let Ok(decoded) = STANDARD.decode(&bytes)
    {
        if logbook.record(stream, &bytes).is_err() {
            eprintln!("Entrypoint log persistence failed");
        }
        if stream == "stdout" {
            let _ = tokio::io::stdout().write_all(&decoded).await;
        } else {
            let _ = tokio::io::stderr().write_all(&decoded).await;
        }
    }
}

async fn terminate(agent: &Agent, signal: i32) {
    for process in agent.processes.list() {
        if let Ok(process) = agent.processes.get(&process.id) {
            let _ = process.signal(signal);
        }
    }
    let wait = async {
        for process in agent.processes.list() {
            if let Ok(process) = agent.processes.get(&process.id) {
                process.wait().await;
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), wait).await;
    for process in agent.processes.list() {
        if let Ok(process) = agent.processes.get(&process.id) {
            let _ = process.signal(libc::SIGKILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::AGENT_PORT;

    #[test]
    fn process_bootstrap_requires_complete_explicit_bindings() {
        let agent_port = AGENT_PORT.to_string();
        let spare_port = (AGENT_PORT + 1).to_string();
        let base = [
            "pryx-agent",
            "--sandbox",
            "one",
            "--epoch",
            "1",
            "--data",
            "/data",
            "--domain",
            "pryx.test",
            "--port",
            &agent_port,
            "--spare-ports",
            &spare_port,
            "--secret-env",
            "TEST_SECRET",
        ];
        let args = Bootstrap::try_parse_from(base).unwrap();
        assert!(args.bindings(true).is_err());
        assert!(args.bindings(false).unwrap()[&AGENT_PORT][0].is_unspecified());
        for ips in
            [Vec::<IpAddr>::new(), vec![Ipv4Addr::UNSPECIFIED.into()], vec![std::net::Ipv6Addr::UNSPECIFIED.into()]]
        {
            let json = serde_json::to_string(&BTreeMap::from([
                (AGENT_PORT, ips),
                (AGENT_PORT + 1, vec![Ipv4Addr::LOCALHOST.into()]),
            ]))
            .unwrap();
            let args = Bootstrap::try_parse_from(base.into_iter().chain(["--host-bindings", &json])).unwrap();
            assert!(args.bindings(true).is_err());
        }
        for json in ["{}".to_string(), serde_json::json!({agent_port.clone(): ["127.0.0.1"]}).to_string()] {
            let args = Bootstrap::try_parse_from(base.into_iter().chain(["--host-bindings", &json])).unwrap();
            assert!(args.bindings(true).is_err());
        }
        let expected = BTreeMap::from([
            (AGENT_PORT, vec!["127.0.0.1".parse::<IpAddr>().unwrap(), "::1".parse().unwrap()]),
            (AGENT_PORT + 1, vec!["127.0.0.2".parse::<IpAddr>().unwrap()]),
        ]);
        let json = serde_json::to_string(&expected).unwrap();
        let args = Bootstrap::try_parse_from(base.into_iter().chain(["--host-bindings", &json])).unwrap();
        assert_eq!(args.bindings(true).unwrap(), expected);
    }
}
