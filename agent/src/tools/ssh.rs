// SPDX-License-Identifier: FSL-1.1-ALv2
//! Each OpenSSH listener gets one client key generated in memory. Only its public key and
//! ephemeral server host key reach private runtime files; no client private key is retained.
use super::{
    command, state,
    types::{SshCreated, SshView},
};
use crate::{
    auth::Claims,
    context::{Agent, Failure},
    ports::Ports,
    process::Process,
};
use ssh_key::{Algorithm, LineEnding, PrivateKey};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

pub(crate) struct SshSession {
    id: String,
    process: Arc<Process>,
    ports: Arc<Ports>,
    directory: tempfile::TempDir,
    port: u16,
    slot: u16,
    user: String,
    host_key: String,
    public_key: String,
    expires: AtomicI64,
    authorized: crate::paths::DataDir,
    authorized_path: String,
}
impl Drop for SshSession {
    fn drop(&mut self) {
        if self.process.view().running {
            let _ = self.process.signal(libc::SIGKILL);
        }
    }
}
struct PendingAuthorization {
    data: Option<crate::paths::DataDir>,
    path: String,
}
impl Drop for PendingAuthorization {
    fn drop(&mut self) {
        if let Some(data) = &self.data {
            let _ = data.delete(&self.path);
        }
    }
}
impl SshSession {
    fn view(&self) -> SshView {
        SshView {
            id: self.id.clone(),
            port: self.port,
            slot: self.slot,
            user: self.user.clone(),
            host_key: self.host_key.clone(),
            expires_at: self.expires.load(Ordering::Acquire),
            running: self.process.view().running,
        }
    }
    fn authorize(&self, expires: i64) -> Result<(), Failure> {
        let time = chrono::DateTime::<chrono::Utc>::from_timestamp(expires, 0)
            .ok_or_else(|| state::failure("invalid_request", "Invalid SSH expiry"))?
            .format("%Y%m%d%H%M%SZ");
        let keys = format!(
            "expiry-time=\"{time}\",no-agent-forwarding,no-port-forwarding,no-X11-forwarding {}\n",
            self.public_key
        );
        self.authorized.write_atomic(&self.authorized_path, keys.as_bytes(), false)?;
        Ok(())
    }
    async fn stop(&self) -> Result<(), Failure> {
        self.ports.unexpose_ssh(&self.id).await?;
        if self.process.view().running {
            let _ = self.process.signal(libc::SIGKILL);
            self.process.wait().await;
        }
        let _ = self.authorized.delete(&self.authorized_path);
        let _ = std::fs::remove_dir_all(self.directory.path());
        Ok(())
    }
}
fn key() -> Result<PrivateKey, Failure> {
    PrivateKey::random(&mut ssh_key::rand_core::OsRng, Algorithm::Ed25519)
        .map_err(|_| state::failure("key_generation", "Cannot generate SSH key"))
}

pub(crate) async fn create(agent: &Agent, claims: &Claims) -> Result<SshCreated, Failure> {
    state::native(agent, "SSH access")?;
    let state = state::get(agent)?;
    let mut sessions = state.ssh.lock().await;
    let dead: Vec<_> = sessions
        .iter()
        .filter(|(_, s)| !s.downcast_ref::<SshSession>().expect("SSH state").process.view().running)
        .map(|(id, _)| id.clone())
        .collect();
    for id in dead {
        if let Some(session) = sessions.remove(&id) {
            session.downcast::<SshSession>().expect("SSH state").stop().await?
        }
    }
    if sessions.len() >= usize::from(claims.max_ports) {
        return Err(state::failure("ssh_capacity", "No SSH listener slot is available"));
    }
    let expires = state::expiry(claims)?;
    let user = nix::unistd::User::from_uid(nix::unistd::getuid())
        .map_err(std::io::Error::from)?
        .ok_or_else(|| {
            state::failure("tool_unavailable", "OpenSSH needs a named sandbox user; use the tools template")
        })?
        .name;
    if nix::unistd::getuid().is_root() || !user.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return Err(state::failure("unsupported", "OpenSSH requires a named non-root sandbox user"));
    }
    let client = key()?;
    let host = key()?;
    let public_key = client
        .public_key()
        .to_openssh()
        .map_err(|_| state::failure("key_generation", "Cannot format SSH public key"))?;
    let host_key =
        host.public_key().to_openssh().map_err(|_| state::failure("key_generation", "Cannot format SSH host key"))?;
    let directory = tempfile::Builder::new().prefix("ssh-").tempdir_in(state.directory.path())?;
    let files = crate::paths::DataDir::open(directory.path())?;
    let host_private =
        host.to_openssh(LineEnding::LF).map_err(|_| state::failure("key_generation", "Cannot format SSH host key"))?;
    files.write_atomic("host_key", host_private.as_bytes(), false)?;
    let held = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let port = held.local_addr()?.port();
    let id = uuid::Uuid::new_v4().to_string();
    let authorized_path = format!(".pryx-agent/ssh/{id}.pub");
    let full_authorized = agent.processes.data.join(&authorized_path).to_string_lossy().into_owned();
    if full_authorized.contains(['\n', '\r']) {
        return Err(state::failure("unsupported", "SSH data path contains a line separator"));
    }
    let quoted_authorized = full_authorized.replace('\\', "\\\\").replace('"', "\\\"");
    let private_path = directory.path().to_string_lossy();
    if private_path.contains(['\n', '\r']) {
        return Err(state::failure("unsupported", "SSH temporary path contains a line separator"));
    }
    let quoted_private = private_path.replace('\\', "\\\\").replace('"', "\\\"");
    let config = format!(
        "Port {port}\nListenAddress 127.0.0.1\nHostKey \"{}/host_key\"\nAuthorizedKeysFile \"{quoted_authorized}\"\nPidFile \"{}/pid\"\nAllowUsers {user}\nUsePAM no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nAuthenticationMethods publickey\nPubkeyAuthentication yes\nPermitRootLogin no\nPermitEmptyPasswords no\nAllowAgentForwarding no\nAllowTcpForwarding no\nX11Forwarding no\nPermitTunnel no\nPermitUserEnvironment no\nPermitUserRC no\nUseDNS no\nStrictModes yes\nLogLevel ERROR\nSubsystem sftp internal-sftp\n",
        quoted_private, quoted_private
    );
    files.write_atomic("config", config.as_bytes(), false)?;
    // OpenSSH reads authorized_keys for every authentication, including after a pause.
    let time =
        chrono::DateTime::<chrono::Utc>::from_timestamp(expires, 0).expect("bounded expiry").format("%Y%m%d%H%M%SZ");
    let authorized = crate::paths::DataDir::open(&agent.processes.data)?;
    let mut pending = PendingAuthorization { data: Some(authorized), path: authorized_path.clone() };
    pending.data.as_ref().expect("pending key").write_atomic(
        &authorized_path,
        format!("expiry-time=\"{time}\",no-agent-forwarding,no-port-forwarding,no-X11-forwarding {public_key}\n")
            .as_bytes(),
        false,
    )?;
    drop(held);
    let process = command::start(
        agent,
        vec![
            "/usr/sbin/sshd".into(),
            "-D".into(),
            "-e".into(),
            "-f".into(),
            directory.path().join("config").to_string_lossy().into_owned(),
        ],
        BTreeMap::new(),
        None,
        false,
    )?;
    let mut starting = command::Starting(Some(process.clone()));
    let ready=async {
        tokio::select!{result=state::ready(agent,port)=>result,_=process.wait()=>Err(state::failure("tool_died","OpenSSH exited before readiness"))}
    }.await;
    if let Err(error) = ready {
        let _ = process.signal(libc::SIGKILL);
        return Err(error);
    }
    let slot = match agent.ports.expose_ssh(port, &id, claims.max_ports).await {
        Ok(slot) => slot,
        Err(error) => {
            let _ = process.signal(libc::SIGKILL);
            return Err(error.into());
        }
    };
    let authorized = pending.data.take().expect("pending authorization");
    let session = Arc::new(SshSession {
        id: id.clone(),
        process,
        ports: agent.ports.clone(),
        directory,
        port,
        slot,
        user,
        host_key,
        public_key,
        expires: AtomicI64::new(expires),
        authorized,
        authorized_path,
    });
    let weak = Arc::downgrade(&session);
    tokio::spawn(async move {
        loop {
            let Some(session) = weak.upgrade() else { return };
            let remaining = session.expires.load(Ordering::Acquire) - crate::telemetry::now();
            if remaining <= 0 || !session.process.view().running {
                let _ = session.stop().await;
                return;
            }
            drop(session);
            tokio::time::sleep(std::time::Duration::from_secs((remaining as u64).min(1))).await;
        }
    });
    let view = session.view();
    sessions.insert(id, session);
    starting.0 = None;
    let encoded = client
        .to_openssh(LineEnding::LF)
        .map_err(|_| state::failure("key_generation", "Cannot format SSH client key"))?;
    Ok(SshCreated { view, key: encoded.to_string() })
}
pub(crate) async fn view(agent: &Agent, id: &str) -> Result<SshView, Failure> {
    let state = state::get(agent)?;
    let sessions = state.ssh.lock().await;
    let session = sessions
        .get(id)
        .and_then(|s| s.downcast_ref::<SshSession>())
        .ok_or_else(|| state::failure("not_found", "SSH lease is unavailable"))?;
    if session.expires.load(Ordering::Acquire) <= crate::telemetry::now() || !session.process.view().running {
        return Err(state::failure("expired", "SSH lease expired"));
    }
    Ok(session.view())
}
pub(crate) async fn expire(agent: &Agent, id: &str, expires: i64) -> Result<SshView, Failure> {
    let state = state::get(agent)?;
    let sessions = state.ssh.lock().await;
    let session = sessions
        .get(id)
        .and_then(|s| s.downcast_ref::<SshSession>())
        .ok_or_else(|| state::failure("not_found", "SSH lease is unavailable"))?;
    if expires > session.expires.load(Ordering::Acquire) || expires <= crate::telemetry::now() {
        return Err(state::failure("invalid_request", "SSH expiry can only be shortened into the future"));
    }
    session.authorize(expires)?;
    session.expires.store(expires, Ordering::Release);
    Ok(session.view())
}
pub(crate) async fn revoke(agent: &Agent, id: &str) -> Result<(), Failure> {
    let state = state::get(agent)?;
    let session = state.ssh.lock().await.remove(id).map(|s| s.downcast::<SshSession>().expect("SSH state"));
    if let Some(session) = session {
        session.stop().await?;
    }
    Ok(())
}
