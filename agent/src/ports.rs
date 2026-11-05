// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Forward slots bind once at sandbox creation. Exposing changes a target in memory; it cannot
//! request another host binding, restart compute or forward outside the sandbox's loopback.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use crate::{
    paths::DataDir,
    state::{self, PORTS_PATH, PortState},
    telemetry::Telemetry,
};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, watch};

pub use crate::contract::{AGENT_PORT, MAX_SPARE_PORTS};

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Exposed {
    pub port: u16,
    pub slot: u16,
    pub url: String,
}

struct Slot {
    port: u16,
    target: watch::Sender<Option<u16>>,
}

pub struct Ports {
    held: Mutex<Vec<Slot>>,
    sandbox: String,
    domain: String,
    agent_port: u16,
    revision: AtomicU64,
    checkpoint: Option<DataDir>,
    telemetry: Arc<Telemetry>,
}

#[derive(Debug, thiserror::Error)]
pub enum PortError {
    #[error("No spare port is available")]
    NoSparePort,
    #[error("Invalid target port")]
    Invalid,
    #[error("Cannot persist port assignment")]
    Persistence,
}

impl Ports {
    pub async fn bind(sandbox: String, domain: String, count: u16) -> std::io::Result<Arc<Self>> {
        Self::bind_ports(sandbox, domain, AGENT_PORT, (1..=count).map(|i| AGENT_PORT + i).collect()).await
    }

    pub async fn bind_ports(
        sandbox: String,
        domain: String,
        agent_port: u16,
        ports: Vec<u16>,
    ) -> std::io::Result<Arc<Self>> {
        Self::bind_persistent(sandbox, domain, agent_port, ports, None, Arc::default()).await
    }

    pub async fn bind_persistent(
        sandbox: String,
        domain: String,
        agent_port: u16,
        ports: Vec<u16>,
        checkpoint: Option<DataDir>,
        telemetry: Arc<Telemetry>,
    ) -> std::io::Result<Arc<Self>> {
        if ports.len() > usize::from(MAX_SPARE_PORTS) || ports.contains(&agent_port) || ports.contains(&0) {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        let restored: PortState = match &checkpoint {
            Some(data) => state::read(data, PORTS_PATH)?,
            None => PortState::default(),
        };
        let mut seen = std::collections::HashSet::new();
        if restored.revision > state::MAX_REVISION
            || restored.targets.len() > ports.len()
            || restored
                .targets
                .iter()
                .flatten()
                .any(|p| *p == 0 || *p == agent_port || ports.contains(p) || !seen.insert(*p))
        {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let mut slots = Vec::new();
        for (index, port) in ports.into_iter().enumerate() {
            let listener = TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).await?;
            let target = watch::channel(restored.targets.get(index).copied().flatten()).0;
            tokio::spawn(serve(listener, target.subscribe(), telemetry.clone()));
            slots.push(Slot { port, target });
        }
        Ok(Arc::new(Self {
            held: Mutex::new(slots),
            sandbox,
            domain,
            agent_port,
            revision: AtomicU64::new(restored.revision),
            checkpoint,
            telemetry,
        }))
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    fn persist(&self, held: &[Slot], changed: usize, value: Option<u16>) -> Result<(), PortError> {
        let revision = self.revision().checked_add(1).ok_or(PortError::Persistence)?;
        if revision > state::MAX_REVISION {
            return Err(PortError::Persistence);
        }
        if let Some(data) = &self.checkpoint {
            let mut targets: Vec<_> = held.iter().map(|s| *s.target.borrow()).collect();
            targets[changed] = value;
            state::write(data, PORTS_PATH, &PortState { revision, targets }, true)
                .map_err(|_| PortError::Persistence)?;
        }
        self.revision.store(revision, Ordering::Release);
        self.telemetry.touch();
        Ok(())
    }

    fn view(&self, slot: &Slot, target: u16) -> Exposed {
        Exposed { port: target, slot: slot.port, url: format!("https://{}-{target}.{}", self.sandbox, self.domain) }
    }

    pub async fn list(&self) -> Vec<Exposed> {
        self.held.lock().await.iter().filter_map(|s| s.target.borrow().map(|t| self.view(s, t))).collect()
    }

    pub async fn expose(&self, target: u16, allowed: u16) -> Result<Exposed, PortError> {
        let held = self.held.lock().await;
        if target == 0 || target == self.agent_port || held.iter().any(|s| s.port == target) {
            return Err(PortError::Invalid);
        }
        if let Some(slot) = held.iter().find(|s| *s.target.borrow() == Some(target)) {
            return Ok(self.view(slot, target));
        }
        if held.iter().filter(|s| s.target.borrow().is_some()).count() >= usize::from(allowed) {
            return Err(PortError::NoSparePort);
        }
        let index = held.iter().position(|s| s.target.borrow().is_none()).ok_or(PortError::NoSparePort)?;
        self.persist(&held, index, Some(target))?;
        let slot = &held[index];
        slot.target.send_replace(Some(target));
        Ok(self.view(slot, target))
    }

    pub async fn unexpose(&self, target: u16) -> Result<(), PortError> {
        let held = self.held.lock().await;
        if let Some(index) = held.iter().position(|s| *s.target.borrow() == Some(target)) {
            self.persist(&held, index, None)?;
            held[index].target.send_replace(None);
        }
        Ok(())
    }
}

async fn serve(listener: TcpListener, target: watch::Receiver<Option<u16>>, telemetry: Arc<Telemetry>) {
    while let Ok((mut downstream, _)) = listener.accept().await {
        let _ = downstream.set_nodelay(true);
        let mut assigned = target.clone();
        let port = *assigned.borrow_and_update();
        if let Some(port) = port {
            let telemetry = telemetry.clone();
            tokio::spawn(async move {
                let header = tokio::time::timeout(Duration::from_secs(3), forward_header(&mut downstream, port)).await;
                let Ok(Ok(header)) = header else { return };
                if assigned.has_changed().unwrap_or(true) {
                    return;
                }
                let _active = telemetry.active();
                let connected = tokio::time::timeout(
                    Duration::from_secs(3),
                    TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)),
                )
                .await;
                let Ok(Ok(mut upstream)) = connected else { return };
                let _ = upstream.set_nodelay(true);
                if upstream.write_all(&header).await.is_err() {
                    return;
                }
                tokio::select! {
                    _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream) => {},
                    _ = assigned.changed() => {},
                }
            });
        }
    }
}

async fn forward_header(stream: &mut TcpStream, target: u16) -> std::io::Result<Vec<u8>> {
    // A reused spare must never send an old hostname to its new target. The edge names the
    // logical target in this bounded HTTP preface, which is stripped before reaching the app.
    const MAX_HEADER: usize = 64 * 1024;
    let mut input = Vec::new();
    let length = loop {
        if let Some(at) = input.windows(4).position(|p| p == b"\r\n\r\n") {
            if at + 4 > MAX_HEADER {
                return Err(std::io::ErrorKind::InvalidInput.into());
            }
            break at + 4;
        }
        if input.len() >= MAX_HEADER {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        let mut chunk = [0; 8192];
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        input.extend_from_slice(&chunk[..count]);
    };
    let mut fields = [httparse::EMPTY_HEADER; 128];
    let mut request = httparse::Request::new(&mut fields);
    request.parse(&input[..length]).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    let assigned = request
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(crate::contract::FORWARD_HEADER))
        .and_then(|h| std::str::from_utf8(h.value).ok())
        .and_then(|v| v.parse::<u16>().ok());
    if assigned != Some(target) {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    let line = input.windows(2).position(|p| p == b"\r\n").ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut forwarded = input[..line + 2].to_vec();
    for header in request.headers.iter().filter(|h| !h.name.eq_ignore_ascii_case(crate::contract::FORWARD_HEADER)) {
        forwarded.extend_from_slice(header.name.as_bytes());
        forwarded.extend_from_slice(b": ");
        forwarded.extend_from_slice(header.value);
        forwarded.extend_from_slice(b"\r\n");
    }
    forwarded.extend_from_slice(b"\r\n");
    forwarded.extend_from_slice(&input[length..]);
    Ok(forwarded)
}
