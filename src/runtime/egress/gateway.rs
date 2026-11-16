// SPDX-License-Identifier: FSL-1.1-ALv2
//! HTTP checks each request's host; TLS checks ClientHello SNI before connecting. Every
//! connection resolves once, refuses the name if any result is non-public, one of this host's
//! own addresses as it has them now, or one the sandbox's own rules refuse, and dials that
//! immutable resolution.
use super::*;
use axum::{Router, body::Body, extract::Request, http::StatusCode, response::Response};
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use std::{io, net::SocketAddr};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
const HEADER_BYTES: usize = 16 * 1024;
const CONNECT: Duration = Duration::from_secs(5);
fn denied() -> io::Error {
    io::ErrorKind::PermissionDenied.into()
}
pub(super) fn allowed(host: &str, policy: &Policy) -> bool {
    let Policy::Names(policy) = policy else { return true };
    fn matches(host: &str, rule: &str) -> bool {
        rule.strip_prefix("*.").map_or(host == rule, |suffix| host.ends_with(&format!(".{suffix}")))
    }
    !policy.deny.iter().any(|rule| matches(host, rule)) && policy.allow.iter().any(|rule| matches(host, rule))
}
async fn resolve(host: &str, port: u16, policy: &Policy, refused: &Refused) -> io::Result<Vec<SocketAddr>> {
    if !allowed(host, policy) || host.parse::<IpAddr>().is_ok() {
        return Err(denied());
    }
    let addresses: Vec<_> = tokio::net::lookup_host((host, port)).await?.collect();
    if addresses.is_empty() || addresses.iter().any(|address| !dialable(*address, refused)) {
        return Err(denied());
    }
    Ok(addresses)
}
/// An upstream a gateway may dial: a public address, not local to this host as the kernel routes
/// it now (a name resolved to the host's own address would reach its services from the daemon),
/// and none the sandbox's own rules refuse.
fn dialable(upstream: SocketAddr, refused: &Refused) -> bool {
    let address = upstream.ip().to_canonical();
    pryx_agent::transfer_target::public_address(address)
        && matches!(local(SocketAddr::new(address, upstream.port())), Ok(false))
        && !refused.names(upstream)
}
/// Whether the kernel routes `address` to this host itself, asked as a dial is about to be made:
/// a datagram socket connected to it (which sends nothing) gets the source the kernel chose, and
/// a local destination is its own source (the local route's preferred source; RFC 6724's first
/// rule for IPv6). An address with no route is an error, which refuses it.
fn local(upstream: SocketAddr) -> io::Result<bool> {
    let unspecified: IpAddr = match upstream {
        SocketAddr::V4(_) => std::net::Ipv4Addr::UNSPECIFIED.into(),
        SocketAddr::V6(_) => std::net::Ipv6Addr::UNSPECIFIED.into(),
    };
    let probe = std::net::UdpSocket::bind((unspecified, 0))?;
    probe.connect(upstream)?;
    Ok(probe.local_addr()?.ip() == upstream.ip())
}
/// The same question of a connection just made, before a byte of the sandbox's goes on it.
fn looped(stream: &TcpStream) -> io::Result<bool> {
    Ok(stream.local_addr()?.ip().to_canonical() == stream.peer_addr()?.ip().to_canonical())
}
pub(in crate::runtime) fn serve(
    listener: TcpListener,
    tls: bool,
    subnet: ipnet::IpNet,
    policy: Policy,
    refused: Refused,
    capacity: Arc<Semaphore>,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {()=cancel.cancelled()=>break,result=listener.accept()=>result};
            let Ok((stream, peer)) = accepted else { break };
            if !subnet.contains(&peer.ip()) {
                continue;
            }
            let Ok(permit) = capacity.clone().try_acquire_owned() else { continue };
            let (policy, refused) = (policy.clone(), refused.clone());
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let work = async {
                    if tls {
                        let _ = forward_tls(stream, &policy, &refused).await;
                    } else {
                        let app = Router::new().fallback(move |request: Request| {
                            let (policy, refused) = (policy.clone(), refused.clone());
                            async move {
                                forward_http(request, &policy, &refused).await.unwrap_or_else(|_| {
                                    Response::builder()
                                        .status(StatusCode::FORBIDDEN)
                                        .body(Body::from("Egress hostname refused"))
                                        .expect("static response")
                                })
                            }
                        });
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), TowerToHyperService::new(app))
                            .await;
                    }
                };
                tokio::select! {()=cancel.cancelled()=>{},_=tokio::time::timeout(Duration::from_secs(600),work)=>{}}
            });
        }
    });
}
async fn forward_http(request: Request, policy: &Policy, refused: &Refused) -> io::Result<Response> {
    let fields: Vec<_> = request.headers().get_all(hyper::header::HOST).iter().collect();
    if fields.len() != 1 {
        return Err(denied());
    }
    let authority = fields[0].to_str().map_err(|_| denied())?;
    let host = authority.strip_suffix(":80").unwrap_or(authority).to_ascii_lowercase();
    if host.is_empty() || host.contains([':', '/', '\\', '@']) {
        return Err(denied());
    }
    let addresses =
        tokio::time::timeout(CONNECT, resolve(&host, 80, policy, refused)).await.map_err(|_| denied())??;
    let path = request.uri().path_and_query().map_or("/", |p| p.as_str()).to_owned();
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT)
        .timeout(Duration::from_secs(600))
        .resolve_to_addrs(&host, &addresses)
        .build()
        .map_err(io::Error::other)?;
    let (parts, body) = request.into_parts();
    let mut headers = parts.headers;
    for name in [
        "connection",
        "proxy-authorization",
        "proxy-connection",
        "transfer-encoding",
        "upgrade",
        "keep-alive",
        "te",
        "trailer",
    ] {
        headers.remove(name);
    }
    let upstream = client
        .request(parts.method, format!("http://{host}{path}"))
        .headers(headers)
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await
        .map_err(io::Error::other)?;
    let mut response = Response::builder().status(upstream.status());
    for (name, value) in upstream.headers() {
        if !matches!(name.as_str(), "connection" | "transfer-encoding" | "upgrade" | "keep-alive" | "trailer") {
            response = response.header(name, value);
        }
    }
    response.body(Body::from_stream(upstream.bytes_stream())).map_err(io::Error::other)
}
async fn forward_tls(mut downstream: TcpStream, policy: &Policy, refused: &Refused) -> io::Result<()> {
    let mut prefix = Vec::new();
    let host = tokio::time::timeout(CONNECT, async {
        loop {
            if prefix.len() >= 5 {
                let length = u16::from_be_bytes([prefix[3], prefix[4]]) as usize + 5;
                if length > HEADER_BYTES {
                    return Err(denied());
                }
                if prefix.len() >= length {
                    return server_name(&prefix[..length]);
                }
            }
            let mut bytes = [0; 2048];
            let n = downstream.read(&mut bytes).await?;
            if n == 0 {
                return Err(denied());
            }
            prefix.extend_from_slice(&bytes[..n]);
            if prefix.len() > HEADER_BYTES {
                return Err(denied());
            }
        }
    })
    .await
    .map_err(|_| denied())??;
    let addresses =
        tokio::time::timeout(CONNECT, resolve(&host, 443, policy, refused)).await.map_err(|_| denied())??;
    let mut upstream =
        tokio::time::timeout(CONNECT, TcpStream::connect(addresses.as_slice())).await.map_err(|_| denied())??;
    if looped(&upstream)? {
        return Err(denied());
    }
    upstream.write_all(&prefix).await?;
    tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await?;
    Ok(())
}
fn server_name(bytes: &[u8]) -> io::Result<String> {
    fn take<'a>(bytes: &mut &'a [u8], n: usize) -> io::Result<&'a [u8]> {
        if bytes.len() < n {
            return Err(denied());
        }
        let (a, b) = bytes.split_at(n);
        *bytes = b;
        Ok(a)
    }
    fn u16(bytes: &mut &[u8]) -> io::Result<usize> {
        let value = take(bytes, 2)?;
        Ok(usize::from(u16::from_be_bytes([value[0], value[1]])))
    }
    if bytes.len() < 9 || bytes[0] != 22 || bytes[5] != 1 {
        return Err(denied());
    }
    let length = ((bytes[6] as usize) << 16) | ((bytes[7] as usize) << 8) | bytes[8] as usize;
    let mut hello = bytes.get(9..9 + length).ok_or_else(denied)?;
    take(&mut hello, 34)?;
    let session = take(&mut hello, 1)?[0] as usize;
    take(&mut hello, session)?;
    let ciphers = u16(&mut hello)?;
    take(&mut hello, ciphers)?;
    let compression = take(&mut hello, 1)?[0] as usize;
    take(&mut hello, compression)?;
    let extensions = u16(&mut hello)?;
    let mut extensions = take(&mut hello, extensions)?;
    let mut name = None;
    while !extensions.is_empty() {
        let kind = u16(&mut extensions)?;
        let length = u16(&mut extensions)?;
        let mut extension = take(&mut extensions, length)?;
        if kind == 0 {
            if name.is_some() {
                return Err(denied());
            }
            let length = u16(&mut extension)?;
            let mut names = take(&mut extension, length)?;
            while !names.is_empty() {
                let kind = take(&mut names, 1)?[0];
                let length = u16(&mut names)?;
                let value = take(&mut names, length)?;
                if kind == 0 {
                    if name.is_some() {
                        return Err(denied());
                    }
                    name = Some(std::str::from_utf8(value).map_err(|_| denied())?.to_ascii_lowercase());
                }
            }
        }
    }
    let name = name.ok_or_else(denied)?;
    if name.is_empty()
        || name.split('.').any(|p| {
            p.is_empty()
                || p.starts_with('-')
                || p.ends_with('-')
                || !p.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        })
    {
        return Err(denied());
    }
    Ok(name)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deny_overrides_allow_and_wildcards_are_component_bounded() {
        let policy = Policy::Names(EgressPolicy {
            allow: vec!["*.example.com".into(), "example.org".into()],
            deny: vec!["private.example.com".into()],
        });
        assert!(allowed("www.example.com", &policy));
        assert!(!allowed("example.com", &policy));
        assert!(!allowed("otherexample.com", &policy));
        assert!(!allowed("private.example.com", &policy));
        assert!(allowed("example.org", &policy));
    }
    #[test]
    fn malformed_tls_is_refused_without_a_panic() {
        for length in 0..2048 {
            assert!(server_name(&vec![0; length]).is_err());
        }
    }
}

#[cfg(test)]
mod hello_tests {
    use super::*;
    #[test]
    fn reads_the_sni_of_a_real_rustls_client_hello() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        let mut client = rustls::ClientConnection::new(
            Arc::new(config),
            rustls::pki_types::ServerName::try_from("example.com").unwrap(),
        )
        .unwrap();
        let mut bytes = Vec::new();
        client.write_tls(&mut bytes).unwrap();
        assert_eq!(server_name(&bytes).unwrap(), "example.com");
        for at in 0..bytes.len() {
            assert!(server_name(&bytes[..at]).is_err());
        }
    }
    #[tokio::test]
    async fn raw_ip_policy_entries_cannot_bypass_hostname_enforcement() {
        let policy = Policy::Names(EgressPolicy { allow: vec!["127.0.0.1".into()], deny: vec![] });
        assert_eq!(
            resolve("127.0.0.1", 80, &policy, &Refused::default()).await.unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    #[test]
    fn this_hosts_own_and_refused_upstreams_are_never_dialled() {
        // Public addresses outside documentation ranges, so only the rule under test decides.
        let (public, public6): (IpAddr, IpAddr) = ("93.184.215.14".parse().unwrap(), "2606:2800::1".parse().unwrap());
        // The kernel's answer for this host's own addresses, loopback and the rest (a link-local
        // one needs its interface named, which no upstream has).
        let at = |address: IpAddr| SocketAddr::new(address, 443);
        assert!(local(at("127.0.0.1".parse().unwrap())).unwrap());
        for own in crate::bpf::host::addresses() {
            if !matches!(own, IpAddr::V6(own) if own.is_unicast_link_local()) {
                assert!(local(at(own)).unwrap(), "{own} is this host's");
            }
        }
        assert!(!matches!(local(at(public)), Ok(true)));
        let none = Refused::default();
        let mapped = IpAddr::V6(std::net::Ipv4Addr::new(93, 184, 215, 14).to_ipv6_mapped());
        // A sandbox's own deny holds on its ports, for IPv4 and its mapped spelling alike.
        let refused = Refused::new(&[
            Destination { prefix: "93.184.215.0/24".parse().unwrap(), ports: Some((443, 443)) },
            Destination { prefix: "2606:2800::/32".parse().unwrap(), ports: None },
        ]);
        assert!(refused.names(SocketAddr::new(public, 443)));
        assert!(refused.names(SocketAddr::new(mapped, 443)));
        assert!(!refused.names(SocketAddr::new(public, 80)));
        assert!(refused.names(SocketAddr::new(public6, 80)));
        assert!(!none.names(SocketAddr::new(public, 443)));
        for refused_upstream in
            [SocketAddr::new(public, 443), SocketAddr::new(mapped, 443), SocketAddr::new(public6, 80)]
        {
            assert!(!dialable(refused_upstream, &refused), "{refused_upstream}");
        }
        // Non-public addresses stay refused whatever else holds.
        assert!(!dialable(SocketAddr::new("10.0.0.1".parse().unwrap(), 443), &none));
        assert!(!dialable(SocketAddr::new("127.0.0.1".parse().unwrap(), 443), &none));
    }
    #[tokio::test]
    async fn a_connection_to_this_host_is_seen_as_one() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        assert!(looped(&stream).unwrap());
    }
}
