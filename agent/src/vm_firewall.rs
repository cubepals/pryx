// SPDX-License-Identifier: FSL-1.1-ALv2
//! Apply host-owned guest policy before the unprivileged agent exists. The unchanged nft
//! program and its libraries come only from the pinned read-only bootstrap, never the image.
use crate::vm_boot;
use std::{io, path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};

// nft(8): inet hooks cover IPv4 and IPv6; loopback remains available to the guest agent.
const DISCONNECTED: &[u8] = b"table inet pryx {
 chain input { type filter hook input priority filter; policy drop; iifname lo accept; }
 chain output { type filter hook output priority filter; policy drop; oifname lo accept; }
 chain forward { type filter hook forward priority filter; policy drop; }
}
";

pub(crate) async fn disconnected() -> io::Result<()> {
    apply(DISCONNECTED).await
}

pub(crate) async fn redirected(network: &vm_boot::Network) -> io::Result<()> {
    let policy = network.policy.as_ref().ok_or_else(|| io::Error::other("Missing trusted guest redirects"))?;
    if policy.unrestricted {
        return public_network(policy).await;
    }
    let rules = format!(
        "table inet pryx {{
 chain hostname_redirect {{ type nat hook output priority dstnat; policy accept;
  oifname lo return;
  meta nfproto ipv4 tcp dport {http} dnat ip to {host}:{http_target};
  meta nfproto ipv4 tcp dport {tls} dnat ip to {host}:{tls_target};
  meta nfproto ipv4 tcp dport {dns} dnat ip to {host}:{dns_target};
  meta nfproto ipv4 udp dport {dns} dnat ip to {host}:{dns_target};
 }}
 chain input {{ type filter hook input priority filter; policy drop;
  iifname lo accept;
  ct state established,related accept;
  udp sport {server} udp dport {client} accept;
 }}
 chain output {{ type filter hook output priority filter; policy drop;
  oifname lo accept;
  udp sport {client} udp dport {server} accept;
  ip daddr {host} tcp dport {{ {http_target}, {tls_target}, {dns_target} }} accept;
  ip daddr {host} udp dport {dns_target} accept;
 }}
 chain forward {{ type filter hook forward priority filter; policy drop; }}
}}\n",
        host = policy.host,
        http = policy.http.source,
        http_target = policy.http.target,
        tls = policy.tls.source,
        tls_target = policy.tls.target,
        dns = policy.dns.source,
        dns_target = policy.dns.target,
        client = policy.dhcp_client,
        server = policy.dhcp_server,
    );
    apply(rules.as_bytes()).await
}

async fn public_network(policy: &vm_boot::NetworkPolicy) -> io::Result<()> {
    let blocked = crate::transfer_target::non_public_ipv4_networks()
        .map(|(address, bits)| format!("{address}/{bits}"))
        .collect::<Vec<_>>()
        .join(", ");
    let rules = format!(
        "table inet pryx {{
 set non_public_v4 {{ type ipv4_addr; flags interval; elements = {{ {blocked} }}; }}
 chain resolver_redirect {{ type nat hook output priority dstnat; policy accept;
  oifname lo return;
  meta nfproto ipv4 tcp dport {dns} dnat ip to {host}:{target};
  meta nfproto ipv4 udp dport {dns} dnat ip to {host}:{target};
 }}
 chain input {{ type filter hook input priority filter; policy drop;
  iifname lo accept;
  ct state established,related accept;
  udp sport {server} udp dport {client} accept;
 }}
 chain output {{ type filter hook output priority filter; policy drop;
  oifname lo accept;
  udp sport {client} udp dport {server} accept;
  ip daddr {host} tcp dport {target} accept;
  ip daddr {host} udp dport {target} accept;
  ip daddr @non_public_v4 drop;
  meta nfproto ipv4 accept;
 }}
 chain forward {{ type filter hook forward priority filter; policy drop; }}
}}\n",
        host = policy.host,
        dns = policy.dns.source,
        target = policy.dns.target,
        client = policy.dhcp_client,
        server = policy.dhcp_server,
    );
    apply(rules.as_bytes()).await
}

async fn apply(rules: &[u8]) -> io::Result<()> {
    let root = Path::new(vm_boot::BOOTSTRAP_ROOT).join(vm_boot::FIREWALL_DIRECTORY);
    let loader = root.join(vm_boot::FIREWALL_LOADER);
    let libraries = loader.parent().expect("fixed firewall loader has a parent");
    let mut child = Command::new(&loader)
        .arg("--library-path")
        .arg(libraries)
        .arg(root.join(vm_boot::FIREWALL_PROGRAM))
        .args(["--file", "-"])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut input = child.stdin.take().expect("private firewall input configured");
        input.write_all(rules).await?;
        input.shutdown().await?;
        drop(input);
        if !child.wait().await?.success() {
            return Err(io::Error::other("Trusted guest firewall transaction failed"));
        }
        Ok(())
    })
    .await?
}
