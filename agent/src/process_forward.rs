// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Process-tier forwarding uses a trusted metadata/connection worker created before Landlock.
//! Its only request is a target port. It verifies the supervisor's process tree and actual TCP
//! peer, then hands the connected descriptor to the restricted HTTP worker. This prevents the
//! agent API from proxying a neighbor's service; it does not create a private network namespace.

use crate::{guest::Forward, ports::FORWARD_CONNECT_TIMEOUT, process_peer::OwnedPeer};
use std::{io, net::TcpStream as StdStream, sync::Arc};
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    sync::{mpsc, oneshot},
};

type Request = (u16, oneshot::Sender<io::Result<StdStream>>);
pub struct ProcessForward {
    requests: mpsc::Sender<Request>,
}
impl ProcessForward {
    async fn connect(&self, port: u16) -> io::Result<TcpStream> {
        let (reply, result) = oneshot::channel();
        let stream = tokio::time::timeout(FORWARD_CONNECT_TIMEOUT * 2, async {
            self.requests.send((port, reply)).await.map_err(|_| io::ErrorKind::BrokenPipe)?;
            result.await.map_err(|_| io::ErrorKind::BrokenPipe)?
        })
        .await
        .map_err(|_| io::ErrorKind::TimedOut)??;
        stream.set_nonblocking(true)?;
        TcpStream::from_std(stream)
    }

    pub fn start() -> io::Result<Arc<dyn Forward>> {
        let owner = OwnedPeer::new(std::process::id())?;
        let (requests, mut receive) = mpsc::channel::<Request>(128);
        std::thread::Builder::new().name("forward-owner".into()).spawn(move || {
            while let Some((port, reply)) = receive.blocking_recv() {
                let _ = reply.send(owner.connect(port));
            }
        })?;
        Ok(Arc::new(Self { requests }))
    }
}
#[async_trait::async_trait]
impl Forward for ProcessForward {
    async fn probe(&self, port: u16, path: Option<&str>) -> io::Result<Option<u16>> {
        if port == crate::tools::defaults::DESKTOP_PORT
            && let Some(desktop) = crate::process_tools::configured().and_then(|tools| tools.active_desktop())
        {
            return crate::field_process::probe_stream(desktop.connect().await?, path).await;
        }
        if port == crate::contract::BROWSER_PORT
            && let Some(browser) = crate::process_tools::configured().and_then(|tools| tools.active_browser())
        {
            return crate::field_process::probe_stream(browser.connect().await?, path).await;
        }
        crate::field_process::probe_stream(self.connect(port).await?, path).await
    }
    async fn serve(&self, mut downstream: TcpStream, port: u16, header: Vec<u8>) -> io::Result<()> {
        if port == crate::tools::defaults::DESKTOP_PORT
            && let Some(desktop) = crate::process_tools::configured().and_then(|tools| tools.active_desktop())
        {
            let mut upstream = desktop.connect().await?;
            upstream.write_all(&header).await?;
            tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await?;
            return Ok(());
        }
        if port == crate::contract::BROWSER_PORT
            && let Some(browser) = crate::process_tools::configured().and_then(|tools| tools.active_browser())
        {
            let mut upstream = browser.connect().await?;
            upstream.write_all(&header).await?;
            tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await?;
            return Ok(());
        }
        let mut upstream = self.connect(port).await?;
        upstream.write_all(&header).await?;
        tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await?;
        Ok(())
    }
}
