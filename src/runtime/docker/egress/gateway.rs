// SPDX-License-Identifier: FSL-1.1-ALv2
//! HTTP checks each request's host; TLS checks ClientHello SNI before connecting. Every
//! connection resolves once, refuses non-public results and dials that immutable resolution.
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
pub(super) fn allowed(host: &str, policy: &EgressPolicy) -> bool {
    fn matches(host: &str, rule: &str) -> bool {
        rule.strip_prefix("*.").map_or(host == rule, |suffix| host.ends_with(&format!(".{suffix}")))
    }
    !policy.deny.iter().any(|rule| matches(host, rule)) && policy.allow.iter().any(|rule| matches(host, rule))
}
async fn resolve(host: &str, port: u16, policy: &EgressPolicy) -> io::Result<Vec<SocketAddr>> {
    if !allowed(host, policy) || host.parse::<IpAddr>().is_ok() {
        return Err(denied());
    }
    let addresses: Vec<_> = tokio::net::lookup_host((host, port)).await?.collect();
    if addresses.is_empty()
        || addresses.iter().any(|address| !pryx_agent::transfer_target::public_address(address.ip()))
    {
        return Err(denied());
    }
    Ok(addresses)
}
pub(super) fn serve(
    listener: TcpListener,
    tls: bool,
    subnet: ipnet::IpNet,
    policy: EgressPolicy,
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
            let policy = policy.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let work = async {
                    if tls {
                        let _ = forward_tls(stream, &policy).await;
                    } else {
                        let app = Router::new().fallback(move |request: Request| {
                            let policy = policy.clone();
                            async move {
                                forward_http(request, &policy).await.unwrap_or_else(|_| {
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
async fn forward_http(request: Request, policy: &EgressPolicy) -> io::Result<Response> {
    let fields: Vec<_> = request.headers().get_all(hyper::header::HOST).iter().collect();
    if fields.len() != 1 {
        return Err(denied());
    }
    let authority = fields[0].to_str().map_err(|_| denied())?;
    let host = authority.strip_suffix(":80").unwrap_or(authority).to_ascii_lowercase();
    if host.is_empty() || host.contains([':', '/', '\\', '@']) {
        return Err(denied());
    }
    let addresses = tokio::time::timeout(CONNECT, resolve(&host, 80, policy)).await.map_err(|_| denied())??;
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
async fn forward_tls(mut downstream: TcpStream, policy: &EgressPolicy) -> io::Result<()> {
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
    let addresses = tokio::time::timeout(CONNECT, resolve(&host, 443, policy)).await.map_err(|_| denied())??;
    let mut upstream =
        tokio::time::timeout(CONNECT, TcpStream::connect(addresses.as_slice())).await.map_err(|_| denied())??;
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
        let policy = EgressPolicy {
            allow: vec!["*.example.com".into(), "example.org".into()],
            deny: vec!["private.example.com".into()],
        };
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
        let policy = EgressPolicy { allow: vec!["127.0.0.1".into()], deny: vec![] };
        assert_eq!(resolve("127.0.0.1", 80, &policy).await.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
}
