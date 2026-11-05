// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! PID 1 starts the original entrypoint, serves the agent and forwards termination. Adopted
//! descendants are reaped separately from the process API's managed children.

use std::path::PathBuf;
use std::sync::Arc;

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
    port: u16,
    #[arg(long, value_delimiter = ',')]
    spare_ports: Vec<u16>,
    #[arg(long)]
    secret_env: String,
    #[arg(long)]
    keep_alive: bool,
    #[arg(long, default_value = "/")]
    entrypoint_cwd: String,
    #[arg(last = true)]
    entrypoint: Vec<String>,
}

pub async fn serve(args: Bootstrap) -> Result<i32, Box<dyn std::error::Error>> {
    let encoded = std::env::var(&args.secret_env).map_err(|_| "Agent signing material is missing")?;
    let key = SigningKey::from_encoded(&encoded)?;
    let logbook = Arc::new(crate::logbook::Logbook::open(&args.data)?);
    let telemetry = Arc::new(crate::telemetry::Telemetry::default());
    let ports = Ports::bind_persistent(
        args.sandbox.clone(),
        args.domain,
        args.port,
        args.spare_ports,
        Some(DataDir::open(&args.data)?),
        telemetry.clone(),
    )
    .await?;
    let agent = Arc::new(Agent {
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
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, args.port)).await?;
    let listener = listener.tap_io(|socket| {
        let _ = socket.set_nodelay(true);
    });
    let server = tokio::spawn(axum::serve(listener, router(agent.clone())).into_future());
    let mut term = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
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
    let code = tokio::select! {
        code = exited => code,
        _ = term.recv() => { terminate(&agent, libc::SIGTERM).await; 0 },
        _ = interrupt.recv() => { terminate(&agent, libc::SIGINT).await; 0 },
    };
    server.abort();
    reaper.abort();
    if let Some(logs) = logs {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), logs).await;
    }
    Ok(code)
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
