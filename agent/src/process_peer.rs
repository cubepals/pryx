// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Kernel evidence for process-tier application peers. PID generations and socket inodes
//! establish ownership; a shared UID or a matching port number alone is never sufficient.
//! No request bytes leave the agent until the connected server socket is proven owned.

use crate::ports::FORWARD_CONNECT_TIMEOUT;
use procfs::{
    net::{TcpNetEntry, TcpState},
    process::{FDTarget, Process},
};
use std::{
    collections::HashSet,
    io,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    time::{Duration, Instant},
};

const MAX_PROCESSES: usize = 4096;
const MAX_DESCRIPTORS: usize = 131_072;
const PEER_WAIT: Duration = Duration::from_secs(1);
const PEER_POLL: Duration = Duration::from_millis(5);

fn denied() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "Forward target ownership could not be established")
}

#[derive(Clone, Copy)]
struct Identity {
    pid: i32,
    ticks: u64,
}
impl Identity {
    fn read(pid: i32) -> io::Result<Self> {
        let stat = Process::new(pid).and_then(|p| p.stat()).map_err(|_| denied())?;
        Ok(Self { pid, ticks: stat.starttime })
    }
    fn matches(&self, process: &Process) -> bool {
        process.stat().is_ok_and(|stat| stat.starttime == self.ticks)
    }
}

pub(crate) struct OwnedPeer {
    root: Identity,
}
impl OwnedPeer {
    pub(crate) fn new(pid: u32) -> io::Result<Self> {
        Ok(Self { root: Identity::read(i32::try_from(pid).map_err(|_| denied())?)? })
    }

    fn sockets(&self) -> io::Result<HashSet<u64>> {
        let root = Process::new(self.root.pid).map_err(|_| denied())?;
        if !self.root.matches(&root) {
            return Err(denied());
        }
        let mut pending = vec![self.root];
        let mut seen = HashSet::new();
        let mut owned = HashSet::new();
        let mut descriptors = 0;
        while let Some(identity) = pending.pop() {
            if !seen.insert(identity.pid) {
                continue;
            }
            if seen.len() > MAX_PROCESSES {
                return Err(denied());
            }
            let Ok(process) = Process::new(identity.pid) else { continue };
            if !identity.matches(&process) {
                continue;
            }
            let mut children = Vec::new();
            if let Ok(tasks) = process.tasks() {
                for task in tasks.flatten() {
                    if let Ok(pids) = task.children() {
                        for pid in pids {
                            let Ok(pid) = i32::try_from(pid) else { continue };
                            let Ok(stat) = Process::new(pid).and_then(|p| p.stat()) else { continue };
                            if stat.ppid == identity.pid {
                                children.push(Identity { pid, ticks: stat.starttime });
                            }
                        }
                    }
                }
            }
            let mut sockets = Vec::new();
            if let Ok(fds) = process.fd() {
                for fd in fds.flatten() {
                    descriptors += 1;
                    if descriptors > MAX_DESCRIPTORS {
                        return Err(denied());
                    }
                    if let FDTarget::Socket(inode) = fd.target {
                        sockets.push(inode);
                    }
                }
            }
            // Proc directories are generation-bound too; do not retain observations after exit.
            if identity.matches(&process) {
                owned.extend(sockets);
                pending.extend(children);
            }
        }
        if !self.root.matches(&root) {
            return Err(denied());
        }
        Ok(owned)
    }

    pub(crate) fn connect(&self, port: u16) -> io::Result<TcpStream> {
        if port == 0 {
            return Err(denied());
        }
        let sockets = self.sockets()?;
        if !entries()?.iter().any(|entry| {
            entry.state == TcpState::Listen && entry.local_address.port() == port && sockets.contains(&entry.inode)
        }) {
            return Err(denied());
        }
        let stream =
            TcpStream::connect_timeout(&SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port), FORWARD_CONNECT_TIMEOUT)?;
        let local = canonical(stream.local_addr()?);
        let remote = canonical(stream.peer_addr()?);
        let deadline = Instant::now() + PEER_WAIT;
        loop {
            let sockets = self.sockets()?;
            if entries()?.iter().any(|entry| {
                entry.state == TcpState::Established
                    && canonical(entry.local_address) == remote
                    && canonical(entry.remote_address) == local
                    && entry.inode != 0
                    && sockets.contains(&entry.inode)
            }) {
                stream.set_nodelay(true)?;
                return Ok(stream);
            }
            if Instant::now() >= deadline {
                return Err(denied());
            }
            std::thread::sleep(PEER_POLL);
        }
    }
}

fn canonical(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(address.ip().to_canonical(), address.port())
}
fn entries() -> io::Result<Vec<TcpNetEntry>> {
    let mut entries = procfs::net::tcp().map_err(|_| denied())?;
    match procfs::net::tcp6() {
        Ok(v6) => entries.extend(v6),
        Err(procfs::ProcError::NotFound(_)) => {}
        Err(_) => return Err(denied()),
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        process::{Child, Command, Stdio},
    };

    struct Server(Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn server(address: &str) -> (Server, u16) {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "process_peer::tests::owned_peer_fixture", "--nocapture"])
            .env("PRYX_TEST_PEER_ADDRESS", address)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = BufReader::new(child.stdout.as_mut().unwrap());
        for line in output.lines() {
            let line = line.unwrap();
            if let Some((_, port)) = line.rsplit_once("peer-port=") {
                return (Server(child), port.parse().unwrap());
            }
        }
        let mut child = Server(child);
        panic!("peer fixture exited: {:?}", child.0.try_wait());
    }

    #[test]
    fn owned_peer_fixture() {
        let Ok(address) = std::env::var("PRYX_TEST_PEER_ADDRESS") else { return };
        let listener = TcpListener::bind(address).unwrap();
        println!("peer-port={}", listener.local_addr().unwrap().port());
        std::io::stdout().flush().unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        stream.write_all(&byte).unwrap();
    }

    #[test]
    fn connects_to_owned_child_ipv4_and_dual_stack_ipv6() {
        for address in ["127.0.0.1:0", "[::]:0"] {
            let (mut child, port) = server(address);
            let owner = OwnedPeer::new(child.0.id()).unwrap();
            let mut stream = owner.connect(port).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            stream.write_all(b"x").unwrap();
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            assert_eq!(&byte, b"x");
            assert!(child.0.wait().unwrap().success());
        }
    }

    #[test]
    fn same_uid_foreign_listener_is_refused_before_connect() {
        let (child, _) = server("127.0.0.1:0");
        let foreign = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        foreign.set_nonblocking(true).unwrap();
        let owner = OwnedPeer::new(child.0.id()).unwrap();
        assert_eq!(
            owner.connect(foreign.local_addr().unwrap().port()).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(foreign.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn owned_port_does_not_authorize_a_different_connected_peer() {
        let foreign = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = foreign.local_addr().unwrap().port();
        let (child, _) = server(&format!("127.0.0.2:{port}"));
        let owner = OwnedPeer::new(child.0.id()).unwrap();
        let connect = std::thread::spawn(move || owner.connect(port));
        let (mut peer, _) = foreign.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        assert_eq!(connect.join().unwrap().unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        let mut byte = [0];
        assert_eq!(peer.read(&mut byte).unwrap(), 0, "no application bytes may reach a foreign peer");
    }

    #[test]
    fn exited_supervisor_cannot_authorize_a_peer() {
        let (mut child, port) = server("127.0.0.1:0");
        let owner = OwnedPeer::new(child.0.id()).unwrap();
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert_eq!(owner.connect(port).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
}
