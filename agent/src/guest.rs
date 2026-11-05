// SPDX-License-Identifier: FSL-1.1-ALv2
//! An embedded agent frontend can supervise virtual processes without sending virtual PIDs to
//! the host kernel. Native agents keep the existing process and TCP implementations.

use async_trait::async_trait;
use tokio::net::TcpStream;

#[async_trait]
pub trait Forward: Send + Sync {
    async fn serve(&self, stream: TcpStream, port: u16, header: Vec<u8>) -> std::io::Result<()>;
}
