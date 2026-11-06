// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Untrusted transfer destinations resolve once to public unicast addresses. The connection
//! resolver serves only that immutable snapshot; redirects and proxy environment overrides
//! cannot select another destination. Normal TLS certificate and hostname verification remain.

use crate::context::Failure;
use axum::http::StatusCode;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
// IANA IPv4 Special-Purpose Address Registry: non-public protocol, shared, documentation,
// benchmarking and reserved networks, beyond the classes exported by Ipv4Addr's predicates.
const SPECIAL_V4: &[(Ipv4Addr, u32)] = &[
    (Ipv4Addr::new(0, 0, 0, 0), 8),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
    (Ipv4Addr::new(192, 0, 0, 0), 24),
    (Ipv4Addr::new(192, 0, 2, 0), 24),
    (Ipv4Addr::new(198, 18, 0, 0), 15),
    (Ipv4Addr::new(198, 51, 100, 0), 24),
    (Ipv4Addr::new(203, 0, 113, 0), 24),
    (Ipv4Addr::new(240, 0, 0, 0), 4),
];
// IANA IPv6 Global Unicast Assignments currently allocate 2000::/3. Exclude its special-purpose
// protocol block, documentation and deprecated 6to4 (which can encode a forbidden IPv4 target).
const GLOBAL_V6: (u128, u32) = (0x2000_u128 << 112, 3);
const SPECIAL_V6: &[(u128, u32)] = &[
    (0x2001_u128 << 112, 23),
    ((0x2001_u128 << 112) | (0xdb8_u128 << 96), 32),
    (0x2002_u128 << 112, 16),
    (0x3fff_u128 << 112, 20),
];

fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_private()
                && !ip.is_multicast()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
                && !SPECIAL_V4
                    .iter()
                    .any(|(network, bits)| u32::from(ip) >> (32 - bits) == u32::from(*network) >> (32 - bits))
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4() {
                return public(IpAddr::V4(ip));
            }
            let address = u128::from(ip);
            !ip.is_loopback()
                && !ip.is_unicast_link_local()
                && !ip.is_unique_local()
                && !ip.is_multicast()
                && !ip.is_unspecified()
                && address >> (128 - GLOBAL_V6.1) == GLOBAL_V6.0 >> (128 - GLOBAL_V6.1)
                && !SPECIAL_V6.iter().any(|(network, bits)| address >> (128 - bits) == network >> (128 - bits))
        }
    }
}

fn invalid() -> Failure {
    Failure(
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_request",
        "Transfer requires an HTTP URL with only public destination addresses".into(),
    )
}
fn unavailable() -> Failure {
    Failure(StatusCode::BAD_GATEWAY, "transfer_failed", "Remote file transfer failed".into())
}

struct Pinned {
    host: String,
    addresses: Vec<SocketAddr>,
}
impl Pinned {
    fn new(host: String, addresses: Vec<SocketAddr>) -> Result<Self, Failure> {
        if addresses.is_empty() || addresses.iter().any(|a| !public(a.ip())) {
            return Err(invalid());
        }
        Ok(Self { host, addresses })
    }
}
impl Resolve for Pinned {
    fn resolve(&self, name: Name) -> Resolving {
        let matches = name.as_str().eq_ignore_ascii_case(&self.host);
        let addresses = self.addresses.clone();
        Box::pin(async move {
            if !matches {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied).into());
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

pub(crate) async fn request(url: &str, method: reqwest::Method) -> Result<reqwest::RequestBuilder, Failure> {
    let parsed = reqwest::Url::parse(url).map_err(|_| invalid())?;
    if !matches!(parsed.scheme(), "http" | "https") || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(invalid());
    }
    let host = parsed.host_str().ok_or_else(invalid)?;
    let host = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
    let port = parsed.port_or_known_default().ok_or_else(invalid)?;
    let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::lookup_host((host, port)))
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?
            .collect()
    };
    let pinned = Pinned::new(host.into(), addresses)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(pinned))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|_| unavailable())?;
    Ok(client.request(method, parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refuses_each_non_public_address_class_and_encoded_variants() {
        for host in [
            "127.0.0.1",
            "127.255.255.254",
            "2130706433",
            "[::1]",
            "169.254.0.1",
            "169.254.169.254",
            "169.254.255.254",
            "[fe80::1]",
            "[febf::1]",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.0.1",
            "[fc00::1]",
            "[fdff::1]",
            "[fec0::1]",
            "224.0.0.1",
            "239.255.255.250",
            "[ff02::1]",
            "[ff0e::1]",
            "0.0.0.0",
            "255.255.255.255",
            "[::]",
            "100.64.0.1",
            "240.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "[2001:db8::1]",
            "[::ffff:127.0.0.1]",
            "[::ffff:10.0.0.1]",
            "[::ffff:169.254.169.254]",
            "[::ffff:224.0.0.1]",
            "[2002:7f00:1::]",
            "[64:ff9b::a9fe:a9fe]",
        ] {
            for method in [reqwest::Method::GET, reqwest::Method::PUT] {
                assert!(
                    request(&format!("http://{host}/file?signature=private"), method).await.is_err(),
                    "accepted {host}"
                );
            }
        }
    }

    #[tokio::test]
    async fn rejects_resolved_localhost_bad_schemes_and_user_information() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        for url in
            ["http://localhost/file", "file:///etc/passwd", "ftp://8.8.8.8/file", "http://user:secret@8.8.8.8/file"]
        {
            assert!(request(url, reqwest::Method::GET).await.is_err());
        }
        for url in ["http://8.8.8.8/file", "https://[2606:4700:4700::1111]/file"] {
            let request = request(url, reqwest::Method::GET)
                .await
                .unwrap_or_else(|_| panic!("public destination refused"))
                .build()
                .unwrap();
            assert_eq!(request.url().as_str(), url);
        }
    }

    #[test]
    fn rejects_empty_and_mixed_public_private_dns_answers() {
        assert!(Pinned::new("example.test".into(), vec![]).is_err());
        for ip in ["127.0.0.1", "169.254.169.254", "10.0.0.1", "fe80::1", "fc00::1", "224.0.0.1"] {
            assert!(
                Pinned::new(
                    "example.test".into(),
                    vec!["8.8.8.8:443".parse().unwrap(), SocketAddr::new(ip.parse().unwrap(), 443)]
                )
                .is_err()
            );
        }
    }

    #[tokio::test]
    async fn connector_resolution_never_rebinds_or_resolves_another_host() {
        let first = vec!["8.8.8.8:443".parse().unwrap(), "[2606:4700:4700::1111]:443".parse().unwrap()];
        let pinned =
            Pinned::new("example.test".into(), first.clone()).unwrap_or_else(|_| panic!("public DNS answers refused"));
        for _ in 0..3 {
            let resolved = pinned.resolve("example.test".parse().unwrap()).await.unwrap().collect::<Vec<_>>();
            assert_eq!(resolved, first, "a connection must use the first resolution's addresses");
            assert!(!resolved.iter().any(|a| a.ip() == IpAddr::V4(Ipv4Addr::LOCALHOST)));
        }
        assert!(pinned.resolve("other.test".parse().unwrap()).await.is_err());
    }
}
