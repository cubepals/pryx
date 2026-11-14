// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! PID 1 starts the original entrypoint, serves the agent and forwards termination. Adopted
//! descendants are reaped separately from the process API's managed children.
//!
//! Parts (`bootstrap/`):
//! - `files.rs`: trusted file-only VM service, without workload or tool startup.

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

mod files;

type Bindings = BTreeMap<u16, Vec<IpAddr>>;
type HttpTask = tokio::task::JoinHandle<io::Result<()>>;

fn spawn_reaper(reaper: Arc<Agent>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
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
    })
}

async fn bind_spares(ports: &[u16], bindings: &Bindings) -> io::Result<Vec<(u16, Vec<tokio::net::TcpListener>)>> {
    let mut spares = Vec::new();
    for &port in ports {
        let mut held = Vec::new();
        for ip in &bindings[&port] {
            held.push(tokio::net::TcpListener::bind((*ip, port)).await?);
        }
        spares.push((port, held));
    }
    Ok(spares)
}

async fn serve_http(
    agent: Arc<Agent>,
    port: u16,
    bindings: &Bindings,
    cid: Option<u32>,
    service_base: u32,
    extra_ports: &[u16],
    files_only: bool,
) -> io::Result<(Vec<HttpTask>, Option<crate::vm_ingress::Ingress>)> {
    let mut listeners = Vec::new();
    for ip in &bindings[&port] {
        listeners.push(tokio::net::TcpListener::bind((*ip, port)).await?);
    }
    let ingress = cid
        .map(|cid| {
            crate::vm_ingress::Ingress::bind(
                bindings.keys().copied().chain(extra_ports.iter().copied()),
                cid,
                service_base,
            )
        })
        .transpose()?;
    let mut servers = Vec::new();
    for listener in listeners {
        let listener = listener.tap_io(|socket| {
            let _ = socket.set_nodelay(true);
        });
        let app = if files_only { crate::server::files_router(agent.clone()) } else { router(agent.clone()) };
        servers.push(tokio::spawn(axum::serve(listener, app).into_future()));
    }
    Ok((servers, ingress))
}

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
    #[arg(long = crate::bootstrap_capabilities::BROWSER_ARGUMENT)]
    browser_sandbox: Option<bool>,
    #[arg(long = crate::bootstrap_capabilities::VSOCK_ARGUMENT)]
    vm_vsock: bool,
    #[arg(long = crate::bootstrap_capabilities::VM_FILES_ARGUMENT, hide = true)]
    vm_files_only: bool,
    #[arg(long)]
    vm_cid: Option<u32>,
    #[arg(long, default_value_t = 0)]
    vm_port_base: u32,
    #[arg(long, value_delimiter = ',')]
    vm_ports: Vec<u16>,
    #[arg(last = true)]
    entrypoint: Vec<String>,
}

impl Bootstrap {
    fn validate_host_policy(&self, forwarding: bool) -> Result<(), Box<dyn std::error::Error>> {
        if self.vm_files_only
            && (!self.vm_vsock
                || self.vm_cid.is_none()
                || !self.entrypoint.is_empty()
                || !self.spare_ports.is_empty()
                || !self.vm_ports.is_empty()
                || self.host_bindings.is_some())
        {
            return Err(
                "VM file service requires private guest ingress without workload commands or spare ports".into()
            );
        }
        if (!self.vm_ports.is_empty() || self.vm_port_base != 0) && !self.vm_vsock {
            return Err("VM port metadata requires its guest transport".into());
        }
        if (self.vm_port_base != 0 && self.vm_port_base != crate::vm_boot::SERVICE_PORT_BASE)
            || self.vm_port_base.checked_add(u32::from(u16::MAX)).is_none()
            || self.vm_ports.contains(&0)
        {
            return Err("VM service ports must be separate from private control ports".into());
        }
        if self.browser_sandbox == Some(false) && std::env::var_os(crate::process_policy::POLICY_ENV).is_some() {
            return Err("browser-sandbox cannot be disabled on the process tier".into());
        }
        if self.host_bindings.is_some() && !forwarding {
            return Err("Process forwarding ownership channel is required".into());
        }
        Ok(())
    }

    pub(crate) fn vm_files_only(&self) -> bool {
        self.vm_files_only
    }

    pub(crate) fn vm_identity(&self) -> (&str, u64, &str) {
        (&self.sandbox, self.epoch, &self.secret_env)
    }

    fn bindings(&self, process_tier: bool) -> io::Result<Bindings> {
        let expected = std::iter::once(self.port).chain(self.spare_ports.iter().copied()).collect::<BTreeSet<_>>();
        if expected.contains(&0) || expected.len() != self.spare_ports.len() + 1 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if self.vm_vsock {
            if process_tier || self.host_bindings.is_some() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "VM ingress requires guest-loopback bindings"));
            }
            return Ok(expected.into_iter().map(|port| (port, vec![Ipv4Addr::LOCALHOST.into()])).collect());
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
    args.validate_host_policy(forward.is_some())?;
    if args.vm_files_only {
        return args.serve_files().await;
    }
    let bindings = args.bindings(std::env::var_os(crate::process_policy::POLICY_ENV).is_some())?;
    let encoded = std::env::var(&args.secret_env).map_err(|_| "Agent signing material is missing")?;
    let key = SigningKey::from_encoded(&encoded)?;
    let logbook = Arc::new(crate::logbook::Logbook::open(&args.data)?);
    let telemetry = Arc::new(crate::telemetry::Telemetry::default());
    let spares = bind_spares(&args.spare_ports, &bindings).await?;
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
        vm_resources: args.vm_vsock,
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
    crate::tools::prepare(&agent, args.browser_sandbox.unwrap_or(true))
        .map_err(|_| "Native tool runtime directory failed")?;
    let cid = args.vm_vsock.then_some(args.vm_cid.unwrap_or(tokio_vsock::VMADDR_CID_HOST + 1));
    let (servers, mut vm_ingress) =
        serve_http(agent.clone(), args.port, &bindings, cid, args.vm_port_base, &args.vm_ports, false).await?;
    let mut term = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    prepare_mounts(&agent, &servers, &mut term, &mut interrupt).await?;
    let entrypoint = start_entrypoint(&agent, args.entrypoint, args.entrypoint_cwd)?;
    let logs = forward_entrypoint(&entrypoint, logbook).await;
    let reaper = spawn_reaper(agent.clone());
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
        error = ingress_failure(&mut vm_ingress) => Err(error.into()),
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

async fn prepare_mounts(
    agent: &Agent,
    servers: &[HttpTask],
    term: &mut tokio::signal::unix::Signal,
    interrupt: &mut tokio::signal::unix::Signal,
) -> Result<(), &'static str> {
    let mount_gate = tokio::select! {
        result=crate::tools::wait_mounts(agent)=>result.map_err(|_|"Initial sandbox mounts failed"),
        _=term.recv()=>Err("Terminated while preparing mounts"),
        _=interrupt.recv()=>Err("Interrupted while preparing mounts"),
    };
    if let Err(error) = mount_gate {
        terminate(agent, libc::SIGKILL).await;
        for server in servers {
            server.abort()
        }
        return Err(error);
    }
    Ok(())
}

async fn ingress_failure(ingress: &mut Option<crate::vm_ingress::Ingress>) -> io::Error {
    match ingress.as_mut() {
        Some(ingress) => ingress.failed().await,
        None => std::future::pending().await,
    }
}

fn start_entrypoint(
    agent: &Agent,
    arguments: Vec<String>,
    cwd: String,
) -> Result<Option<Arc<crate::process::Process>>, crate::process::ProcessError> {
    if arguments.is_empty() {
        return Ok(None);
    }
    agent
        .processes
        .start(
            Start {
                command: Command::Argv(arguments),
                env: Default::default(),
                cwd: Some(cwd),
                stdin: false,
                pty: false,
                cols: None,
                rows: None,
                timeout: None,
            },
            "entrypoint",
            None,
        )
        .map(Some)
}

async fn forward_entrypoint(
    entrypoint: &Option<Arc<crate::process::Process>>,
    logbook: Arc<crate::logbook::Logbook>,
) -> Option<tokio::task::JoinHandle<()>> {
    let process = entrypoint.as_ref()?;
    let (replay, mut output) = process.subscribe().await;
    Some(tokio::spawn(async move {
        for event in replay {
            log_output(event, &logbook).await;
        }
        while let Some(event) = output.recv().await {
            log_output(event, &logbook).await;
        }
    }))
}

async fn log_output(event: Event, logbook: &crate::logbook::Logbook) {
    if let Event::Output { stream, bytes } = event
        && let Ok(decoded) = STANDARD.decode(&bytes)
    {
        if logbook.record(stream, &bytes).is_err() {
            eprintln!("Entrypoint log persistence failed");
        }
        // Tokio's standard streams hand bytes to a blocking writer; flushing makes them reach the
        // descriptor before an entrypoint's exit ends the process.
        if stream == "stdout" {
            let mut out = tokio::io::stdout();
            let _ = out.write_all(&decoded).await;
            let _ = out.flush().await;
        } else {
            let mut err = tokio::io::stderr();
            let _ = err.write_all(&decoded).await;
            let _ = err.flush().await;
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
        let vm = Bootstrap::try_parse_from(base.into_iter().chain(["--vm-vsock"])).unwrap();
        assert!(vm.bindings(true).is_err());
        assert!(vm.bindings(false).unwrap().values().all(|ips| ips == &[Ipv4Addr::LOCALHOST]));
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
        let conflicting =
            Bootstrap::try_parse_from(base.into_iter().chain(["--host-bindings", &json, "--vm-vsock"])).unwrap();
        assert!(conflicting.bindings(false).is_err());
    }
}
