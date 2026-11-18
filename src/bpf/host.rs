// SPDX-License-Identifier: FSL-1.1-ALv2
//! Where this daemon may keep a process sandbox's programs: a child of its own cgroup v2
//! directory for each sandbox, and a directory on a BPF filesystem it alone writes for their
//! pins. Also the host facts a policy needs: this host's own addresses and the broadcast
//! prefixes its routes also deliver here, and the kernel's word each time they change. Found by looking, never configured; what is missing is reported with
//! what would turn it on. Not the programs: those are `attach.rs`.
use std::{
    fs, io,
    net::IpAddr,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Command,
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

/// Why a host can't give the guarantee, and what would.
#[derive(Debug)]
pub(crate) struct Unavailable {
    pub(crate) evidence: String,
    pub(crate) action: &'static str,
}

fn unavailable(evidence: impl Into<String>, action: &'static str) -> Unavailable {
    Unavailable { evidence: evidence.into(), action }
}

/// One mount: where, of what type, and which part of its filesystem it shows.
struct Mount {
    root: PathBuf,
    point: PathBuf,
    kind: String,
}

/// mountinfo escapes space, tab, newline and backslash as three octal digits.
fn unescape(field: &str) -> PathBuf {
    let mut bytes = Vec::with_capacity(field.len());
    let raw = field.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\\'
            && let Some(value) = field.get(i + 1..i + 4).and_then(|digits| u8::from_str_radix(digits, 8).ok())
        {
            bytes.push(value);
            i += 4;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes))
}

fn mounts(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let (before, after) = line.split_once(" - ")?;
            let fields: Vec<&str> = before.split(' ').collect();
            Some(Mount {
                root: unescape(fields.get(3)?),
                point: unescape(fields.get(4)?),
                kind: after.split(' ').next()?.to_owned(),
            })
        })
        .collect()
}

/// This process's cgroup v2 directory, from its cgroup path and where that hierarchy is mounted.
fn own_cgroup(cgroup: &str, mountinfo: &str) -> Result<PathBuf, Unavailable> {
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::")).ok_or_else(|| {
        unavailable("this process has no cgroup v2 membership", "Boot with the unified cgroup v2 hierarchy")
    })?;
    let path = Path::new(path.trim());
    mounts(mountinfo)
        .into_iter()
        .filter(|mount| mount.kind == "cgroup2")
        .find_map(|mount| {
            let below = path.strip_prefix(&mount.root).ok()?;
            Some(mount.point.join(below))
        })
        .ok_or_else(|| {
            unavailable(
                format!("cgroup {} is outside every mounted cgroup v2 hierarchy", path.display()),
                "Mount the cgroup v2 hierarchy this daemon's cgroup belongs to",
            )
        })
}

/// A directory this process may make entries in.
fn writable(path: &Path) -> bool {
    rustix::fs::access(path, rustix::fs::Access::WRITE_OK | rustix::fs::Access::EXEC_OK).is_ok()
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

/// A sandbox's cgroup, made if missing. A leftover one from a lost run is reused once empty.
pub(crate) fn create_cgroup(path: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o755).create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if members(path)?.is_empty() {
                Ok(())
            } else {
                Err(io::Error::other(format!("cgroup {} still holds processes", path.display())))
            }
        }
        Err(error) => Err(error),
    }
}

/// The processes in a cgroup now.
pub(crate) fn members(path: &Path) -> io::Result<Vec<u32>> {
    Ok(fs::read_to_string(path.join("cgroup.procs"))?.lines().filter_map(|pid| pid.parse().ok()).collect())
}

/// Stops every task and leaves the cgroup frozen, so a late join cannot run after the kill.
/// The programs remain attached until every member is gone.
pub(crate) fn fence_cgroup(path: &Path) -> io::Result<()> {
    let frozen = fs::write(path.join("cgroup.freeze"), b"1");
    let killed = fs::write(path.join("cgroup.kill"), b"1");
    frozen.and(killed)
}

/// Waits before detaching a stopped cgroup's programs. A task that hasn't exited keeps them.
pub(crate) fn wait_empty(path: &Path) -> io::Result<()> {
    const STOP: std::time::Duration = std::time::Duration::from_secs(5);
    let deadline = std::time::Instant::now() + STOP;
    while !members(path)?.is_empty() {
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::other(format!("cgroup {} still holds stopped processes", path.display())));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(())
}

/// Removes an empty cgroup; a missing one is already gone. Exiting processes can hold it for a
/// moment, so it is tried a few times.
pub(crate) fn remove_cgroup(path: &Path) -> io::Result<()> {
    const ATTEMPTS: u32 = 50;
    for _ in 0..ATTEMPTS {
        match fs::remove_dir(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) if error.raw_os_error() == Some(libc::EBUSY) => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            Err(error) => return Err(error),
        }
    }
    fs::remove_dir(path)
}

/// Makes `command`'s process join `cgroup` before it runs anything, so every socket it opens is
/// the cgroup's. The descriptor is opened here, so the kernel checks this daemon's right to move
/// it even after the child has become its sandbox's user.
pub(crate) fn join_on_exec(command: &mut Command, cgroup: &Path) -> io::Result<()> {
    use std::os::unix::process::CommandExt;
    let procs = fs::OpenOptions::new().write(true).open(cgroup.join("cgroup.procs"))?;
    // SAFETY: between fork and exec the closure makes one write(2) to a descriptor it owns, which
    // is async-signal-safe and touches no lock or allocation.
    #[expect(unsafe_code, reason = "joining a cgroup between fork and exec needs pre_exec")]
    unsafe {
        command.pre_exec(move || {
            rustix::io::write(&procs, b"0")?;
            Ok(())
        });
    }
    Ok(())
}

/// Keeps this daemon's ambient capabilities (CAP_BPF and CAP_NET_ADMIN where it was granted them
/// to attach programs) from passing to `command`, whose process then runs a sandbox's code.
pub(crate) fn drop_ambient_on_exec(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: between fork and exec the closure makes one prctl(2), which is async-signal-safe
    // and touches no lock or allocation.
    #[expect(unsafe_code, reason = "clearing the ambient set between fork and exec needs pre_exec")]
    unsafe {
        command.pre_exec(|| {
            rustix::thread::clear_ambient_capability_set()?;
            Ok(())
        });
    }
}

/// What a policy keeps this host to, as the kernel routes it now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Addresses {
    /// This host's own addresses.
    pub(crate) own: Vec<IpAddr>,
    /// The IPv4 prefixes routed as broadcast (a subnet's last address, loopback's): a datagram
    /// sent there reaches this host's own sockets too, from the link rather than from loopback.
    pub(crate) broadcast: Vec<ipnet::IpNet>,
}

/// This host's own addresses and broadcast prefixes: IPv4 from the routing trie, IPv6 from the
/// interface table. An unreadable table is no proof that the host owns no address: it leaves the
/// policy unavailable.
pub(crate) fn addresses() -> io::Result<Addresses> {
    addresses_at(Path::new("/proc/net"))
}

fn addresses_at(directory: &Path) -> io::Result<Addresses> {
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
    let mut own: Vec<IpAddr> = crate::infer::local_addresses(&ipv4).into_iter().map(IpAddr::V4).collect();
    own.extend(ipv6_addresses(&ipv6).into_iter().map(IpAddr::V6));
    own.sort_unstable();
    own.dedup();
    let mut broadcast: Vec<ipnet::IpNet> =
        crate::infer::broadcast_routes(&ipv4).into_iter().map(ipnet::IpNet::V4).collect();
    broadcast.sort_unstable();
    broadcast.dedup();
    Ok(Addresses { own, broadcast })
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

    const MOUNTINFO: &str = "\
25 30 0:22 / /sys rw,nosuid shared:7 - sysfs sysfs rw
29 25 0:26 / /sys/fs/cgroup rw,nosuid shared:9 - cgroup2 cgroup2 rw,nsdelegate
33 25 0:30 / /sys/fs/bpf rw,nosuid shared:12 - bpf bpf rw,mode=700
90 30 0:40 / /run/pryx\\040bpf rw shared:50 - bpf none rw
";

    #[test]
    fn the_own_cgroup_is_the_v2_path_below_its_mount() {
        assert_eq!(
            own_cgroup("0::/user.slice/user-1001.slice/user@1001.service/app.slice/pryxd.service\n", MOUNTINFO)
                .unwrap(),
            Path::new("/sys/fs/cgroup/user.slice/user-1001.slice/user@1001.service/app.slice/pryxd.service")
        );
        let namespaced = "40 30 0:26 /system.slice/docker-1.scope /sys/fs/cgroup ro - cgroup2 cgroup2 rw\n";
        assert_eq!(
            own_cgroup("0::/system.slice/docker-1.scope/x\n", namespaced).unwrap(),
            Path::new("/sys/fs/cgroup/x")
        );
        assert!(own_cgroup("1:name=systemd:/\n", MOUNTINFO).is_err());
        assert!(own_cgroup("0::/a\n", "25 30 0:22 / /sys rw - sysfs sysfs rw\n").is_err());
    }

    #[test]
    fn mount_points_are_unescaped() {
        let points: Vec<_> = mounts(MOUNTINFO).into_iter().filter(|m| m.kind == "bpf").map(|m| m.point).collect();
        assert_eq!(points, [PathBuf::from("/sys/fs/bpf"), PathBuf::from("/run/pryx bpf")]);
    }

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
        let trie = "Main:\n  +-- 203.0.113.0/24 2 0 2\n     |-- 203.0.113.9\n        /32 host LOCAL\n\
                    |-- 203.0.113.255\n        /32 link BROADCAST\n";
        fs::write(&ipv4, trie).unwrap();
        fs::write(&ipv6, "20010db8000000000000000000000009 01 00 00 80 lo\n").unwrap();
        let own: Vec<IpAddr> = ["203.0.113.9", "2001:db8::9"].map(|address| address.parse().unwrap()).into();
        let broadcast = vec!["203.0.113.255/32".parse().unwrap()];
        assert_eq!(addresses_at(directory.path()).unwrap(), Addresses { own, broadcast });
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
            assert_eq!(addresses_at(directory.path()).unwrap(), Addresses::default());
        } else {
            assert_eq!(addresses_at(directory.path()).unwrap_err().kind(), io::ErrorKind::NotFound);
        }
    }
}
