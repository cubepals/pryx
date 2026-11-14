// SPDX-License-Identifier: FSL-1.1-ALv2
//! Own a complete private listener set before publishing hostname egress readiness.
//! Docker supplies fixed redirected ports; user-mode VM networking requests ephemeral ones.
//! Cancellation retains the existing gateways' bounded connection and resolver behavior.
use super::{dns, gateway, *};
use std::{io, net::IpAddr};
use tokio::net::UdpSocket;
use tokio_util::sync::DropGuard;

pub(in crate::runtime) struct Ports {
    pub http: u16,
    pub tls: u16,
    pub dns: u16,
}

async fn resolver(address: IpAddr, port: u16) -> io::Result<(UdpSocket, TcpListener)> {
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

pub(in crate::runtime) async fn listen(
    address: IpAddr,
    ports: Ports,
    subnet: ipnet::IpNet,
    policy: Policy,
    capacity: usize,
) -> io::Result<(Ports, DropGuard)> {
    let private = match address {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
    };
    if !private || capacity == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Hostname gateways require private listeners and capacity",
        ));
    }
    let http = TcpListener::bind((address, ports.http)).await?;
    let tls = TcpListener::bind((address, ports.tls)).await?;
    let (dns_udp, dns_tcp) = resolver(address, ports.dns).await?;
    let ports =
        Ports { http: http.local_addr()?.port(), tls: tls.local_addr()?.port(), dns: dns_tcp.local_addr()?.port() };
    let cancel = CancellationToken::new();
    let guard = cancel.clone().drop_guard();
    let connections = Arc::new(Semaphore::new(capacity));
    gateway::serve(http, false, subnet, policy.clone(), connections.clone(), cancel.clone());
    gateway::serve(tls, true, subnet, policy.clone(), connections, cancel.clone());
    dns::serve(dns_udp, dns_tcp, subnet, policy, Arc::new(Semaphore::new(capacity)), cancel);
    Ok((ports, guard))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn bind_tcp(address: Ipv4Addr, port: u16) -> io::Result<tokio::net::TcpListener> {
        let socket = tokio::net::TcpSocket::new_v4()?;
        socket.set_reuseaddr(true)?;
        socket.bind((address, port).into())?;
        socket.listen(1)
    }

    #[tokio::test]
    async fn ephemeral_listeners_enforce_policy_and_retire_together() {
        let address = Ipv4Addr::LOCALHOST;
        let policy = Policy::Names(EgressPolicy { allow: vec![], deny: vec![] });
        let (ports, lifetime) = listen(
            address.into(),
            Ports { http: 0, tls: 0, dns: 0 },
            ipnet::IpNet::new(address.into(), u32::BITS as u8).unwrap(),
            policy.clone(),
            2,
        )
        .await
        .unwrap();
        assert!(ports.http != 0 && ports.tls != 0 && ports.dns != 0);
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}:{}/", ports.http))
            .header(hyper::header::HOST, "denied.pryx.invalid")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), hyper::StatusCode::FORBIDDEN);
        // This fixture's own question; the real resolver owns the expected refusal bytes.
        let mut query = vec![0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in ["denied", "pryx", "invalid"] {
            query.push(label.len() as u8);
            query.extend_from_slice(label.as_bytes());
        }
        query.extend_from_slice(&[0, 0, 1, 0, 1]);
        let expected = dns::answer(&query, &policy, false).await.unwrap();
        let udp = UdpSocket::bind((address, 0)).await.unwrap();
        udp.send_to(&query, (address, ports.dns)).await.unwrap();
        let mut bytes = [0; 512];
        let (count, _) =
            tokio::time::timeout(Duration::from_secs(2), udp.recv_from(&mut bytes)).await.unwrap().unwrap();
        assert_eq!(&bytes[..count], expected);
        let mut tcp = tokio::net::TcpStream::connect((address, ports.dns)).await.unwrap();
        tcp.write_u16(query.len() as u16).await.unwrap();
        tcp.write_all(&query).await.unwrap();
        let count = tokio::time::timeout(Duration::from_secs(2), tcp.read_u16()).await.unwrap().unwrap();
        let mut bytes = vec![0; usize::from(count)];
        tcp.read_exact(&mut bytes).await.unwrap();
        assert_eq!(bytes, expected);
        drop(tcp);
        drop(response);
        drop(lifetime);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let tcp = [ports.http, ports.tls, ports.dns].map(|port| bind_tcp(address, port));
            if tcp.iter().all(Result::is_ok) && std::net::UdpSocket::bind((address, ports.dns)).is_ok() {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "Cancelled hostname gateways retained listeners");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn public_listeners_and_zero_capacity_are_refused() {
        for (address, capacity) in
            [(Ipv4Addr::UNSPECIFIED, 1), (Ipv4Addr::new(1, 1, 1, 1), 1), (Ipv4Addr::LOCALHOST, 0)]
        {
            let result = listen(
                address.into(),
                Ports { http: 0, tls: 0, dns: 0 },
                ipnet::IpNet::new(Ipv4Addr::LOCALHOST.into(), u32::BITS as u8).unwrap(),
                Policy::Names(EgressPolicy { allow: vec![], deny: vec![] }),
                capacity,
            )
            .await;
            assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::InvalidInput));
        }
    }
}
