// SPDX-License-Identifier: FSL-1.1-ALv2
//! Host-CID-only virtio-socket ingress for a VM with or without a network device. The existing
//! agent owns authentication, ports and protocols; this adapter only relays their bytes.
use std::{io, net::Ipv4Addr};
use tokio::{net::TcpStream, task::JoinSet};
use tokio_vsock::{VMADDR_CID_HOST, VsockAddr, VsockListener};

pub(crate) struct Ingress {
    tasks: JoinSet<io::Result<()>>,
}

impl Ingress {
    pub(crate) fn bind(ports: impl IntoIterator<Item = u16>, cid: u32, base: u32) -> io::Result<Self> {
        if !crate::vm_boot::guest_cid(cid)
            || (rustix::process::getpid() != rustix::process::Pid::INIT
                && rustix::process::getppid() != Some(rustix::process::Pid::INIT))
        {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "VM ingress requires the guest supervisor"));
        }
        let mut tasks = JoinSet::new();
        // Bind every listener before any accept loop starts. A partial failure drops
        // the listeners and no task can expose an incomplete set of forward slots.
        let listeners = ports
            .into_iter()
            .map(|port| VsockListener::bind(VsockAddr::new(cid, base + u32::from(port))).map(|l| (port, l)))
            .collect::<io::Result<Vec<_>>>()?;
        for (port, listener) in listeners {
            tasks.spawn(async move {
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let (mut peer, address) = accepted?;
                            if address.cid() != VMADDR_CID_HOST {
                                continue;
                            }
                            connections.spawn(async move {
                                let mut guest = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await?;
                                guest.set_nodelay(true)?;
                                tokio::io::copy_bidirectional(&mut peer, &mut guest).await?;
                                Ok::<_, io::Error>(())
                            });
                        }
                        _ = connections.join_next(), if !connections.is_empty() => {}
                    }
                }
            });
        }
        Ok(Self { tasks })
    }

    pub(crate) async fn failed(&mut self) -> io::Error {
        match self.tasks.join_next().await {
            Some(Ok(Err(error))) => error,
            Some(Err(error)) => io::Error::other(error),
            _ => io::Error::other("VM ingress stopped"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ordinary_host_process_cannot_enable_vm_ingress() {
        assert_ne!(rustix::process::getpid(), rustix::process::Pid::INIT);
        let result = Ingress::bind([crate::contract::AGENT_PORT], VMADDR_CID_HOST + 1, 0);
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied));
    }
}
