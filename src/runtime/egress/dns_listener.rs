// SPDX-License-Identifier: FSL-1.1-ALv2
//! Reserve both DNS transports together; serving and policy belong to the callers.
use std::{io, net::IpAddr};
use tokio::net::{TcpListener, UdpSocket};

pub(super) async fn resolver(address: IpAddr, port: u16) -> io::Result<(UdpSocket, TcpListener)> {
    const PORT_ATTEMPTS: usize = 8;
    for _ in 0..PORT_ATTEMPTS {
        let udp = UdpSocket::bind((address, port)).await?;
        match TcpListener::bind(udp.local_addr()?).await {
            Ok(tcp) => return Ok((udp, tcp)),
            Err(error) if port == 0 && error.kind() == io::ErrorKind::AddrInUse => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::AddrInUse, "No paired DNS listener port is available"))
}
