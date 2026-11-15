// SPDX-License-Identifier: FSL-1.1-ALv2
//! A worker's writable compute belongs to one kernel-authenticated frontend lifetime.
//! Caller loss closes admission, cancels streams and drains owned operations before retiring
//! compute. This neither grants a runtime nor interprets a sandbox request or heartbeat.
use super::transport;
use std::{
    future::Future,
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::{OwnedRwLockReadGuard, RwLock},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

// This transport owns its fixed readiness acknowledgement. The lifetime carries no commands.
const READY: &[u8] = b"pryx worker owner v1\n";

pub struct Owner {
    _connection: UnixStream,
}

impl Owner {
    pub async fn connect(path: &Path, uid: u32) -> io::Result<Self> {
        Self::ready(transport::connect(path, uid).await?).await
    }

    pub(super) async fn ready(mut connection: UnixStream) -> io::Result<Self> {
        let mut ready = vec![0; READY.len()];
        connection.read_exact(&mut ready).await?;
        if ready != READY {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Worker lifetime acknowledgement differs"));
        }
        Ok(Self { _connection: connection })
    }
}

#[derive(Clone)]
pub struct Authority {
    gate: Arc<RwLock<()>>,
    closing: Arc<AtomicBool>,
    cancel: CancellationToken,
    uid: u32,
    pid: i32,
}

pub struct Permit {
    _admitted: OwnedRwLockReadGuard<()>,
}

fn refused() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "Worker frontend authority is absent, different or retiring")
}

impl Authority {
    /// The lifetime peer comes from an already admitted private socket. The fixed expected
    /// UID is installed host policy; only the kernel supplies the frontend process identity.
    pub fn watch(
        mut connection: UnixStream,
        uid: u32,
        retire: impl Future<Output = io::Result<()>> + Send + 'static,
    ) -> io::Result<(Self, JoinHandle<io::Result<()>>)> {
        let credentials = connection.peer_cred()?;
        let pid = credentials.pid().filter(|pid| *pid > 0).ok_or_else(refused)?;
        if credentials.uid() != uid {
            return Err(refused());
        }
        let authority = Self {
            gate: Arc::new(RwLock::new(())),
            closing: Arc::new(AtomicBool::new(false)),
            cancel: CancellationToken::new(),
            uid,
            pid,
        };
        let owned = authority.clone();
        let task = tokio::spawn(async move {
            let observation = async {
                connection.write_all(READY).await?;
                let mut byte = [0];
                if connection.read(&mut byte).await? != 0 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Worker lifetime accepts only EOF"));
                }
                Ok(())
            }
            .await;
            // Even an acknowledgement/read failure must close admission and retire compute.
            owned.closing.store(true, Ordering::Release);
            owned.cancel.cancel();
            let _drained = owned.gate.write_owned().await;
            retire.await?;
            observation
        });
        Ok((authority, task))
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub async fn admit(&self, connection: &UnixStream) -> io::Result<Permit> {
        self.admit_credentials(connection.peer_cred()?).await
    }

    pub async fn admit_peer(&self, peer: &transport::Peer) -> io::Result<Permit> {
        self.admit_credentials(peer.credentials()?).await
    }

    async fn admit_credentials(&self, credentials: tokio::net::unix::UCred) -> io::Result<Permit> {
        if credentials.uid() != self.uid || credentials.pid() != Some(self.pid) || self.closing.load(Ordering::Acquire)
        {
            return Err(refused());
        }
        let admitted = self.gate.clone().read_owned().await;
        if self.closing.load(Ordering::Acquire) {
            return Err(refused());
        }
        Ok(Permit { _admitted: admitted })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
        process::Stdio,
        time::Duration,
    };
    use tokio::{process::Command, time::timeout};

    #[tokio::test]
    async fn actual_frontend_eof_drains_operations_before_releasing_compute_authority() {
        let root = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
        let socket = root.path().join("owner.sock");
        let uid = rustix::process::geteuid().as_raw();
        let endpoint = transport::Endpoint::bind(&socket, uid).await.unwrap();
        let path = socket.clone();
        let connecting = tokio::spawn(async move { Owner::connect(&path, uid).await.unwrap() });
        let lifetime = timeout(Duration::from_secs(2), endpoint.accept()).await.unwrap().unwrap();
        let lease_path = root.path().join("compute.lock");
        let lease =
            fs::OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&lease_path).unwrap();
        rustix::fs::flock(&lease, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
        let observer = fs::OpenOptions::new().read(true).write(true).open(&lease_path).unwrap();
        let (authority, retirement) = Authority::watch(lifetime, uid, async move {
            drop(lease);
            Ok(())
        })
        .unwrap();
        let owner = connecting.await.unwrap();
        let request = transport::connect(&socket, uid).await.unwrap();
        let accepted = endpoint.accept().await.unwrap();
        let permit = authority.admit(&accepted).await.unwrap();
        let mut foreign = Command::new("python3")
            .args([
                "-c",
                "import socket,sys,time; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); time.sleep(30)",
            ])
            .arg(&socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let foreign_socket = timeout(Duration::from_secs(2), endpoint.accept()).await.unwrap().unwrap();
        assert!(authority.admit(&foreign_socket).await.is_err(), "Another actual PID cannot borrow frontend authority");
        foreign.kill().await.unwrap();
        drop(owner);
        timeout(Duration::from_secs(2), authority.cancellation().cancelled()).await.unwrap();
        assert!(authority.admit(&accepted).await.is_err());
        assert_eq!(
            rustix::fs::flock(&observer, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap_err(),
            rustix::io::Errno::WOULDBLOCK
        );
        assert!(!retirement.is_finished(), "An admitted operation must keep compute authority owned");
        drop(permit);
        timeout(Duration::from_secs(2), retirement).await.unwrap().unwrap().unwrap();
        rustix::fs::flock(&observer, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
        assert!(authority.admit(&accepted).await.is_err());
        drop(request);
    }
}
