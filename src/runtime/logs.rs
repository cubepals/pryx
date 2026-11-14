// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Decode persistent agent entrypoint records into runtime logs, applying timestamp, tail and
//! since filters. Reading files, following output and selecting a backend belong to their owners.
use super::{LogChunk, LogOptions, LogStream, RuntimeError, format_time};
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use pryx_agent::logbook::Chunk;
use time::OffsetDateTime;
fn error(value: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Other(value.to_string())
}

pub(crate) fn records(data: &[u8], options: &LogOptions) -> Result<Vec<LogChunk>, RuntimeError> {
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
