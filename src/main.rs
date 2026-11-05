// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

//! pryxd: pryx's node daemon. See README.md.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use pryxd::api::tls::ServerCert;
use pryxd::api::{self, AppState};
use pryxd::config::{Config, FleetConfig, Mode, RuntimeKind, TlsConfig};
use pryxd::fleet::identity::{Credentials, Identity};
use pryxd::manager::{Manager, Records};
use pryxd::metrics::Metrics;
use pryxd::runtime::ContainerRuntime;
use pryxd::runtime::docker::DockerRuntime;
use pryxd::store::Store;
use pryxd::upgrade::Upgrader;
use pryxd::upgrade::backstop::Backstop;
use pryxd::{certs, doctor, ports, reconcile};

#[derive(Parser)]
#[command(name = "pryxd", version, about = "pryx's node daemon: runs sandboxes on this host for a pryx control plane.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum LogFormat {
    Json,
    Pretty,
}

#[derive(Subcommand)]
enum Command {
    #[command(hide = true)]
    LeaseWatchdog {
        #[arg(long)]
        group: PathBuf,
        #[arg(long)]
        deadline: u64,
    },
    /// Run the daemon.
    Serve {
        #[arg(long, default_value = pryxd::config::PATH)]
        config: PathBuf,
        #[arg(long, value_enum, default_value = "json")]
        log_format: LogFormat,
    },
    /// Check a configuration file (and its certificates) and exit: doctor's first check alone.
    CheckConfig {
        #[arg(long)]
        config: PathBuf,
    },
    /// Reserve configured workload ports from the host's outgoing ephemeral allocation.
    ReservePorts {
        #[arg(long, default_value = pryxd::config::PATH)]
        config: PathBuf,
    },
    /// Make this host a node of the fleet a pasted token names: its configuration, the fleet CA,
    /// the token and the systemd unit, then start it. A token for the node this host already is
    /// enrolls it again under its id.
    Join {
        /// The `bk1.` token `bun scripts/fleet.ts token` prints.
        token: String,
        /// The address to serve the fleet on, when the host has several private ones or none.
        #[arg(long)]
        address: Option<IpAddr>,
        /// Write everything, but leave the service stopped.
        #[arg(long)]
        no_start: bool,
        /// Image reference prefix allowed on this host, repeated for each prefix.
        #[arg(long = "allow-image")]
        allow_images: Vec<String>,
        /// Where `/` is, for tests.
        #[arg(long, hide = true, default_value = "/")]
        root: PathBuf,
        /// The configuration, for tests that keep it elsewhere.
        #[arg(long, hide = true, default_value = pryxd::config::PATH)]
        config: PathBuf,
        /// What to run as systemctl, for tests that run pryxd as a plain process.
        #[arg(long, hide = true, default_value = "systemctl")]
        systemctl: PathBuf,
    },
    /// Check this host the way pryxd will use it, and say what to fix.
    Doctor {
        #[arg(long, default_value = pryxd::config::PATH)]
        config: PathBuf,
        /// Print {checks: [{name, status, detail, fix}]} instead of text.
        #[arg(long)]
        json: bool,
        /// Fail only on what keeps pryxd from starting, and report the rest as warnings.
        #[arg(long)]
        preflight: bool,
        /// First apply the safe fixes (daemon.json, the state directory) on a host with no sandboxes.
        #[arg(long)]
        fix: bool,
    },
    /// Keep workloads off this host, and off the ports pryxd serves on the private network: the
    /// host's firewall rules (one nftables table), as root. The unit runs it before every start.
    Isolate {
        #[arg(long, default_value = pryxd::config::PATH)]
        config: PathBuf,
    },
    /// Upgrade to the pryxd the control plane serves, and restart into it. It puts this one back
    /// by itself if the new one doesn't come up.
    Upgrade {
        #[arg(long, default_value = pryxd::config::PATH)]
        config: PathBuf,
        /// Install it, and leave the running pryxd as it is until the next restart.
        #[arg(long)]
        no_restart: bool,
        /// After the service stopped (its ExecStopPost): put this binary back if the one on trial
        /// stopped before it came up, as systemd's SERVICE_RESULT (or EXIT_CODE and EXIT_STATUS) says.
        #[arg(long, hide = true)]
        exited: bool,
    },
    /// Write a throwaway CA, a node certificate and a client certificate, for local use only.
    DevCerts {
        #[arg(long)]
        out: PathBuf,
        #[arg(long = "node-dns", default_value = "localhost")]
        node_dns: Vec<String>,
        #[arg(long = "node-ip", default_value = "127.0.0.1")]
        node_ip: Vec<IpAddr>,
        #[arg(long, default_value = "control-plane.dev.blockly.internal")]
        client: String,
        /// Also issue a client certificate for this name from the same CA (repeatable).
        #[arg(long = "extra-client")]
        extra_client: Vec<String>,
    },
}

fn init_tracing(format: LogFormat) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,hyper=warn,bollard=warn,rustls=warn"));
    match format {
        LogFormat::Json => tracing_subscriber::fmt()
            .json()
            .with_current_span(true)
            .with_span_list(false)
            .with_env_filter(filter)
            .init(),
        LogFormat::Pretty => tracing_subscriber::fmt().with_env_filter(filter).init(),
    }
}

fn main() -> ExitCode {
    if let Some(error) = pryxd::runtime::wasm::dispatch_sidecar() {
        eprintln!("agentOS launcher refused: {error}");
        return ExitCode::FAILURE;
    }
    #[cfg(debug_assertions)]
    if pryxd::upgrade::debug_legacy_executable() && std::env::args().skip(1).eq(["--version"]) {
        println!("blocklyd {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let cli = Cli::parse();
    let result = match cli.command {
        Command::LeaseWatchdog { group, deadline } => {
            pryxd::store_lease::guard::run(&group, deadline).map_err(Into::into)
        }
        Command::Serve { config, log_format } => {
            init_tracing(log_format);
            // Two workers are plenty: pryxd waits on Docker and the network, and blocking
            // work (disk walks) goes to tokio's blocking pool.
            match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
                Ok(runtime) => {
                    let result = runtime.block_on(serve(&config));
                    // Blocking work in flight (a disk walk) can't be cancelled; don't let it hold
                    // the exit hostage.
                    runtime.shutdown_timeout(Duration::from_secs(5));
                    result
                }
                Err(e) => Err(e.into()),
            }
        }
        Command::ReservePorts { config } => Config::load(&config).map_err(anyhow::Error::from).and_then(|config| {
            pryxd::cli::port_reservation::reserve(config.network.port_range, std::path::Path::new("/"))
        }),
        Command::CheckConfig { config } => {
            let report = doctor::Report { checks: vec![doctor::check_config(&config).0], fixed: None };
            print_report(&report, false, false)
        }
        Command::Doctor { config, json, preflight, fix } => {
            match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => {
                    let report = runtime.block_on(doctor::run(&doctor::Options { config, preflight, fix }));
                    print_report(&report, json, preflight)
                }
                Err(e) => Err(e.into()),
            }
        }
        Command::Join { token, address, no_start, allow_images, root, config, systemctl } => {
            let join =
                pryxd::cli::join::Join { token, address, config, root, start: !no_start, systemctl, allow_images };
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(anyhow::Error::from)
                .and_then(|runtime| runtime.block_on(pryxd::cli::join::join(join)))
                .map(|said| print!("{said}"))
        }
        Command::Isolate { config } => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|runtime| runtime.block_on(pryxd::isolation::isolate(&config)))
            .map(|said| print!("{said}")),
        Command::Upgrade { config, exited: true, .. } => {
            let state_dir = pryxd::upgrade::stopped::state_dir(&config);
            let layout = pryxd::upgrade::Layout::new(&state_dir, Path::new("/"));
            let var = |name: &str| std::env::var(name).ok();
            let (result, code, status) = (var("SERVICE_RESULT"), var("EXIT_CODE"), var("EXIT_STATUS"));
            pryxd::upgrade::exited(&layout, result.as_deref(), code.as_deref(), status.as_deref())
                .map(|said| eprint!("{said}"))
        }
        Command::Upgrade { config, no_restart, .. } => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|runtime| runtime.block_on(pryxd::cli::upgrade::upgrade(&config, !no_restart)))
            .map(|said| print!("{said}")),
        Command::DevCerts { out, node_dns, node_ip, client, extra_client } => {
            certs::generate(&out, &node_dns, &node_ip, &client, &extra_client).map(|paths| {
                println!("CA certificate:     {}", paths.ca_cert.display());
                println!("node certificate:   {} (key {})", paths.node_cert.display(), paths.node_key.display());
                println!("client certificate: {} (key {})", paths.client_cert.display(), paths.client_key.display());
                println!("client name to allow: {client}");
            })
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!(error = format!("{e:#}"), "pryxd failed");
            eprintln!("pryxd: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Prints a doctor report; a failure in it is pryxd's exit status.
fn print_report(report: &doctor::Report, json: bool, preflight: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        print!("{}", report.text(preflight));
    }
    anyhow::ensure!(report.ok(), "the host isn't ready (above)");
    Ok(())
}

/// What a node says about itself when it enrolls.
fn facts(config: &Config, fleet: &FleetConfig) -> pryxd::protocol::wire::NodeFacts {
    let host = config.host_facts();
    let memory_total_mb = host.memory_total_bytes.map_or(0, |b| b / (1024 * 1024));
    let (disk_total, disk_available) = pryxd::host::disk(&config.state_dir).unzip();
    let [low, high] = config.network.port_range;
    pryxd::protocol::wire::NodeFacts {
        hostname: host.hostname,
        boot_id: pryxd::host::boot_id(),
        machine_id_sha256: pryxd::fleet::enroll::machine_id_sha256(),
        daemon_version: pryxd::upgrade::daemon_version(),
        protocol: pryxd::protocol::ProtocolVersions::ours(),
        features: pryxd::protocol::features(),
        api_address: fleet.api_address.unwrap_or(config.api.listen),
        edge_ips: config.network.edge_ips.clone(),
        control_ips: config.network.control_ips.clone(),
        capacity: pryxd::protocol::wire::NodeCapacity {
            memory_total_mb,
            reserved_memory_mb: config.capacity.reserved_memory_mb,
            allocatable_memory_mb: config.capacity.allocatable_mb(memory_total_mb),
            cpus: host.cpus,
            reserved_cpu_millis: config.capacity.reserved_cpu_millis,
            load_average: host.load_average,
            disk_total_bytes: disk_total,
            disk_available_bytes: disk_available,
            min_free_disk_mb: config.capacity.min_free_disk_mb,
            ports_total: u32::from(high - low) + 1,
            ..Default::default()
        },
        labels: fleet.labels.clone(),
    }
}

async fn shutdown_signal() -> &'static str {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("SIGINT handler");
    tokio::select! {
        _ = term.recv() => "SIGTERM",
        _ = int.recv() => "SIGINT",
    }
}

fn publish_addresses(config: &Config) -> Vec<IpAddr> {
    let mut publish: Vec<IpAddr> = config.network.edge_ips.clone();
    publish.extend(config.network.control_ips.iter().copied());
    publish.dedup();
    publish
}

async fn container_runtime(config: &Config) -> anyhow::Result<Arc<dyn ContainerRuntime>> {
    let runtime: Arc<dyn ContainerRuntime> = if config.runtime == RuntimeKind::Wasm {
        Arc::new(pryxd::runtime::wasm::WasmRuntime::new(&config.state_dir, &config.wasm).await?)
    } else if config.runtime == RuntimeKind::Process {
        Arc::new(pryxd::runtime::process::ProcessRuntime::new(&config.state_dir, config.process.clone())?)
    } else {
        let docker = DockerRuntime::new(&config.docker.socket)?;
        if config.store_lease.enabled {
            docker.check_store_lease().await?;
            pryxd::store_lease::check_policy(&config.state_dir, true)?;
        }
        Arc::new(docker)
    };
    Ok(runtime)
}

async fn serve(path: &Path) -> anyhow::Result<()> {
    let boot = Instant::now();
    let mut config = Config::load(path)?;
    info!(
        node = %config.node_id,
        deployment = %config.deployment_id,
        fleet = config.fleet.as_ref().map(|f| f.url.as_str()),
        version = env!("CARGO_PKG_VERSION"),
        "pryxd starting"
    );
    for inferred in &config.inferred {
        info!(key = inferred.key, value = %inferred.value, from = %inferred.from, "worked out from the host; set it in the configuration to override");
    }
    let _ = rustls::crypto::ring::default_provider().install_default();

    let store = Store::open(&config.state_dir)?;
    // Held until exit: a second pryxd on this state directory refuses to start.
    let _lock = store.lock()?;
    // A start on trial after an upgrade ends itself if the trial hasn't ended long after it began,
    // whatever the startup below does (upgrade/backstop.rs). Armed once the state directory is
    // this process's alone: arming drops a trial left for another version.
    let backstop = Backstop::arm(&config.state_dir, boot);
    // Every create gives a data directory to the workloads' user, which takes root (CAP_CHOWN): a
    // daemon that can't is refused here, rather than by every create that follows.
    store.check_ownable(if config.runtime != RuntimeKind::Container {
        (rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw())
    } else {
        config.data_owner_ids()
    })?;
    // Fleet mode: the node's id and TLS material come from enrollment, before anything is served.
    let (identity, tls_material): (Option<Identity>, TlsConfig) = match config.mode()? {
        Mode::Fleet(fleet) => {
            let fleet = fleet.clone();
            let identity = pryxd::fleet::enroll::ensure_identity(&config, &fleet, facts(&config, &fleet)).await?;
            config.node_id = identity.node_id().clone();
            let tls = identity.server_tls()?;
            (Some(identity), tls)
        }
        Mode::Standalone(tls) => (None, tls.clone()),
    };
    let config = Arc::new(config);
    let (tls, server_cert) = api::tls::reloadable_server_config(&tls_material).context("loading TLS material")?;

    if !config.store_lease.enabled {
        pryxd::store_lease::check_policy(&config.state_dir, false)?;
    }
    let runtime = container_runtime(&config).await?;
    let publish = publish_addresses(&config);
    let manager = Manager::new(config.clone(), runtime, store, Arc::new(Metrics::new()), ports::bind_probe(publish));
    if let (Some(identity), Some(fleet)) = (&identity, &config.fleet) {
        manager.set_fleet(identity.node_id(), &fleet.url);
    }

    // Rebuild everything from disk and the runtime before answering anyone. If Docker is down
    // pryxd still starts, knows its records, and says it is degraded until Docker is back.
    let report = manager.reconcile(Records::FromDisk).await;
    manager.stop_unleased().await?;
    match &report.error {
        None => info!(
            workloads = report.workloads,
            adopted = report.adopted,
            issues = report.issues,
            took_ms = report.duration_ms,
            "reconciled with the runtime"
        ),
        Some(e) => warn!(error = %e, "couldn't reconcile with the runtime; serving degraded and retrying"),
    }

    let cancel = CancellationToken::new();
    let api_listener =
        TcpListener::bind(config.api.listen).await.with_context(|| format!("binding {}", config.api.listen))?;
    let ops_listener =
        TcpListener::bind(config.ops.listen).await.with_context(|| format!("binding {}", config.ops.listen))?;
    let app = api::router(AppState { manager: manager.clone() }, config.api.max_body_bytes);

    let ops = tokio::spawn(api::serve_ops(ops_listener, manager.clone(), cancel.clone()));
    let events = tokio::spawn(reconcile::follow_events(manager.clone(), cancel.clone()));
    let periodic = tokio::spawn(reconcile::periodic(manager.clone(), cancel.clone()));
    let restarts = tokio::spawn(manager.clone().restart_supervisor(cancel.clone()));
    // Heartbeats start once the node has reconciled, so the first one reports what is really there.
    let fleet = match (identity, &config.fleet) {
        (Some(identity), Some(fleet)) => Some(start_fleet(&manager, identity, server_cert, fleet, &backstop, &cancel)?),
        _ => None,
    };
    let api_task = tokio::spawn(api::serve_tls(
        api_listener,
        tls,
        tls_material.allowed_clients.clone(),
        app,
        manager.clone(),
        cancel.clone(),
        api::Bounds {
            grace: Duration::from_secs(config.api.shutdown_grace_seconds),
            handshakes: api::HandshakeLimits::default(),
        },
    ));
    info!(
        api = %config.api.listen,
        ops = %config.ops.listen,
        startup_ms = boot.elapsed().as_millis() as u64,
        "ready"
    );

    until_stopped(&cancel).await;
    cancel.cancel();
    let _ = api_task.await;
    let _ = tokio::join!(ops, events, periodic, restarts);
    // An upgrade on trial that was put back stops pryxd with an error, saying why.
    let rolled_back = match fleet {
        Some(fleet) => fleet.stopped().await,
        None => None,
    };
    info!("stopped");
    rolled_back.map_or(Ok(()), |reason| Err(anyhow::anyhow!(reason)))
}

/// Fleet mode's own tasks, and what decides how pryxd stops.
struct FleetTasks {
    heartbeat: tokio::task::JoinHandle<()>,
    trial: tokio::task::JoinHandle<()>,
    upgrader: Arc<Upgrader>,
}

impl FleetTasks {
    /// Once they have ended: why an upgrade on trial was put back, if it was.
    async fn stopped(self) -> Option<String> {
        let _ = tokio::join!(self.heartbeat, self.trial);
        self.upgrader.rolled_back()
    }
}

/// Until a signal, or until pryxd stops itself so that systemd runs the binary now in place:
/// an upgrade installed, or one on trial put back.
async fn until_stopped(cancel: &CancellationToken) {
    tokio::select! {
        signal = shutdown_signal() => info!(signal, "shutting down; workloads keep running"),
        () = cancel.cancelled() => info!("stopping to run the pryxd now installed; workloads keep running"),
    }
}

/// Fleet mode's own tasks: the heartbeat, and the trial a start after an upgrade is on until it
/// reconciles and a heartbeat is accepted (upgrade.rs).
fn start_fleet(
    manager: &Arc<Manager>,
    identity: Identity,
    server_cert: Arc<ServerCert>,
    fleet: &FleetConfig,
    backstop: &Backstop,
    cancel: &CancellationToken,
) -> anyhow::Result<FleetTasks> {
    let credentials = Credentials::new(identity, server_cert).context("the node's identity")?;
    let upgrader = Arc::new(Upgrader::new(&manager.config.state_dir, &fleet.url, backstop.ended(), cancel.clone()));
    let trial = tokio::spawn(upgrader.clone().watch_trial());
    let heartbeat = tokio::spawn(pryxd::fleet::heartbeat::run(
        manager.clone(),
        credentials,
        upgrader.clone(),
        fleet.url.clone(),
        fleet.heartbeat_seconds,
        cancel.clone(),
    ));
    Ok(FleetTasks { heartbeat, trial, upgrader })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_takes_each_allowed_image_prefix() {
        let cli = Cli::try_parse_from([
            "pryxd",
            "join",
            "token",
            "--allow-image",
            "alpine:",
            "--allow-image",
            "ghcr.io/example/",
            "--no-start",
        ])
        .unwrap();
        let Command::Join { allow_images, .. } = cli.command else { panic!("join") };
        assert_eq!(allow_images, ["alpine:", "ghcr.io/example/"]);
    }
}
