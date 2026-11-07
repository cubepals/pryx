// SPDX-License-Identifier: FSL-1.1-ALv2
//! Virtual process supervision translates the existing agent contract through the pinned Rust
//! client. Retained sidecar output is authoritative; callbacks only coalesce wake notifications.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use agentos_client::{AgentOs, OpenShellOptions, ProcessStream, SpawnOptions, StdinInput};
use async_trait::async_trait;
use pryx_agent::process::{Backend, Command, Control, Process, ProcessError, Start};
use tokio::sync::Notify;

pub(super) struct Guest {
    pub vm: AgentOs,
    pub environment: BTreeMap<String, String>,
    pub cwd: String,
    next_terminal: AtomicU32,
    status: Arc<std::sync::atomic::AtomicU8>,
}

impl Guest {
    pub(super) fn new(
        vm: AgentOs,
        environment: BTreeMap<String, String>,
        cwd: String,
        status: Arc<std::sync::atomic::AtomicU8>,
    ) -> Self {
        Self { vm, environment, cwd, status, next_terminal: AtomicU32::new(1) }
    }
}

enum Handle {
    Process(u32),
    Terminal(String),
}

struct Remote {
    vm: AgentOs,
    handle: Handle,
    status: Arc<std::sync::atomic::AtomicU8>,
}

fn error(error: impl Into<anyhow::Error>) -> ProcessError {
    let error = error.into();
    let code = match error.downcast_ref::<agentos_client::ClientError>() {
        Some(
            agentos_client::ClientError::Kernel { code, .. } | agentos_client::ClientError::ResourceLimit { code, .. },
        ) => code.as_str(),
        Some(agentos_client::ClientError::OperationTimedOut { .. }) => "operation_timeout",
        Some(agentos_client::ClientError::TerminationFailed { .. }) => "termination_unconfirmed",
        Some(agentos_client::ClientError::ProcessNotFound(_)) => "process_not_found",
        Some(agentos_client::ClientError::Sidecar(_)) => "sidecar_failure",
        _ => "backend_failure",
    };
    tracing::warn!(code, "agentOS process operation failed");
    ProcessError::Io(std::io::Error::other(format!("agentOS process operation failed: {code}")))
}

impl Backend for Guest {
    fn spawn(&self, request: Start, timeout: Option<u32>) -> Result<Arc<Process>, ProcessError> {
        if self.status.load(Ordering::Acquire) != super::VM_RUNNING {
            return Err(ProcessError::Invalid("VM is closed"));
        }
        let (command, mut args) = match request.command {
            Command::Shell(script) => ("sh".into(), vec!["-c".into(), script]),
            Command::Argv(mut argv) if !argv.is_empty() && !argv[0].is_empty() => (argv.remove(0), argv),
            _ => return Err(ProcessError::Invalid("empty command")),
        };
        // Bundled brush-shell's reedline input drops subsequent pasted lines on this PTY.
        // Select brush's documented minimal input backend for ordered batched stdin.
        if request.pty && command == "sh" && args.is_empty() {
            args.extend(["--input-backend".into(), "minimal".into()]);
        }
        let mut env = self.environment.clone();
        env.extend(request.env);
        let cwd = Some(request.cwd.unwrap_or_else(|| self.cwd.clone()));
        let (handle, pid) = if request.pty {
            let terminal = self
                .vm
                .open_shell(OpenShellOptions {
                    command: Some(command),
                    args,
                    env,
                    cwd,
                    cols: request.cols,
                    rows: request.rows,
                })
                .map_err(error)?;
            (Handle::Terminal(terminal.shell_id), self.next_terminal.fetch_add(1, Ordering::Relaxed))
        } else {
            let process = self
                .vm
                .spawn_process(
                    &command,
                    args,
                    SpawnOptions {
                        env,
                        cwd,
                        stream_stdin: Some(request.stdin),
                        retain_output: true,
                        ..Default::default()
                    },
                )
                .map_err(error)?;
            (Handle::Process(process.pid), process.pid)
        };
        let remote = Arc::new(Remote { vm: self.vm.clone(), handle, status: self.status.clone() });
        let process = Process::guest(pid, remote.clone());
        let owned = process.clone();
        tokio::spawn(async move { remote.supervise(owned, timeout).await });
        Ok(process)
    }
}

#[async_trait]
impl Control for Remote {
    async fn write(&self, bytes: &[u8]) -> Result<(), ProcessError> {
        let input = StdinInput::Bytes(bytes.to_vec());
        match &self.handle {
            Handle::Process(pid) => self.vm.write_process_stdin_awaited(*pid, input).await.map_err(error),
            Handle::Terminal(id) => self.vm.write_shell_awaited(id, input).await.map_err(error),
        }
    }

    async fn close(&self) -> Result<(), ProcessError> {
        match &self.handle {
            Handle::Process(pid) => self.vm.close_process_stdin_awaited(*pid).await.map_err(error),
            Handle::Terminal(_) => self.write(b"\x04").await,
        }
    }

    async fn signal(&self, signal: i32) -> Result<(), ProcessError> {
        let name = match signal {
            libc::SIGTERM => "SIGTERM",
            libc::SIGKILL => "SIGKILL",
            libc::SIGINT => "SIGINT",
            libc::SIGHUP => "SIGHUP",
            libc::SIGUSR1 => "SIGUSR1",
            libc::SIGUSR2 => "SIGUSR2",
            libc::SIGWINCH => "SIGWINCH",
            _ => return Err(ProcessError::Invalid("signal")),
        };
        match &self.handle {
            Handle::Process(pid) => self.vm.signal_process_awaited(*pid, name).await.map_err(error),
            Handle::Terminal(id) if signal == libc::SIGKILL || signal == libc::SIGTERM => {
                self.vm.close_shell_awaited(id).await.map_err(error)
            }
            Handle::Terminal(_) if signal == libc::SIGINT => self.write(b"\x03").await,
            Handle::Terminal(id) => {
                let pid = self.vm.list_shells().iter().find(|s| s.shell_id == *id).ok_or(ProcessError::NotFound)?.pid;
                let result = self
                    .vm
                    .exec_process(
                        &format!("kill -{} {pid}", name.trim_start_matches("SIG")),
                        agentos_client::ExecOptions {
                            cwd: Some("/".into()),
                            capture_stdio: Some(true),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(error)?;
                if result.exit_code == 0 { Ok(()) } else { Err(ProcessError::Invalid("terminal signal refused")) }
            }
        }
    }

    async fn resize(&self, cols: u16, rows: u16) -> Result<(), ProcessError> {
        match &self.handle {
            Handle::Process(_) => Err(ProcessError::Invalid("process has no terminal")),
            Handle::Terminal(id) => self.vm.resize_shell_awaited(id, cols, rows).await.map_err(error),
        }
    }
}

impl Remote {
    async fn wait(&self) -> Result<i32, agentos_client::ClientError> {
        match &self.handle {
            Handle::Process(pid) => self.vm.wait_process(*pid).await,
            Handle::Terminal(id) => self.vm.wait_shell(id).await,
        }
    }

    async fn output(&self, process: &Process) -> Result<i32, ProcessError> {
        let changed = Arc::new(Notify::new());
        let wake = changed.clone();
        let _subscription = match &self.handle {
            Handle::Process(pid) => self.vm.on_process_output(*pid, move |_| wake.notify_one()).map_err(error)?,
            Handle::Terminal(id) => self.vm.on_shell_data(id, move |_| wake.notify_one()).map_err(error)?,
        };
        let mut cursor = None;
        loop {
            let (events, next, more, truncated, exited) = self.page(cursor).await?;
            if truncated {
                process.guest_truncated().await;
            }
            for (stream, bytes) in events {
                process.guest_output(&bytes, if stream == ProcessStream::Stderr { "stderr" } else { "stdout" }).await;
            }
            cursor = next.or(cursor);
            if !more {
                // The wait notification can precede the final retained-output page. Only
                // the replay owner's exit marker proves that stdout/stderr were drained.
                if let Some(code) = exited {
                    return Ok(code);
                }
                tokio::select! {
                    () = changed.notified() => {},
                    result = self.wait() => { result.map_err(error)?; },
                }
            }
        }
    }

    async fn page(&self, cursor: Option<u64>) -> Result<Page, ProcessError> {
        let limit = None;
        match &self.handle {
            Handle::Process(pid) => {
                let page = self.vm.read_process_output(*pid, cursor, Some(32), limit).await.map_err(error)?;
                Ok((
                    page.events.into_iter().map(|e| (e.stream, e.data)).collect(),
                    page.next_cursor,
                    page.has_more,
                    page.truncated,
                    page.exit_code,
                ))
            }
            Handle::Terminal(id) => {
                let page = self.vm.snapshot_shell_page(id, cursor, Some(32), limit).await.map_err(error)?;
                Ok((
                    page.events.into_iter().map(|e| (e.stream, e.data)).collect(),
                    page.next_cursor,
                    page.has_more,
                    page.truncated,
                    page.exit_code,
                ))
            }
        }
    }

    async fn supervise(&self, process: Arc<Process>, timeout: Option<u32>) {
        let output = self.output(&process);
        tokio::pin!(output);
        let deadline = async {
            match timeout {
                Some(seconds) => tokio::time::sleep(Duration::from_secs(seconds.into())).await,
                None => std::future::pending().await,
            }
        };
        let (result, timed_out) = tokio::select! {
            result = &mut output => (result.ok(), false),
            () = deadline => {
                if self.signal(libc::SIGKILL).await.is_err() {
                    tracing::error!("agentOS timed-out process termination was not acknowledged");
                }
                (tokio::time::timeout(Duration::from_secs(3), &mut output).await
                    .ok().and_then(Result::ok), true)
            }
        };
        // Lifecycle disposal intentionally closes output observation. It already owns the
        // VM shutdown; an old process must not start a second disposal during recreation.
        if result.is_none() && self.status.load(Ordering::Acquire) == super::VM_RUNNING {
            match self.vm.shutdown().await {
                Ok(()) => self.status.store(super::VM_DISPOSED, Ordering::Release),
                Err(_) => {
                    self.status.store(super::VM_UNCERTAIN, Ordering::Release);
                    tracing::error!("agentOS uncertain process ownership could not be disposed");
                }
            }
        }
        process.guest_finished(result, timed_out).await;
    }
}

type Page = (Vec<(ProcessStream, Vec<u8>)>, Option<u64>, bool, bool, Option<i32>);

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn real_terminal_exit_and_immediate_http_listener() {
        let assets = std::env::var_os("PRYX_AGENTOS_DISTRIBUTION")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| crate::config::WasmConfig::default().distribution);
        let options = super::super::boot::options(
            None,
            &assets,
            &super::super::boot::binary(&assets),
            &uuid::Uuid::new_v4().to_string(),
            (rustix::process::geteuid().as_raw(), rustix::process::getegid().as_raw()),
        )
        .unwrap();
        let vm = AgentOs::create(options).await.unwrap();
        immediate_http(&vm).await;
        let guest = Guest::new(
            vm.clone(),
            Default::default(),
            "/".into(),
            Arc::new(std::sync::atomic::AtomicU8::new(super::super::VM_RUNNING)),
        );
        let terminal = guest
            .spawn(
                Start {
                    command: Command::Argv(vec!["sh".into()]),
                    env: Default::default(),
                    cwd: None,
                    stdin: true,
                    pty: true,
                    cols: None,
                    rows: None,
                    timeout: None,
                },
                None,
            )
            .unwrap();
        terminal.write(b"printf 'cli:%s\\n' ready\nexit\n").await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(20), terminal.wait()).await;
        if result.is_err() {
            let (events, _) = terminal.subscribe().await;
            eprintln!("terminal retained events: {events:?}");
            eprintln!("shells: {:?}", vm.list_shells());
        }
        assert_eq!(result.unwrap().code, Some(0));
        vm.shutdown().await.unwrap();
    }

    async fn immediate_http(vm: &AgentOs) {
        let process = vm
            .spawn_process(
                "node",
                vec![
                    "-e".into(),
                    "require('node:http').createServer((req,res)=>res.end('ready')).listen(3000,'0.0.0.0')".into(),
                ],
                SpawnOptions::default(),
            )
            .unwrap();
        let began = std::time::Instant::now();
        let head = super::super::network::start(
            vm,
            agentos_client::net::HttpRequest {
                port: 3000,
                path: "/".into(),
                method: "GET".into(),
                headers: Default::default(),
                body: None,
            },
        )
        .await;
        if let Err(error) = &head {
            eprintln!("guest HTTP readiness failed after {:?}: {error:?}", began.elapsed());
            eprintln!("guest processes: {:?}", vm.list_processes());
            eprintln!("listener output: {:?}", vm.read_process_output(process.pid, None, Some(32), None).await);
        }
        let head = head.unwrap();
        assert_eq!(head.status, hyper::StatusCode::OK.as_u16());
        assert_eq!(vm.http_request_stream_read(&head.stream_id, 1024).await.unwrap().body, b"ready");
        vm.signal_process_awaited(process.pid, "SIGTERM").await.unwrap();
        vm.wait_process(process.pid).await.unwrap();
    }
}
