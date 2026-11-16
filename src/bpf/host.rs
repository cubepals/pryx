// SPDX-License-Identifier: FSL-1.1-ALv2
//! Where this daemon may keep a process sandbox's programs: a child of its own cgroup v2
//! directory for each sandbox, and a directory on a BPF filesystem it alone writes for their
//! pins. Also the host facts a policy needs: this host's own addresses, and the kernel's word
//! each time they change. Found by looking, never configured; what is missing is reported with
//! what would turn it on. Not the programs: those are `attach.rs`.
#[cfg(test)]
pub(crate) use crate::cgroup::members;
pub(crate) use crate::cgroup::{
    Unavailable, create_cgroup, drop_ambient_on_exec, fence_cgroup, join_on_exec, remove_cgroup, wait_empty,
};
use crate::cgroup::{mounts, own_cgroup, unavailable, writable};
use std::{
    fs, io,
    net::IpAddr,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

/// The root of a BPF filesystem as systemd mounts it.
const BPF_FILESYSTEM: &str = "/sys/fs/bpf";
/// The directory this daemon owns there.
const PINS: &str = "pryx";

pub(crate) struct Host {
    /// This daemon's own cgroup v2 directory; each sandbox gets a child of it.
    pub(crate) cgroups: PathBuf,
    /// A directory on a BPF filesystem; each sandbox's pins go in a child of it.
    pub(crate) pins: PathBuf,
    /// Whether the cgroup was delegated to this ordinary user, not reached as root.
    pub(crate) delegated: bool,
}

/// The first BPF filesystem this daemon may write its own directory on: root's at /sys/fs/bpf,
/// else one mounted for this user.
fn pin_root(mountinfo: &str) -> Result<PathBuf, Unavailable> {
    let mut points: Vec<PathBuf> =
        mounts(mountinfo).into_iter().filter(|mount| mount.kind == "bpf").map(|mount| mount.point).collect();
    points.sort_by_key(|point| point != Path::new(BPF_FILESYSTEM));
    points.dedup();
    for point in points {
        let pins = point.join(PINS);
        if pins.is_dir() && writable(&pins) {
            return Ok(pins);
        }
        if writable(&point) {
            match fs::DirBuilder::new().mode(0o700).create(&pins) {
                Ok(()) => return Ok(pins),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && writable(&pins) => return Ok(pins),
                Err(_) => continue,
            }
        }
    }
    Err(unavailable(
        "no BPF filesystem this daemon can write is mounted",
        "As root, mount a BPF filesystem this daemon's user owns, or run it as root",
    ))
}

impl Host {
    /// This host's cgroup and BPF filesystem, if this daemon may write both.
    pub(crate) fn find() -> Result<Self, Unavailable> {
        let read = |path: &str| {
            fs::read_to_string(path)
                .map_err(|error| unavailable(format!("{path}: {error}"), "Run on Linux with /proc mounted"))
        };
        let mountinfo = read("/proc/self/mountinfo")?;
        let cgroups = own_cgroup(&read("/proc/self/cgroup")?, &mountinfo)?;
        let delegated = !rustix::process::geteuid().is_root();
        if !writable(&cgroups)
            || rustix::fs::access(cgroups.join("cgroup.procs"), rustix::fs::Access::WRITE_OK).is_err()
        {
            return Err(unavailable(
                format!("cgroup {} is not writable by this daemon", cgroups.display()),
                "Run as root, or delegate this daemon's cgroup to its user (systemd Delegate=yes)",
            ));
        }
        let pins = pin_root(&mountinfo)?;
        Ok(Self { cgroups, pins, delegated })
    }

    pub(crate) fn cgroup(&self, key: &str) -> PathBuf {
        self.cgroups.join(format!("pryx-{key}"))
    }

    pub(crate) fn pin_directory(&self, key: &str) -> PathBuf {
        self.pins.join(key)
    }

    pub(crate) fn describe(&self) -> String {
        format!(
            "cgroup v2 {} ({}); pins in {}",
            self.cgroups.display(),
            if self.delegated { "delegated to this user" } else { "as root" },
            self.pins.display()
        )
    }
}

/// This host's own addresses: IPv4 from the routing trie, IPv6 from the interface table. An
/// unreadable table is no proof that the host owns no address: it leaves the policy unavailable.
pub(crate) fn addresses() -> io::Result<Vec<IpAddr>> {
    addresses_at(Path::new("/proc/net"))
}

fn addresses_at(directory: &Path) -> io::Result<Vec<IpAddr>> {
    let read = |file: &str| {
        let path = directory.join(file);
        fs::read_to_string(&path)
            .map_err(|problem| io::Error::new(problem.kind(), format!("reading {}: {problem}", path.display())))
    };
    let ipv4 = read("fib_trie")?;
    let ipv6 = match read("if_inet6") {
        Ok(table) => table,
        // Linux exposes no IPv6 interface table when that address family is absent. Prove
        // that absence with a socket; a masked or denied proc file must not hide live addresses.
        Err(problem) if problem.kind() == io::ErrorKind::NotFound && ipv6_unavailable() => String::new(),
        Err(problem) => return Err(problem),
    };
    let mut found: Vec<IpAddr> = crate::infer::local_addresses(&ipv4).into_iter().map(IpAddr::V4).collect();
    found.extend(ipv6_addresses(&ipv6).into_iter().map(IpAddr::V6));
    found.sort_unstable();
    found.dedup();
    Ok(found)
}

fn ipv6_unavailable() -> bool {
    use rustix::{
        io::Errno,
        net::{AddressFamily, SocketFlags, SocketType},
    };
    matches!(
        rustix::net::socket_with(AddressFamily::INET6, SocketType::DGRAM, SocketFlags::CLOEXEC, None),
        Err(Errno::AFNOSUPPORT)
    )
}

/// The kernel's word that this host's addresses may have changed: a netlink socket in the groups
/// rtnetlink tells of every IPv4 and IPv6 address added or removed, and of IPv4 routes, since an
/// IPv4 address is read from the local routing table, whose route the kernel adds just after it
/// tells of the address. Nothing is read from the messages; `addresses` is read again.
pub(crate) struct AddressChanges(std::os::fd::OwnedFd);

/// What lets this daemon hear of address changes where it can't.
pub(crate) const WATCH_ACTION: &str =
    "Allow this daemon netlink sockets (systemd's RestrictAddressFamilies with AF_NETLINK)";

/// The subscribed socket; needs no privilege, only a netlink socket (systemd's
/// RestrictAddressFamilies must allow AF_NETLINK). The probe opens one to prove it.
pub(crate) fn subscribe() -> io::Result<AddressChanges> {
    use rustix::net::{AddressFamily, SocketFlags, SocketType, netlink::SocketAddrNetlink};
    let groups = libc::RTMGRP_IPV4_IFADDR | libc::RTMGRP_IPV6_IFADDR | libc::RTMGRP_IPV4_ROUTE;
    // No protocol is NETLINK_ROUTE, rtnetlink's.
    let socket = rustix::net::socket_with(AddressFamily::NETLINK, SocketType::RAW, SocketFlags::CLOEXEC, None)?;
    rustix::net::bind(&socket, &SocketAddrNetlink::new(0, groups.cast_unsigned()))?;
    Ok(AddressChanges(socket))
}

impl AddressChanges {
    /// Blocks until the kernel tells of a change, then takes every message already queued, so a
    /// burst is one change. A queue that overflowed (ENOBUFS) lost messages, so it is a change too.
    pub(crate) fn changed(&self) -> io::Result<()> {
        use rustix::{io::Errno, net::RecvFlags};
        // A message is a few hundred bytes; any buffer takes one per read.
        let mut message = [0u8; 4096];
        let mut waiting = RecvFlags::empty();
        loop {
            match rustix::net::recv(&self.0, &mut message, waiting) {
                Ok(_) | Err(Errno::NOBUFS) => waiting = RecvFlags::DONTWAIT,
                Err(Errno::AGAIN) if waiting == RecvFlags::DONTWAIT => return Ok(()),
                Err(Errno::INTR) => {}
                Err(problem) => return Err(problem.into()),
            }
        }
    }
}

/// /proc/net/if_inet6: each line starts with an address as 32 hex digits.
fn ipv6_addresses(table: &str) -> Vec<std::net::Ipv6Addr> {
    table
        .lines()
        .filter_map(|line| u128::from_str_radix(line.split_whitespace().next()?, 16).ok())
        .map(std::net::Ipv6Addr::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CI runs it as the runner's own user; tests/process_network.rs adds an address as root.
    #[test]
    fn address_changes_are_heard_without_privilege() {
        subscribe().expect("rtnetlink's address and route groups need no privilege");
    }

    #[test]
    fn interface_addresses_are_read_as_ipv6() {
        let table = "00000000000000000000000000000001 01 80 10 80       lo\n\
                     fe800000000000000a0027fffe12f2b4 02 40 20 80     eth0\n";
        assert_eq!(
            ipv6_addresses(table),
            ["::1".parse::<std::net::Ipv6Addr>().unwrap(), "fe80::a00:27ff:fe12:f2b4".parse().unwrap()]
        );
    }

    #[test]
    fn unreadable_address_tables_are_refused_instead_of_an_empty_host() {
        let directory = tempfile::tempdir().unwrap();
        let ipv4 = directory.path().join("fib_trie");
        let ipv6 = directory.path().join("if_inet6");
        fs::write(&ipv4, "Main:\n  +-- 203.0.113.0/24 2 0 2\n     |-- 203.0.113.9\n        /32 host LOCAL\n").unwrap();
        fs::write(&ipv6, "20010db8000000000000000000000009 01 00 00 80 lo\n").unwrap();
        let expected: Vec<IpAddr> = ["203.0.113.9", "2001:db8::9"].map(|address| address.parse().unwrap()).into();
        assert_eq!(addresses_at(directory.path()).unwrap(), expected);
        fs::remove_file(&ipv4).unwrap();
        let problem = addresses_at(directory.path()).unwrap_err();
        assert_eq!(problem.kind(), io::ErrorKind::NotFound);
        assert!(problem.to_string().contains("fib_trie"), "{problem}");
        fs::write(&ipv4, "Main:\n").unwrap();
        // A present but unreadable IPv6 table is always an error, even without IPv6 sockets.
        fs::remove_file(&ipv6).unwrap();
        fs::create_dir(&ipv6).unwrap();
        let problem = addresses_at(directory.path()).unwrap_err();
        assert!(problem.to_string().contains("if_inet6"), "{problem}");
        fs::remove_dir(&ipv6).unwrap();
        if ipv6_unavailable() {
            assert_eq!(addresses_at(directory.path()).unwrap(), Vec::<IpAddr>::new());
        } else {
            assert_eq!(addresses_at(directory.path()).unwrap_err().kind(), io::ErrorKind::NotFound);
        }
    }
}
