// SPDX-License-Identifier: FSL-1.1-ALv2
//! Admit the installed VM worker only through a private host-owned Unix socket and kernel
//! peer identity. One exclusive file lease prevents replacement of a live server's socket.
//! HTTP and runtime dispatch belong to their owners; this provides no privileged operation.
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};
use tokio::net::{UnixListener, UnixStream};

fn denied() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "VM worker authority is not private and host-owned")
}

fn parent(path: &Path, owner: u32) -> io::Result<()> {
    let parent = path.parent().ok_or_else(denied)?;
    super::super::bundle::ancestors(parent)?;
    let info = fs::symlink_metadata(parent)?;
    if !info.is_dir() || info.uid() != owner || info.mode() & 0o077 != 0 {
        return Err(denied());
    }
    Ok(())
}

fn socket(path: &Path, owner: u32) -> io::Result<()> {
    parent(path, owner)?;
    let info = fs::symlink_metadata(path)?;
    if !info.file_type().is_socket() || info.uid() != owner || info.mode() & 0o077 != 0 || info.nlink() != 1 {
        return Err(denied());
    }
    Ok(())
}

fn peer(stream: &UnixStream, owner: u32) -> io::Result<()> {
    let credentials = stream.peer_cred()?;
    if credentials.uid() != owner || credentials.pid().is_none_or(|pid| pid <= 0) {
        return Err(denied());
    }
    Ok(())
}

pub async fn connect(path: &Path, owner: u32) -> io::Result<UnixStream> {
    socket(path, owner)?;
    let stream = UnixStream::connect(path).await?;
    peer(&stream, owner)?;
    Ok(stream)
}

pub struct Endpoint {
    listener: UnixListener,
    _lease: File,
    owner: u32,
}

#[derive(Clone)]
pub struct Peer(Option<tokio::net::unix::UCred>);

impl Peer {
    pub(super) fn credentials(&self) -> io::Result<tokio::net::unix::UCred> {
        self.0.ok_or_else(denied)
    }
}

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, Endpoint>> for Peer {
    fn connect_info(stream: axum::serve::IncomingStream<'_, Endpoint>) -> Self {
        Self(stream.io().peer_cred().ok())
    }
}

impl Endpoint {
    /// The owner comes from fixed host policy, never a request or a socket's file metadata.
    pub async fn bind(path: &Path, owner: u32) -> io::Result<Self> {
        if rustix::process::geteuid().as_raw() != owner {
            return Err(denied());
        }
        parent(path, owner)?;
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path.with_extension("lock"))?;
        let info = lease.metadata()?;
        if !info.is_file() || info.uid() != owner || info.mode() & 0o077 != 0 || info.nlink() != 1 {
            return Err(denied());
        }
        rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive)?;
        match fs::symlink_metadata(path) {
            Ok(_) => {
                socket(path, owner)?;
                match UnixStream::connect(path).await {
                    Ok(_) => return Err(io::Error::new(io::ErrorKind::AddrInUse, "VM worker socket is live")),
                    Err(problem) if problem.kind() == io::ErrorKind::ConnectionRefused => fs::remove_file(path)?,
                    Err(problem) => return Err(problem),
                }
            }
            Err(problem) if problem.kind() == io::ErrorKind::NotFound => (),
            Err(problem) => return Err(problem),
        }
        let listener = UnixListener::bind(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(Self { listener, _lease: lease, owner })
    }

    pub async fn accept(&self) -> io::Result<UnixStream> {
        let (stream, _) = self.listener.accept().await?;
        peer(&stream, self.owner)?;
        Ok(stream)
    }
}

impl axum::serve::Listener for Endpoint {
    type Io = UnixStream;
    type Addr = tokio::net::unix::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match Self::accept(self).await {
                Ok(stream) => match stream.peer_addr() {
                    Ok(address) => return (stream, address),
                    Err(_) => continue,
                },
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn actual_socket_authenticates_peers_and_preserves_one_live_authority() {
        let root = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
        let path = root.path().join("worker.sock");
        let owner = rustix::process::geteuid().as_raw();
        let endpoint = Endpoint::bind(&path, owner).await.unwrap();
        assert!(Endpoint::bind(&path, owner).await.is_err());
        let challenge = uuid::Uuid::new_v4();
        let mut client = connect(&path, owner).await.unwrap();
        let mut server = endpoint.accept().await.unwrap();
        assert!(peer(&server, owner.checked_add(1).unwrap()).is_err());
        client.write_all(challenge.as_bytes()).await.unwrap();
        let mut received = *challenge.as_bytes();
        received.fill(0);
        server.read_exact(&mut received).await.unwrap();
        assert_eq!(received, *challenge.as_bytes());
        drop((server, client, endpoint));
        let replacement = Endpoint::bind(&path, owner).await.unwrap();
        assert!(connect(&path, owner).await.is_ok());
        drop(replacement);
    }

    #[tokio::test]
    async fn links_public_state_and_non_socket_files_cannot_become_worker_authority() {
        let root = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
        let path = root.path().join("worker.sock");
        let owner = rustix::process::geteuid().as_raw();
        let challenge = uuid::Uuid::new_v4();
        fs::write(&path, challenge.as_bytes()).unwrap();
        assert!(Endpoint::bind(&path, owner).await.is_err());
        assert!(connect(&path, owner).await.is_err());
        assert_eq!(fs::read(&path).unwrap(), challenge.as_bytes());
        fs::remove_file(&path).unwrap();
        symlink(root.path().join("absent"), &path).unwrap();
        assert!(Endpoint::bind(&path, owner).await.is_err());
        assert!(connect(&path, owner).await.is_err());
        fs::remove_file(&path).unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Endpoint::bind(&path, owner).await.is_err());
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let lock = path.with_extension("lock");
        let extra = root.path().join("extra-lock");
        fs::hard_link(&lock, &extra).unwrap();
        assert!(Endpoint::bind(&path, owner).await.is_err());
    }
}
