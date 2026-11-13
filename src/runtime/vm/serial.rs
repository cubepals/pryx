// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2
//! Bounded host-side monitor diagnostics. Drain the monitor continuously with backpressure,
//! retain its last admitted bytes and finish capture before validating a shutdown proof.
//! This is byte capture, not a guest protocol parser or a replacement for application logs.
use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io,
    os::unix::{fs::OpenOptionsExt, net::UnixStream},
    path::Path,
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    task::JoinHandle,
};

pub(super) struct Capture {
    task: Option<JoinHandle<io::Result<bool>>>,
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
    reader.set_nonblocking(true)?;
    let mut reader = tokio::net::UnixStream::from_std(reader)?;
    let task = tokio::spawn(async move {
        let mut file = tokio::fs::File::from_std(file);
        let mut tail = VecDeque::new();
        let mut buffer = [0; 16 * 1024];
        let mut total = 0u64;
        let mut unpublished = 0usize;
        loop {
            let size = reader.read(&mut buffer).await?;
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
            if total <= limit as u64 {
                file.write_all(&buffer[..size]).await?;
            } else {
                unpublished = unpublished.saturating_add(size);
                if unpublished >= limit {
                    rewrite(&mut file, &tail).await?;
                    unpublished = 0;
                }
            }
        }
        rewrite(&mut file, &tail).await?;
        file.sync_data().await?;
        Ok(total > limit as u64)
    });
    Ok((writer, Capture { task: Some(task), result: None }))
}
async fn rewrite(file: &mut tokio::fs::File, bytes: &VecDeque<u8>) -> io::Result<()> {
    file.set_len(0).await?;
    file.seek(io::SeekFrom::Start(0)).await?;
    let (first, second) = bytes.as_slices();
    file.write_all(first).await?;
    file.write_all(second).await
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
