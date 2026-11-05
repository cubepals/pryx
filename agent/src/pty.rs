// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! PTY ownership and blocking platform I/O. Process lifetime and output policy live in process.rs;
//! websocket connections attach to the same process rather than owning or restarting its shell.

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Mutex;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio_util::io::SyncIoBridge;

pub struct Terminal(Mutex<Box<dyn MasterPty + Send>>);

impl Terminal {
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        if cols == 0 || rows == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "terminal dimensions"));
        }
        self.0.lock().unwrap().resize(size(cols, rows)).map_err(failed)
    }
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { cols, rows, pixel_width: 0, pixel_height: 0 }
}

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

pub(crate) struct Spawned {
    pub pid: u32,
    pub terminal: Terminal,
    pub input: tokio::io::DuplexStream,
    pub output: tokio::io::DuplexStream,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
}

pub(crate) fn spawn(
    argv: &[String],
    cwd: &Path,
    env: &std::collections::BTreeMap<String, String>,
    secret_env: &str,
    cols: u16,
    rows: u16,
) -> io::Result<Spawned> {
    if cols == 0 || rows == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "terminal dimensions"));
    }
    let pair = native_pty_system().openpty(size(cols, rows)).map_err(failed)?;
    let mut command = CommandBuilder::new(&argv[0]);
    command.args(&argv[1..]);
    command.cwd(cwd);
    for (name, value) in env {
        command.env(name, value);
    }
    command.env_remove(secret_env);
    let child = pair.slave.spawn_command(command).map_err(failed)?;
    let pid = child.process_id().ok_or(io::Error::other("Terminal has no process id"))?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(failed)?;
    let mut writer = pair.master.take_writer().map_err(failed)?;
    let (input, input_reader) = tokio::io::duplex(64 * 1024);
    let (output_writer, output) = tokio::io::duplex(64 * 1024);
    tokio::task::spawn_blocking(move || {
        let mut source = SyncIoBridge::new(input_reader);
        let mut buffer = [0; 8192];
        while let Ok(n) = source.read(&mut buffer) {
            if n == 0 || writer.write_all(&buffer[..n]).is_err() {
                break;
            }
        }
    });
    tokio::task::spawn_blocking(move || {
        let mut sink = SyncIoBridge::new(output_writer);
        let mut buffer = [0; 8192];
        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 || sink.write_all(&buffer[..n]).is_err() {
                break;
            }
        }
    });
    Ok(Spawned { pid, terminal: Terminal(Mutex::new(pair.master)), input, output, child })
}
