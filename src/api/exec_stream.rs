// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! One-way exec attachment with bounded queues. Disconnecting a reader never cancels a command
//! or its containment timeout. Completed idempotency keys replay the bounded result.

use super::*;
use crate::protocol::{EXEC_CHUNK_BYTES, ErrorBody, ExecEvent, ExecResponse};
use crate::runtime::{LogChunk, LogStream};
use tokio::sync::mpsc;

const QUEUE: usize = 8;

pub(super) async fn exec_stream(
    State(app): State<AppState>,
    WorkloadPath(id): WorkloadPath,
    headers: HeaderMap,
    ApiJson(request): ApiJson<ExecRequest>,
) -> Result<Response, Response> {
    let key = exec_key(&headers)?;
    let epoch = epoch(&headers)?;
    request.validate().map_err(|e| NodeError::Invalid(e).into_response())?;
    let (output, receive) = mpsc::channel(QUEUE);
    let (wire, body) = mpsc::channel::<bytes::Bytes>(QUEUE);
    let manager = app.manager;
    let run = tokio::spawn(async move { manager.exec_live(id, request, key, epoch, Some(output)).await });
    tokio::spawn(forward(receive, run, wire));
    let stream = futures_util::stream::unfold(body, |mut body| async {
        body.recv().await.map(|bytes| (Ok::<_, std::convert::Infallible>(bytes), body))
    });
    Ok(([(header::CONTENT_TYPE, "application/x-ndjson")], Body::from_stream(stream)).into_response())
}

async fn send(wire: &mpsc::Sender<bytes::Bytes>, event: ExecEvent) {
    if wire.is_closed() {
        return;
    }
    let mut bytes = serde_json::to_vec(&event).expect("exec events serialize");
    bytes.push(b'\n');
    let _ = wire.send(bytes.into()).await;
}

/// Decode across Docker frame boundaries, retaining at most one incomplete UTF-8 character.
#[derive(Default)]
struct Text {
    pending: Vec<u8>,
    sent: String,
}

impl Text {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut out = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    out.push_str(text);
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    out.push_str(std::str::from_utf8(&self.pending[..valid]).expect("valid UTF-8 prefix"));
                    let Some(invalid) = error.error_len() else {
                        self.pending.drain(..valid);
                        break;
                    };
                    out.push('\u{fffd}');
                    self.pending.drain(..valid + invalid);
                }
            }
        }
        self.sent.push_str(&out);
        out
    }
    #[cfg(test)]
    fn finish(&mut self) -> String {
        let out = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        self.sent.push_str(&out);
        out
    }
}

async fn chunk(wire: &mpsc::Sender<bytes::Bytes>, stream: LogStream, data: String, truncated: bool) {
    if data.is_empty() && !truncated {
        return;
    }
    let event = match stream {
        LogStream::Stdout => ExecEvent::Stdout { data, truncated },
        LogStream::Stderr => ExecEvent::Stderr { data, truncated },
    };
    send(wire, event).await;
}

async fn remaining(wire: &mpsc::Sender<bytes::Bytes>, stream: LogStream, text: &mut Text, full: &str, truncated: bool) {
    text.pending.clear();
    // Also covers cached results, runtimes without attachment, and a timeout during backpressure.
    let mut rest = full.strip_prefix(&text.sent).unwrap_or("");
    while !rest.is_empty() {
        let mut end = rest.len().min(EXEC_CHUNK_BYTES);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        chunk(wire, stream, rest[..end].into(), false).await;
        rest = &rest[end..];
    }
    if truncated {
        chunk(wire, stream, String::new(), true).await;
    }
}

async fn forward(
    mut receive: mpsc::Receiver<LogChunk>,
    run: tokio::task::JoinHandle<Result<ExecResponse, NodeError>>,
    wire: mpsc::Sender<bytes::Bytes>,
) {
    let mut stdout = Text::default();
    let mut stderr = Text::default();
    while let Some(output) = receive.recv().await {
        let text = match output.stream {
            LogStream::Stdout => &mut stdout,
            LogStream::Stderr => &mut stderr,
        };
        chunk(&wire, output.stream, text.push(&output.bytes), false).await;
    }
    match run.await {
        Ok(Ok(result)) => {
            remaining(&wire, LogStream::Stdout, &mut stdout, &result.stdout, result.stdout_truncated).await;
            remaining(&wire, LogStream::Stderr, &mut stderr, &result.stderr, result.stderr_truncated).await;
            send(
                &wire,
                ExecEvent::Exit {
                    code: result.exit_code,
                    timed_out: result.timed_out,
                    killed: result.killed,
                    duration_ms: result.duration_ms,
                },
            )
            .await;
        }
        error => {
            let response = match error {
                Ok(Err(error)) => error.into_response(),
                _ => NodeError::Internal("exec task ended unexpectedly".into()).into_response(),
            };
            if let Ok(bytes) = axum::body::to_bytes(response.into_body(), 64 * 1024).await
                && let Ok(body) = serde_json::from_slice::<ErrorBody>(&bytes)
            {
                send(&wire, ExecEvent::Error { error: body.error }).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_boundaries_match_the_buffered_result() {
        for bytes in ["héllo 🌍".as_bytes(), b"a\xff\xc3\xa9\xe2"] {
            let mut text = Text::default();
            let mut out = String::new();
            for byte in bytes {
                out.push_str(&text.push(&[*byte]));
            }
            out.push_str(&text.finish());
            assert_eq!(out, String::from_utf8_lossy(bytes));
            assert_eq!(text.sent, out);
        }
    }
    #[tokio::test]
    async fn buffered_tail_completes_a_live_utf8_prefix() {
        let mut text = Text::default();
        assert_eq!(text.push(&[0xf0, 0x9f]), "");
        let (wire, mut receive) = mpsc::channel(QUEUE);
        remaining(&wire, LogStream::Stdout, &mut text, "🌍done", false).await;
        let event: serde_json::Value = serde_json::from_slice(&receive.recv().await.unwrap()).unwrap();
        assert_eq!(event["data"], "🌍done");
        assert!(receive.try_recv().is_err());
    }
}
