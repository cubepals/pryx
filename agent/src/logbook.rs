// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Bounded entrypoint logs persist across agent restart. Only application output enters this
//! store. Signing material, process requests and environments do not. Slow readers get a gap.

use crate::{contract::CAPTURE_BYTES, paths::DataDir};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
    sync::Mutex,
};
use tokio::sync::broadcast;
pub const LOG_PATH: &str = ".pryx-agent/entrypoint-log.ndjson";
const DISK_BYTES: u64 = (CAPTURE_BYTES * 4) as u64;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Chunk {
    pub at: u64,
    pub stream: String,
    pub bytes: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Frame {
    Replay { chunks: Vec<Chunk> },
    Chunk { chunk: Chunk },
    Gap,
}

struct State {
    chunks: VecDeque<(Chunk, usize)>,
    bytes: usize,
    file: Option<File>,
    disk_bytes: u64,
    failed: bool,
}

pub struct Logbook {
    directory: DataDir,
    state: Mutex<State>,
    sender: broadcast::Sender<Frame>,
}

impl Logbook {
    pub fn open(path: &Path) -> io::Result<Self> {
        let directory = DataDir::open(path)?;
        let mut state = State { chunks: VecDeque::new(), bytes: 0, file: None, disk_bytes: 0, failed: false };
        match directory.read(LOG_PATH) {
            Ok(mut file) => {
                let length = file.metadata()?.len();
                if length > DISK_BYTES {
                    file.seek(SeekFrom::End(-(DISK_BYTES as i64)))?;
                }
                let mut bytes = Vec::new();
                file.take(DISK_BYTES).read_to_end(&mut bytes)?;
                for line in bytes.split(|b| *b == b'\n') {
                    if let Ok(chunk) = serde_json::from_slice::<Chunk>(line)
                        && matches!(chunk.stream.as_str(), "stdout" | "stderr")
                        && chunk.bytes.len() <= CAPTURE_BYTES
                    {
                        state.push(chunk, line.len() + 1);
                    }
                }
                state.disk_bytes = length;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(Self { directory, state: Mutex::new(state), sender: broadcast::channel(32).0 })
    }

    pub fn record(&self, stream: &str, bytes: &str) -> io::Result<()> {
        let chunk = Chunk {
            at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()
                as u64,
            stream: stream.into(),
            bytes: bytes.into(),
        };
        let line = line_bytes(&chunk)?;
        let mut state = self.state.lock().unwrap();
        state.push(chunk.clone(), line.len());
        let persisted = (|| {
            if state.disk_bytes + line.len() as u64 > DISK_BYTES {
                let mut history = Vec::new();
                for (chunk, _) in &state.chunks {
                    history.extend(line_bytes(chunk)?);
                }
                self.directory.write_atomic(LOG_PATH, &history, false)?;
                state.file = Some(self.directory.append(LOG_PATH)?);
                state.disk_bytes = history.len() as u64;
            } else {
                if state.file.is_none() {
                    state.file = Some(self.directory.append(LOG_PATH)?);
                }
                state.file.as_mut().expect("opened log").write_all(&line)?;
                state.disk_bytes += line.len() as u64;
            }
            Ok(())
        })();
        state.failed = persisted.is_err();
        let _ = self.sender.send(Frame::Chunk { chunk });
        if state.failed {
            let _ = self.sender.send(Frame::Gap);
        }
        persisted
    }

    pub fn subscribe(&self) -> (Vec<Frame>, broadcast::Receiver<Frame>) {
        let state = self.state.lock().unwrap();
        let mut frames = vec![Frame::Replay { chunks: state.chunks.iter().map(|(chunk, _)| chunk.clone()).collect() }];
        if state.failed {
            frames.push(Frame::Gap);
        }
        (frames, self.sender.subscribe())
    }
}

impl State {
    fn push(&mut self, chunk: Chunk, bytes: usize) {
        self.chunks.push_back((chunk, bytes));
        self.bytes += bytes;
        while self.bytes > CAPTURE_BYTES {
            if let Some((_, bytes)) = self.chunks.pop_front() {
                self.bytes -= bytes;
            }
        }
    }
}
fn line_bytes(chunk: &Chunk) -> io::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(chunk).map_err(io::Error::other)?;
    line.push(b'\n');
    Ok(line)
}
