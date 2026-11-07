// SPDX-License-Identifier: FSL-1.1-ALv2
//! Safe, bounded entrypoint log reads. Rotation is an explicit gap rather than silently
//! replaying old output; cancellation stops idle followers without another application write.
use super::journal::error;
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use pryx_agent::{
    logbook::{Chunk, LOG_PATH},
    paths::DataDir,
};
use std::{
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
};

fn records(data: &[u8], options: &LogOptions) -> Result<Vec<LogChunk>, RuntimeError> {
    let mut found = Vec::new();
    for line in data.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let chunk: Chunk = serde_json::from_slice(line).map_err(error)?;
        if options.since_unix.is_some_and(|since| chunk.at / 1000 < since.max(0) as u64) {
            continue;
        }
        let stream = match chunk.stream.as_str() {
            "stdout" => LogStream::Stdout,
            "stderr" => LogStream::Stderr,
            _ => return Err(error("Invalid entrypoint log stream")),
        };
        let decoded = STANDARD.decode(chunk.bytes).map_err(error)?;
        let stamp =
            format_time(OffsetDateTime::from_unix_timestamp_nanos(i128::from(chunk.at) * 1_000_000).map_err(error)?);
        for line in String::from_utf8_lossy(&decoded).split_inclusive('\n') {
            found.push(LogChunk { stream, bytes: Bytes::from(format!("{stamp} {line}")) });
        }
    }
    if let Some(tail) = options.tail {
        found.drain(..found.len().saturating_sub(tail as usize));
    }
    Ok(found)
}
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logs_decode_timestamps_tail_and_since() {
        let rows = b"{\"at\":1000,\"stream\":\"stdout\",\"bytes\":\"b25lXG4=\"}\n{\"at\":2000,\"stream\":\"stderr\",\"bytes\":\"dHdvCg==\"}\n";
        let found = records(rows, &LogOptions { tail: Some(1), ..Default::default() }).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].stream, LogStream::Stderr);
        assert!(found[0].bytes.ends_with(b" two\n"));
        assert!(records(rows, &LogOptions { since_unix: Some(3), ..Default::default() }).unwrap().is_empty());
    }
}
