// SPDX-License-Identifier: FSL-1.1-ALv2
//! Exercise Linux abstract Unix socket scoping in disposable Landlock domains. This discovers
//! a primitive, not native-tool readiness; it creates no TCP listener or persistent file.
use base64::Engine;
use landlock::{CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetCreated, RulesetStatus, Scope};
use ring::rand::SecureRandom;
use serde::Serialize;
use std::{
    io::{ErrorKind, Read, Write},
    os::{
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr, UnixListener, UnixStream},
    },
    sync::{Arc, OnceLock, mpsc},
    thread,
    time::Duration,
};

const DEADLINE: Duration = Duration::from_secs(2);
static OBSERVATION: OnceLock<IpcProbe> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcProbe {
    pub scope_available: bool,
    pub owner_reachable: Option<bool>,
    pub neighbour_refused: Option<bool>,
    pub completed: bool,
}

impl IpcProbe {
    pub fn supported(&self) -> bool {
        self.scope_available
            && self.completed
            && self.owner_reachable == Some(true)
            && self.neighbour_refused == Some(true)
    }
}

pub fn probe() -> IpcProbe {
    *OBSERVATION.get_or_init(observe)
}

fn ruleset() -> Option<RulesetCreated> {
    Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .scope(Scope::AbstractUnixSocket)
        .ok()?
        .create()
        .ok()
}

fn confined() -> bool {
    ruleset()
        .and_then(|rules| rules.restrict_self().ok())
        .is_some_and(|status| status.ruleset == RulesetStatus::FullyEnforced)
}

fn observe() -> IpcProbe {
    if ruleset().is_none() {
        return IpcProbe::default();
    }
    let mut result = IpcProbe { scope_available: true, ..Default::default() };
    let Some(address) = address() else { return result };
    let address = Arc::new(address);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let owner_address = address.clone();
    let Ok(owner) = thread::Builder::new().name("pryx-ipc-owner".into()).spawn(move || {
        let listener = if confined() { owned_socket(&owner_address) } else { None };
        let reachable = listener.is_some();
        let _ = ready_tx.send(reachable);
        // Keep both the creator domain and its listener alive throughout the neighbour attempt.
        reachable && finish_rx.recv_timeout(DEADLINE).is_ok()
    }) else {
        return result;
    };
    result.owner_reachable = ready_rx.recv_timeout(DEADLINE).ok();
    if result.owner_reachable == Some(true) {
        result.neighbour_refused = thread::Builder::new()
            .name("pryx-ipc-neighbour".into())
            .spawn(move || {
                if !confined() {
                    return None;
                }
                match UnixStream::connect_addr(&address) {
                    Err(error) if error.kind() == ErrorKind::PermissionDenied => Some(true),
                    Ok(_) => Some(false),
                    Err(_) => None,
                }
            })
            .ok()
            .and_then(|worker| worker.join().ok())
            .flatten();
    }
    let _ = finish_tx.send(());
    result.completed = owner.join().unwrap_or(false);
    result
}

fn address() -> Option<SocketAddr> {
    let mut nonce = [0; 24];
    ring::rand::SystemRandom::new().fill(&mut nonce).ok()?;
    let name = format!("pryx-ipc-probe-{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(nonce));
    SocketAddr::from_abstract_name(name).ok()
}

fn owned_socket(address: &SocketAddr) -> Option<UnixListener> {
    let listener = UnixListener::bind_addr(address).ok()?;
    let mut client = UnixStream::connect_addr(address).ok()?;
    client.set_write_timeout(Some(DEADLINE)).ok()?;
    client.write_all(b"own!").ok()?;
    let mut server = listener.accept().ok()?.0;
    server.set_read_timeout(Some(DEADLINE)).ok()?;
    let mut bytes = [0; 4];
    server.read_exact(&mut bytes).ok()?;
    (bytes == *b"own!").then_some(listener)
}
