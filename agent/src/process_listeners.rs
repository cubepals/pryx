// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Listening sockets the node bound for a process supervisor before starting it in an empty
//! network namespace of its own. A socket belongs to the namespace it was made in, so these keep
//! answering on the host's addresses while every socket the sandbox makes is in its own.
//!
//! The node names each descriptor with its address in `LISTENERS_ENV`. `adopt` runs first in the
//! executable: it takes each one out of reach of every later exec, checks it is a listening TCP
//! socket bound to exactly that address, and removes the variable. With the variable present, a
//! listener is only ever one of these: a supervisor in its own namespace never binds there,
//! where nothing could reach it. Without it, binding is unchanged. Not for container or VM
//! agents, which listen inside their own network.
use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::sync::Mutex;

/// The variable naming inherited listeners: a JSON list of `{"address": "ip:port", "fd": n}`.
pub const LISTENERS_ENV: &str = "PRYX_PROCESS_LISTENERS";

#[derive(serde::Deserialize, serde::Serialize)]
pub struct Inherited {
    pub address: SocketAddr,
    pub fd: RawFd,
}

/// `None`: the supervisor binds its own listeners. `Some`: only the adopted ones exist.
static ADOPTED: Mutex<Option<BTreeMap<SocketAddr, OwnedFd>>> = Mutex::new(None);

/// Takes the listeners the node passed, before anything can exec with them open.
#[expect(unsafe_code, reason = "Owning descriptors the node passed across exec, once, before any thread")]
pub fn adopt() -> io::Result<()> {
    let Some(encoded) = std::env::var_os(LISTENERS_ENV) else { return Ok(()) };
    // SAFETY: the bootstrap is still single-threaded; no other thread reads the environment.
    unsafe { std::env::remove_var(LISTENERS_ENV) };
    let listed: Vec<Inherited> = serde_json::from_slice(encoded.as_encoded_bytes()).map_err(io::Error::other)?;
    let mut adopted = BTreeMap::new();
    for item in listed {
        if item.fd <= 2 || adopted.contains_key(&item.address) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid inherited listener"));
        }
        let flags = rustix::io::fcntl_getfd(unsafe_borrow(item.fd)).map_err(io::Error::from)?;
        rustix::io::fcntl_setfd(unsafe_borrow(item.fd), flags | rustix::io::FdFlags::CLOEXEC)
            .map_err(io::Error::from)?;
        // SAFETY: the node passed this descriptor for this process alone, and it is taken once.
        let fd = unsafe { OwnedFd::from_raw_fd(item.fd) };
        let listening = rustix::net::sockopt::socket_acceptconn(&fd).map_err(io::Error::from)?;
        let tcp = rustix::net::sockopt::socket_type(&fd).map_err(io::Error::from)? == rustix::net::SocketType::STREAM;
        let bound = std::net::TcpListener::from(fd.try_clone()?).local_addr()?;
        if !listening || !tcp || bound != item.address {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Inherited listener is not the named address"));
        }
        adopted.insert(item.address, fd);
    }
    *ADOPTED.lock().unwrap() = Some(adopted);
    Ok(())
}

#[expect(unsafe_code, reason = "Reading descriptor flags before taking ownership")]
fn unsafe_borrow(fd: RawFd) -> std::os::fd::BorrowedFd<'static> {
    // SAFETY: the descriptor stays open until `adopt` owns it a few lines later.
    unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) }
}

/// The listener for `ip:port`: the adopted one where the node passed listeners, else a new bind.
pub async fn listen(ip: IpAddr, port: u16) -> io::Result<tokio::net::TcpListener> {
    let address = SocketAddr::new(ip, port);
    let taken = {
        let mut adopted = ADOPTED.lock().unwrap();
        match adopted.as_mut() {
            None => None,
            Some(held) => Some(held.remove(&address).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "The node passed no listener for this address")
            })?),
        }
    };
    match taken {
        Some(fd) => {
            let listener = std::net::TcpListener::from(fd);
            listener.set_nonblocking(true)?;
            tokio::net::TcpListener::from_std(listener)
        }
        None => tokio::net::TcpListener::bind(address).await,
    }
}
