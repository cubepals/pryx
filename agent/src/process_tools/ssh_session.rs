// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Native SSH leases hold a reserved logical TCP port without serving TCP. Forwarding resolves
//! that exact port to a scoped Unix worker; authorization never enters persistent data.
use super::ssh::Launch;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub struct Ssh {
    launch: Launch,
    child: Mutex<std::process::Child>,
    stopped: AtomicBool,
    reservation: tokio::net::TcpListener,
}
impl Ssh {
    pub(super) fn new(launch: Launch, child: std::process::Child, reservation: tokio::net::TcpListener) -> Self {
        Self { launch, child: Mutex::new(child), stopped: AtomicBool::new(false), reservation }
    }
    pub fn running(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && self.child.lock().unwrap().try_wait().is_ok_and(|status| status.is_none())
    }
    pub(super) fn ready(&self) -> bool {
        self.launch.directory.join("ready").is_file()
    }
    pub async fn connect(&self) -> std::io::Result<tokio::net::UnixStream> {
        if !self.running() {
            return Err(std::io::ErrorKind::NotConnected.into());
        }
        tokio::net::UnixStream::connect(format!("\0{}", self.launch.address)).await
    }
    pub fn stop(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}
impl Drop for Ssh {
    fn drop(&mut self) {
        self.stop();
        // Release the listening port even if a concurrent fork still holds a copy of the socket.
        let _ = rustix::net::shutdown(&self.reservation, rustix::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::{fd::AsFd, linux::net::SocketAddrExt, unix::net::SocketAddr};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn scoped_ssh_lifetime_reserves_port_and_refuses_after_stop() {
        let root = tempfile::tempdir().unwrap();
        let address = format!("pryx-ssh-unit-{}", uuid::Uuid::new_v4());
        let listener =
            std::os::unix::net::UnixListener::bind_addr(&SocketAddr::from_abstract_name(&address).unwrap()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::UnixListener::from_std(listener).unwrap();
        // Other concurrent tests allocate ephemeral ports on localhost. Use a fresh
        // loopback address so they cannot claim this fixture's port after its release.
        let mut octets = std::net::Ipv4Addr::LOCALHOST.octets();
        octets[1..].copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..3]);
        let held = tokio::net::TcpListener::bind((std::net::Ipv4Addr::from(octets), 0)).await.unwrap();
        let port = held.local_addr().unwrap();
        // A concurrent fork can hold the same socket until the child's exec closes its descriptor.
        let inherited = held.as_fd().try_clone_to_owned().unwrap();
        let child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let session = Ssh::new(Launch { directory: root.path().into(), address }, child, held);
        assert!(session.running());
        assert_eq!(tokio::net::TcpListener::bind(port).await.unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            stream.write_all(b"private\0SSH\xff").await.unwrap();
        });
        let mut stream = session.connect().await.unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes, b"private\0SSH\xff");
        peer.await.unwrap();
        session.stop();
        assert!(!session.running());
        assert_eq!(session.connect().await.unwrap_err().kind(), std::io::ErrorKind::NotConnected);
        drop(session);
        tokio::net::TcpListener::bind(port).await.expect("Dropping the SSH lease must immediately release its port");
        drop(inherited);
    }
}
