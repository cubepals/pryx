// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Bounded host-side monitor diagnostics. Drain the monitor continuously with backpressure,
//! retain its last admitted bytes and finish capture before validating a shutdown proof.
//! This is byte capture, not a guest protocol parser or a replacement for application logs.
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{self, Read, Seek, Write},
    os::unix::{fs::OpenOptionsExt, net::UnixStream},
    path::Path,
};
use tokio::sync::oneshot;

pub(super) struct Capture {
    task: Option<oneshot::Receiver<io::Result<bool>>>,
    result: Option<Result<bool, (io::ErrorKind, String)>>,
}
impl Capture {
    pub(super) async fn finish(&mut self) -> io::Result<bool> {
        if let Some(task) = self.task.as_mut() {
            let result = task.await.map_err(io::Error::other).and_then(|result| result);
            self.task.take();
            self.result = Some(result.map_err(|error| (error.kind(), error.to_string())));
        }
        match self.result.as_ref() {
            Some(Ok(truncated)) => Ok(*truncated),
            Some(Err((kind, text))) => Err(io::Error::new(*kind, text.clone())),
            None => Err(io::Error::other("Monitor capture has not completed")),
        }
    }
}

pub(super) fn start(path: &Path, limit: usize) -> io::Result<(UnixStream, Capture)> {
    if limit == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Monitor log budget must be positive"));
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let (reader, writer) = UnixStream::pair()?;
    let (finished, observation) = oneshot::channel();
    std::thread::Builder::new().name("pryx-vm-serial".into()).spawn(move || {
        // Keep draining even if its async owner is cancelled; the owned monitor must
        // retire without waiting for the executor or per-read filesystem I/O.
        let _ = finished.send(drain(reader, file, limit));
    })?;
    Ok((writer, Capture { task: Some(observation), result: None }))
}

fn drain(mut reader: UnixStream, mut file: File, limit: usize) -> io::Result<bool> {
    let mut tail = VecDeque::new();
    let mut buffer = [0; 16 * 1024];
    let mut total = 0u64;
    loop {
        let size = reader.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        total = total.saturating_add(size as u64);
        if size >= limit {
            tail.clear();
            tail.extend(&buffer[size - limit..size]);
        } else {
            let discard = tail.len().saturating_add(size).saturating_sub(limit);
            tail.drain(..discard);
            tail.extend(&buffer[..size]);
        }
    }
    file.set_len(0)?;
    file.seek(io::SeekFrom::Start(0))?;
    let (first, second) = tail.as_slices();
    file.write_all(first)?;
    file.write_all(second)?;
    file.sync_data()?;
    Ok(total > limit as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    #[tokio::test]
    async fn capture_bounds_real_socket_output_and_keeps_final_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("serial");
        let (writer, mut capture) = start(&path, 4096).unwrap();
        writer.set_nonblocking(true).unwrap();
        let mut writer = tokio::net::UnixStream::from_std(writer).unwrap();
        let mut bytes = (0..=u8::MAX).cycle().take(128 * 1024).collect::<Vec<_>>();
        bytes.extend_from_slice(b"final observation\n");
        writer.write_all(&bytes).await.unwrap();
        writer.shutdown().await.unwrap();
        drop(writer);
        assert!(capture.finish().await.unwrap());
        assert_eq!(std::fs::read(path).unwrap(), bytes[bytes.len() - 4096..]);
        assert!(capture.finish().await.unwrap());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn capture_drains_when_the_async_executor_cannot_progress() {
        use std::{io::Write as _, sync::mpsc, time::Duration};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("serial");
        let (mut writer, mut capture) = start(&path, 4096).unwrap();
        rustix::net::sockopt::set_socket_send_buffer_size(&writer, 4096).unwrap();
        let window = rustix::net::sockopt::socket_send_buffer_size(&writer).unwrap();
        let mut bytes = (0..=u8::MAX).cycle().take(window * 4).collect::<Vec<_>>();
        bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
        let expected = bytes[bytes.len() - 4096..].to_vec();
        let (finished, observation) = mpsc::channel();
        let producer = std::thread::spawn(move || {
            let result = writer.write_all(&bytes);
            drop(writer);
            let _ = finished.send(result);
        });
        // Deliberately hold this executor: draining serial cannot depend on its progress.
        observation.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        producer.join().unwrap();
        assert!(capture.finish().await.unwrap());
        assert_eq!(std::fs::read(path).unwrap(), expected);
    }
}
