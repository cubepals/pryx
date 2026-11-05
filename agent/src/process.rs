// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Processes belong to the agent, not an HTTP connection. Output is bounded when detached and
//! applies backpressure to attached readers. Commands, environment and tokens never enter views.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex as AsyncMutex, mpsc, watch};

use crate::pty::Terminal;

pub const MAX_PROCESSES: usize = 256;
pub use crate::contract::CAPTURE_BYTES;
const SUBSCRIBER_CHUNKS: usize = 32;
const RETAIN: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Command {
    Shell(String),
    Argv(Vec<String>),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Start {
    pub command: Command,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub stdin: bool,
    #[serde(default)]
    pub pty: bool,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub timeout: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct View {
    pub id: String,
    pub pid: u32,
    pub running: bool,
    pub result: Option<ResultView>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ResultView {
    pub code: Option<i32>,
    pub timed_out: bool,
    pub killed: bool,
    pub duration_ms: u64,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Event {
    Output { stream: &'static str, bytes: String },
    Truncated,
    Exit { result: ResultView },
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("Invalid process request: {0}")]
    Invalid(&'static str),
    #[error("Process not found")]
    NotFound,
    #[error("Process capacity reached")]
    Full,
    #[error("Idempotency key was used for a different request")]
    KeyConflict,
    #[error("Process operation failed: {0}")]
    Io(#[from] std::io::Error),
}

type Input = Box<dyn AsyncWrite + Unpin + Send>;
struct Output {
    replay: VecDeque<Event>,
    bytes: usize,
    dropped: bool,
    subscribers: Vec<mpsc::Sender<Event>>,
}

pub struct Process {
    pub id: String,
    pub pid: u32,
    pub(crate) input: AsyncMutex<Option<Input>>,
    pub(crate) terminal: Option<Terminal>,
    output: AsyncMutex<Output>,
    ended: watch::Sender<Option<ResultView>>,
    started: Instant,
}

impl Process {
    pub fn view(&self) -> View {
        let result = self.ended.borrow().clone();
        View { id: self.id.clone(), pid: self.pid, running: result.is_none(), result }
    }

    pub async fn wait(&self) -> ResultView {
        let mut ended = self.ended.subscribe();
        loop {
            if let Some(result) = ended.borrow_and_update().clone() {
                return result;
            }
            if ended.changed().await.is_err() {
                unreachable!("a Process owns its completion sender");
            }
        }
    }

    pub async fn write(&self, bytes: &[u8]) -> Result<(), ProcessError> {
        let mut input = self.input.lock().await;
        input.as_mut().ok_or(ProcessError::Invalid("stdin is closed"))?.write_all(bytes).await?;
        Ok(())
    }

    pub async fn close_input(&self) {
        self.input.lock().await.take();
    }

    pub fn signal(&self, signal: i32) -> Result<(), ProcessError> {
        if self.ended.borrow().is_some() {
            return Err(ProcessError::Invalid("process already exited"));
        }
        let pid = rustix::process::Pid::from_raw(self.pid as i32).ok_or(ProcessError::NotFound)?;
        let signal = rustix::process::Signal::from_named_raw(signal).ok_or(ProcessError::Invalid("signal"))?;
        rustix::process::kill_process_group(pid, signal).map_err(std::io::Error::from)?;
        Ok(())
    }

    pub async fn subscribe(&self) -> (Vec<Event>, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::channel(SUBSCRIBER_CHUNKS);
        let mut output = self.output.lock().await;
        let mut replay = Vec::new();
        if output.dropped {
            replay.push(Event::Truncated);
        }
        replay.extend(output.replay.iter().cloned());
        if self.ended.borrow().is_none() {
            output.subscribers.push(tx);
        }
        (replay, rx)
    }

    async fn emit(&self, event: Event, bytes: usize) {
        let mut output = self.output.lock().await;
        output.bytes += bytes;
        output.replay.push_back(event.clone());
        while output.bytes > CAPTURE_BYTES {
            if let Some(Event::Output { bytes, .. }) = output.replay.pop_front() {
                output.bytes = output.bytes.saturating_sub(bytes.len());
                output.dropped = true;
            }
        }
        // Holding the lock orders stdout, stderr, subscription and exit consistently.
        let mut live = Vec::new();
        for subscriber in output.subscribers.drain(..) {
            if matches!(tokio::time::timeout(Duration::from_secs(5), subscriber.send(event.clone())).await, Ok(Ok(())))
            {
                live.push(subscriber);
            }
        }
        output.subscribers = live;
    }

    async fn pump(self: Arc<Self>, mut reader: impl AsyncRead + Unpin, stream: &'static str) {
        let mut buffer = vec![0; 8192];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let bytes = STANDARD.encode(&buffer[..n]);
                    let size = bytes.len();
                    self.emit(Event::Output { stream, bytes }, size).await;
                }
            }
        }
    }

    async fn finish(&self, code: Option<i32>, timed_out: bool) {
        let dropped = self.output.lock().await.dropped;
        let result = ResultView {
            code,
            timed_out,
            killed: timed_out,
            duration_ms: self.started.elapsed().as_millis() as u64,
            stdout_truncated: dropped,
            stderr_truncated: dropped,
        };
        self.emit(Event::Exit { result: result.clone() }, 0).await;
        let mut output = self.output.lock().await;
        self.ended.send_replace(Some(result));
        output.subscribers.clear();
    }
}

struct Entry {
    process: Arc<Process>,
    key: String,
    fingerprint: Vec<u8>,
}

pub struct Processes {
    held: Mutex<HashMap<String, Entry>>,
    pub data: PathBuf,
    pub secret_env: String,
}

impl Processes {
    pub fn new(data: PathBuf, secret_env: String) -> Self {
        Self { held: Mutex::new(HashMap::new()), data, secret_env }
    }

    pub fn get(&self, id: &str) -> Result<Arc<Process>, ProcessError> {
        self.held.lock().unwrap().get(id).map(|e| e.process.clone()).ok_or(ProcessError::NotFound)
    }

    pub fn list(&self) -> Vec<View> {
        self.held.lock().unwrap().values().map(|e| e.process.view()).collect()
    }

    pub fn reap_orphans(&self) {
        // start holds this same lock from spawn through insertion, so a new managed child
        // cannot be mistaken for an orphan between those operations.
        let held = self.held.lock().unwrap();
        let Ok(tasks) = std::fs::read_dir("/proc/self/task") else { return };
        for task in tasks.flatten() {
            let Ok(children) = std::fs::read_to_string(task.path().join("children")) else { continue };
            for pid in children.split_whitespace().filter_map(|s| s.parse::<i32>().ok()) {
                if held.values().any(|e| e.process.pid == pid as u32) {
                    continue;
                }
                if let Some(pid) = rustix::process::Pid::from_raw(pid) {
                    let _ = rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG);
                }
            }
        }
    }

    pub fn start(&self, request: Start, key: &str, max_timeout: Option<u32>) -> Result<Arc<Process>, ProcessError> {
        let fingerprint = serde_json::to_vec(&request).expect("Start serializes");
        if key.is_empty() || key.len() > 128 || fingerprint.len() > crate::contract::MAX_REQUEST_BYTES {
            return Err(ProcessError::Invalid("idempotency key or request size"));
        }
        let mut held = self.held.lock().unwrap();
        held.retain(|_, entry| entry.process.view().running || entry.process.started.elapsed() < RETAIN);
        if let Some(entry) = held.values().find(|e| e.key == key) {
            return if entry.fingerprint == fingerprint {
                Ok(entry.process.clone())
            } else {
                Err(ProcessError::KeyConflict)
            };
        }
        if held.len() >= MAX_PROCESSES {
            return Err(ProcessError::Full);
        }
        let timeout = request.timeout.or(max_timeout);
        if timeout == Some(0) || max_timeout.is_some_and(|max| timeout.is_some_and(|t| t > max)) {
            return Err(ProcessError::Invalid("exec timeout exceeds token policy"));
        }
        let process = self.spawn(request, timeout)?;
        held.insert(process.id.clone(), Entry { process: process.clone(), key: key.into(), fingerprint });
        Ok(process)
    }

    fn spawn(&self, request: Start, timeout: Option<u32>) -> Result<Arc<Process>, ProcessError> {
        let argv = crate::process_policy::wrap(argv(&request.command)?)?;
        let cwd = request.cwd.as_ref().map(PathBuf::from).unwrap_or_else(|| self.data.clone());
        if request.pty {
            let spawned = crate::pty::spawn(
                &argv,
                &cwd,
                &request.env,
                &self.secret_env,
                request.cols.unwrap_or(80),
                request.rows.unwrap_or(24),
            )?;
            return Ok(supervise_terminal(spawned, timeout));
        }
        let mut command = tokio::process::Command::new(&argv[0]);
        command
            .args(&argv[1..])
            .envs(&request.env)
            .env_remove(&self.secret_env)
            .env_remove(crate::process_policy::POLICY_ENV)
            .current_dir(cwd);
        command.stdin(if request.stdin { Stdio::piped() } else { Stdio::null() });
        command.stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
        let mut child = command.spawn()?;
        let process = Arc::new(Process::new(
            child.id().ok_or(ProcessError::NotFound)?,
            child.stdin.take().map(|s| Box::new(s) as Input),
            None,
        ));
        let stdout = tokio::spawn(process.clone().pump(child.stdout.take().unwrap(), "stdout"));
        let stderr = tokio::spawn(process.clone().pump(child.stderr.take().unwrap(), "stderr"));
        let own = process.clone();
        tokio::spawn(async move {
            let limit = async {
                match timeout {
                    Some(seconds) => tokio::time::sleep(Duration::from_secs(seconds.into())).await,
                    None => std::future::pending().await,
                }
            };
            let (status, timed_out) = tokio::select! {
                status = child.wait() => (status, false),
                () = limit => {
                    let _ = own.signal(libc::SIGKILL);
                    (child.wait().await, true)
                }
            };
            // Descendants cannot hold the command's pipes forever after its leader exits.
            let _ = own.signal(libc::SIGKILL);
            let _ = stdout.await;
            let _ = stderr.await;
            own.finish(status.ok().map(exit_code), timed_out).await;
        });
        Ok(process)
    }
}

impl Process {
    pub(crate) fn new(pid: u32, input: Option<Input>, terminal: Option<Terminal>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            pid,
            input: AsyncMutex::new(input),
            terminal,
            output: AsyncMutex::new(Output {
                replay: VecDeque::new(),
                bytes: 0,
                dropped: false,
                subscribers: Vec::new(),
            }),
            ended: watch::channel(None).0,
            started: Instant::now(),
        }
    }
    pub(crate) async fn terminal_output(self: Arc<Self>, reader: impl AsyncRead + Unpin) {
        self.pump(reader, "stdout").await;
    }
    pub(crate) async fn terminal_exit(&self, code: i32, timed_out: bool) {
        self.finish(Some(code), timed_out).await;
    }
}

pub fn argv(command: &Command) -> Result<Vec<String>, ProcessError> {
    match command {
        Command::Argv(argv) if !argv.is_empty() && !argv[0].is_empty() => Ok(argv.clone()),
        Command::Shell(script) => {
            if PathBuf::from("/bin/sh").is_file() {
                Ok(vec!["/bin/sh".into(), "-c".into(), script.clone()])
            } else {
                Ok(vec![std::env::current_exe()?.to_string_lossy().into(), "shell".into(), script.clone()])
            }
        }
        _ => Err(ProcessError::Invalid("empty command")),
    }
}

fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(libc::SIGKILL))
}

fn supervise_terminal(spawned: crate::pty::Spawned, timeout: Option<u32>) -> Arc<Process> {
    let crate::pty::Spawned { pid, terminal, input, output, mut child } = spawned;
    let process = Arc::new(Process::new(pid, Some(Box::new(input)), Some(terminal)));
    let drained = tokio::spawn(process.clone().terminal_output(output));
    let own = process.clone();
    let waited = tokio::task::spawn_blocking(move || child.wait());
    tokio::spawn(async move {
        let limit = async {
            match timeout {
                Some(seconds) => tokio::time::sleep(Duration::from_secs(seconds.into())).await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(waited);
        let (status, timed_out) = tokio::select! {
            status = &mut waited => (status, false),
            () = limit => {
                let _ = own.signal(libc::SIGKILL);
                (waited.await, true)
            }
        };
        let _ = own.signal(libc::SIGKILL);
        let _ = drained.await;
        let code = status.ok().and_then(Result::ok).map(|s| s.exit_code() as i32).unwrap_or(128 + libc::SIGKILL);
        own.terminal_exit(code, timed_out).await;
    });
    process
}
