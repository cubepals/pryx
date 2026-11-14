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
        input.write_all(DISCONNECTED).await?;
        input.shutdown().await?;
        drop(input);
        if !child.wait().await?.success() {
            return Err(io::Error::other("Trusted guest firewall transaction failed"));
        }
        Ok(())
    })
    .await?
}
