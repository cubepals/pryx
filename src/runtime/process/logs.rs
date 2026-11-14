// SPDX-License-Identifier: FSL-1.1-ALv2
//! Safe, bounded entrypoint log reads. Rotation is an explicit gap rather than silently
//! replaying old output; cancellation stops idle followers without another application write.
use super::journal::error;
use super::*;
use crate::runtime::logs::records;
use pryx_agent::{logbook::LOG_PATH, paths::DataDir};
use std::{
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
};

impl ProcessRuntime {
    pub(super) fn entrypoint_logs(
        &self,
        name: &str,
        options: LogOptions,
    ) -> BoxStream<'static, Result<LogChunk, RuntimeError>> {
        let own = self.clone();
        let name = name.to_owned();
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        tokio::spawn(async move {
            let result = async {
                let entry = own.entry(&name)?;
                let directory = DataDir::open(&entry.spec.data_dir).map_err(error)?;
                let mut position = 0;
                let mut inode = None;
                let mut pending = Vec::new();
                let mut first = true;
                loop {
                    match directory.read(LOG_PATH) {
                        Ok(mut file) => {
                            let meta = file.metadata().map_err(error)?;
                            let identity = (meta.dev(), meta.ino());
                            if inode.is_some_and(|previous| previous != identity) || meta.len() < position {
                                return Err(error("Entrypoint log rotated; output gap"));
                            }
                            inode = Some(identity);
                            file.seek(SeekFrom::Start(position)).map_err(error)?;
                            let mut bytes = Vec::new();
                            const MAX_BYTES: u64 = (pryx_agent::contract::CAPTURE_BYTES * 4) as u64;
                            file.take(MAX_BYTES + 1).read_to_end(&mut bytes).map_err(error)?;
                            if bytes.len() as u64 > MAX_BYTES {
                                return Err(error("Entrypoint log exceeded its bound"));
                            }
                            position += bytes.len() as u64;
                            pending.extend(bytes);
                            if pending.len() as u64 > MAX_BYTES {
                                return Err(error("Entrypoint log line exceeded its bound"));
                            }
                            if let Some(end) = pending.iter().rposition(|b| *b == b'\n') {
                                let read_options =
                                    LogOptions { tail: if first { options.tail } else { None }, ..options.clone() };
                                for chunk in records(&pending[..=end], &read_options)? {
                                    if tx.send(Ok(chunk)).await.is_err() {
                                        return Ok(());
                                    }
                                }
                                pending.drain(..=end);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(error(e)),
                    }
                    first = false;
                    if !options.follow || !own.running(&name)? {
                        return Ok(());
                    }
                    tokio::select! {
                        () = tx.closed() => return Ok(()),
                        () = tokio::time::sleep(Duration::from_millis(100)) => {},
                    }
                }
            }
            .await;
            if let Err(e) = result {
                let _ = tx.send(Err(e)).await;
            }
        });
        stream::poll_fn(move |cx| rx.poll_recv(cx)).boxed()
    }
}
