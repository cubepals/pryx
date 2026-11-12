// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Native SSH leases hold a reserved logical TCP port without serving TCP. Forwarding resolves
//! that exact port to a scoped Unix worker; the private home mount contains no host-side keys.
use super::ssh::Launch;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub struct Ssh {
    launch: Launch,
    child: Mutex<std::process::Child>,
    stopped: AtomicBool,
    _home: tempfile::TempDir,
    _held: tokio::net::TcpListener,
}
impl Ssh {
    pub(super) fn new(
        launch: Launch,
        child: std::process::Child,
        home: tempfile::TempDir,
        held: tokio::net::TcpListener,
    ) -> Self {
        Self { launch, child: Mutex::new(child), stopped: AtomicBool::new(false), _home: home, _held: held }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::{linux::net::SocketAddrExt, unix::net::SocketAddr};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn scoped_ssh_lifetime_reserves_port_and_refuses_after_stop() {
        let root = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir_in(root.path()).unwrap();
        let home_path = home.path().to_path_buf();
        let address = format!("pryx-ssh-unit-{}", uuid::Uuid::new_v4());
        let listener =
            std::os::unix::net::UnixListener::bind_addr(&SocketAddr::from_abstract_name(&address).unwrap()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::UnixListener::from_std(listener).unwrap();
        let held = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = held.local_addr().unwrap();
        let child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let session = Ssh::new(
            Launch { directory: root.path().into(), home_mount: home_path.clone(), address },
            child,
            home,
            held,
        );
        assert!(session.running());
        assert!(tokio::net::TcpListener::bind(port).await.is_err());
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
        assert!(!home_path.exists());
        assert!(tokio::net::TcpListener::bind(port).await.is_ok());
    }
}
