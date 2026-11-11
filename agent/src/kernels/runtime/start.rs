// SPDX-License-Identifier: FSL-1.1-ALv2
//! Supervised kernel launch and Jupyter channel readiness; execution remains in the parent.
use super::*;

impl Session {
    pub(crate) async fn start(owner: &Processes, language: Language) -> Result<Arc<Self>, Failure> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let directory = tempfile::Builder::new().prefix("pryx-kernel-").tempdir()?;
        let path = directory.path().join("connection.json");
        let ip = std::net::Ipv4Addr::LOCALHOST;
        let (ports, reservations) = jupyter_zmq_client::peek_ports_with_listeners(ip.into(), CHANNELS)
            .await
            .map_err(|_| failure("kernel_start_failed", "Cannot reserve kernel channels"))?;
        let info = ConnectionInfo {
            transport: jupyter_protocol::connection_info::Transport::TCP,
            ip: ip.to_string(),
            stdin_port: ports[0],
            control_port: ports[1],
            hb_port: ports[2],
            shell_port: ports[3],
            iopub_port: ports[4],
            key: uuid::Uuid::new_v4().to_string(),
            signature_scheme: SIGNATURE.into(),
            kernel_name: None,
        };
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
        file.write_all(serde_json::to_string(&info).expect("ConnectionInfo serializes").as_bytes())?;
        drop(file);
        let config = path.to_string_lossy().into_owned();
        let command = match language {
            Language::Python => {
                let mut command = vec!["python3".into(), "-m".into(), "ipykernel_launcher".into(), "-f".into(), config];
                // Read image configuration explicitly; runtime history/state belongs in the
                // private writable directory, never the image's root-owned IPython profile.
                if let Some(root) = std::env::var_os("IPYTHONDIR") {
                    command.extend([
                        "--config".into(),
                        std::path::Path::new(&root)
                            .join("profile_default/ipython_config.py")
                            .to_string_lossy()
                            .into_owned(),
                    ]);
                }
                command
            }
            Language::Javascript => vec![
                "node".into(),
                "--require".into(),
                "/opt/pryx/interrupt.cjs".into(),
                "/opt/pryx/kernel.cjs".into(),
                config,
            ],
        };
        let mut env = BTreeMap::new();
        env.insert("MPLCONFIGDIR".into(), directory.path().join("matplotlib").to_string_lossy().into_owned());
        env.insert("JUPYTER_RUNTIME_DIR".into(), directory.path().to_string_lossy().into_owned());
        env.insert("IPYTHONDIR".into(), directory.path().join("ipython").to_string_lossy().into_owned());
        env.insert("PYTHONUNBUFFERED".into(), "1".into());
        let process = owner.start(
            Start {
                command: Command::Argv(command),
                env,
                cwd: None,
                stdin: false,
                pty: false,
                cols: None,
                rows: None,
                timeout: None,
            },
            &uuid::Uuid::new_v4().to_string(),
            None,
        )?;
        let mut starting = Starting(Some(process.clone()));
        drop(reservations);
        let connected = async {
            let session = uuid::Uuid::new_v4().to_string();
            let iopub = jupyter_zmq_client::create_client_iopub_connection(&info, "", &session).await?;
            let identity = jupyter_zmq_client::peer_identity_for_session(&session)?;
            let shell =
                jupyter_zmq_client::create_client_shell_connection_with_identity(&info, &session, identity).await?;
            Ok::<_, jupyter_zmq_client::RuntimeError>(Channels { shell, iopub })
        };
        let mut channels = tokio::select! {
            result = tokio::time::timeout(STARTUP, connected) => result
                .map_err(|_| failure("kernel_start_timeout", "Kernel did not bind its channels"))?
                .map_err(|_| failure("kernel_start_failed", "Cannot connect kernel channels"))?,
            _ = process.wait() => return Err(failure("kernel_died", "Kernel exited during startup")),
        };
        let ready = async {
            loop {
                let request: JupyterMessage = KernelInfoRequest {}.into();
                let id = request.header.msg_id.clone();
                channels
                    .shell
                    .send(request)
                    .await
                    .map_err(|_| failure("kernel_protocol", "Kernel handshake failed"))?;
                let round = async {
                    let (shell, iopub) = (&mut channels.shell, &mut channels.iopub);
                    let (mut replied, mut idle) = (false, false);
                    while !replied || !idle {
                        tokio::select! {
                            result = shell.read() => {
                                let message = result.map_err(|_| failure("kernel_protocol", "Invalid kernel reply"))?;
                                if parent(&message, &id) && matches!(message.content, JupyterMessageContent::KernelInfoReply(_)) { replied = true; }
                            },
                            result = iopub.read() => {
                                let message = result.map_err(|_| failure("kernel_protocol", "Invalid kernel publication"))?;
                                if parent(&message, &id) && matches!(message.content, JupyterMessageContent::Status(ref s) if s.execution_state == ExecutionState::Idle) { idle = true; }
                            },
                            _ = process.wait() => return Err(failure("kernel_died", "Kernel exited during startup")),
                        }
                    }
                    Ok::<_, Failure>(())
                };
                if let Ok(result) = tokio::time::timeout(Duration::from_millis(500), round).await {
                    break result;
                }
            }
        };
        tokio::time::timeout(STARTUP, ready)
            .await
            .map_err(|_| failure("kernel_start_timeout", "Kernel did not become ready"))??;
        starting.0.take();
        Ok(Arc::new(Self {
            process,
            cancel: CancellationToken::new(),
            busy: AtomicBool::new(false),
            jobs: Arc::new(Semaphore::new(16)),
            channels: Mutex::new(channels),
            _directory: directory,
        }))
    }
}
