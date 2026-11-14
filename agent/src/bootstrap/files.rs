// SPDX-License-Identifier: FSL-1.1-ALv2
//! Trusted VM maintenance serves persistent data using the existing authenticated handlers.
//! No image command, tools, spare forwarding, persistent logs or activity reaper starts here.
use super::*;

fn guest_identity(cid: u32) -> io::Result<()> {
    if !crate::vm_boot::guest_cid(cid)
        || rustix::process::getppid() != Some(rustix::process::Pid::INIT)
        || rustix::process::geteuid().is_root()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "VM file service requires its trusted non-root guest agent",
        ));
    }
    // Linux af_vsock.c::__vsock_bind rejects a non-local explicit CID. Unlike
    // get_local_cid(), binding an unprivileged port needs no root-only /dev/vsock open.
    let socket = vsock::VsockListener::bind(&vsock::VsockAddr::new(cid, libc::VMADDR_PORT_ANY))?;
    if socket.local_addr()?.cid() != cid {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "VM file service CID differs from its socket"));
    }
    Ok(())
}

impl Bootstrap {
    pub(super) async fn serve_files(self) -> Result<i32, Box<dyn std::error::Error>> {
        let cid = self.vm_cid.ok_or("VM file service is missing its guest CID")?;
        guest_identity(cid)?;
        let bindings = self.bindings(false)?;
        let encoded = std::env::var(&self.secret_env).map_err(|_| "Agent signing material is missing")?;
        let key = SigningKey::from_encoded(&encoded)?;
        let temporary = PathBuf::from("/tmp");
        let agent = Arc::new(Agent {
            file_tasks: Default::default(),
            shells: Default::default(),
            logs: Arc::new(crate::logbook::Logbook::open(&temporary)?),
            telemetry: Arc::default(),
            sandbox: self.sandbox.clone(),
            epoch: self.epoch,
            public_key: key.public_key(),
            key,
            data: DataDir::open(&self.data)?,
            processes: Processes::new(temporary, self.secret_env),
            ports: Ports::bind_ports(self.sandbox, self.domain, self.port, Vec::new()).await?,
        });
        let (servers, mut ingress) =
            serve_http(agent, self.port, &bindings, Some(cid), self.vm_port_base, &[], true).await?;
        let mut term = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        let result = tokio::select! {
            error = ingress_failure(&mut ingress) => Err(error.into()),
            _ = term.recv() => Ok(0),
            _ = interrupt.recv() => Ok(0),
        };
        for server in servers {
            server.abort();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_service_identity_refuses_the_host_before_opening_a_socket() {
        assert_ne!(rustix::process::getppid(), Some(rustix::process::Pid::INIT));
        assert_eq!(
            guest_identity(tokio_vsock::VMADDR_CID_HOST + 1).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(guest_identity(tokio_vsock::VMADDR_CID_HOST).is_err());
    }
    #[test]
    fn files_only_parser_refuses_host_and_workload_bootstraps() {
        let port = crate::contract::AGENT_PORT.to_string();
        let mode = format!("--{}", crate::bootstrap_capabilities::VM_FILES_ARGUMENT);
        let base = [
            "pryx-agent",
            "--sandbox",
            "one",
            "--epoch",
            "1",
            "--data",
            "/data",
            "--domain",
            "invalid",
            "--port",
            &port,
            "--secret-env",
            "private",
            &mode,
        ];
        let host = Bootstrap::try_parse_from(base).unwrap();
        assert!(host.validate_host_policy(false).is_err());
        let cid = (tokio_vsock::VMADDR_CID_HOST + 1).to_string();
        let guest = ["--vm-vsock", "--vm-cid", &cid];
        let args = Bootstrap::try_parse_from(base.into_iter().chain(guest)).unwrap();
        assert!(args.validate_host_policy(false).is_ok());
        let command = Bootstrap::try_parse_from(base.into_iter().chain(guest).chain(["--", "workload"])).unwrap();
        assert!(command.validate_host_policy(false).is_err());
        let spares = Bootstrap::try_parse_from(base.into_iter().chain(guest).chain(["--spare-ports", &port])).unwrap();
        assert!(spares.validate_host_policy(false).is_err());
    }
}
