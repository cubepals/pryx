<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->
# Kernel network policy for process sandboxes

A process sandbox shares its host's network stack. Where the host lets pryxd attach eBPF
programs to a cgroup of its own, each process sandbox gets its own cgroup and a small set of
programs that decide every destination it dials in the kernel, so it gets the egress policy a
container gets with the host firewall loaded. Where the host can't give it, the guarantee is off
and the process tier is exactly what [process-tier.md](process-tier.md) describes.

## Where it is on

`pryxd probe --json` and `pryx doctor` report the `kernel-network-policy` capability. It is
proved, not read: pryxd makes a disposable child of its own cgroup v2 directory, loads every
program (the kernel's verifier is the safety proof), attaches each, starts a process into the
cgroup, then removes everything. It is on for:

| Host | Cgroup | Pins |
| --- | --- | --- |
| pryxd as root | a child of pryxd's own cgroup | `/sys/fs/bpf/pryx` |
| an ordinary user with CAP_BPF and CAP_NET_ADMIN and a delegated cgroup (systemd `Delegate=yes`) | a child of the delegated cgroup | a `pryx` directory it owns on a BPF filesystem |

It is off, with the reason and what would turn it on, without cgroup v2, without a writable
cgroup (a default `docker run`, an undelegated user service), without CAP_BPF and CAP_NET_ADMIN
(any ordinary user they weren't granted to, the installed root unit, whose capability set is
unchanged), or without a BPF filesystem pryxd can write. Nothing is approximated where it is off.

As root, granting it to a user means: the capabilities on its service
(`AmbientCapabilities=CAP_BPF CAP_NET_ADMIN`), `Delegate=yes`, and
`install -d -o <user> /sys/fs/bpf/pryx` with `chmod o+x /sys/fs/bpf`. pryxd clears its ambient
capabilities for every supervisor it starts, so a sandbox never holds them.

## What a sandbox gets

The same specification as a container's network decides:

| Specification | Reaches |
| --- | --- |
| `network: internet` (public internet on) | Every destination but this host, link-local (169.254.0.0/16, fe80::/10, where clouds serve metadata), multicast, and private addresses on the ports pryxd serves and its API's. DNS reaches the resolvers `/etc/resolv.conf` names. |
| `egress: {allow, deny}` (public internet off, hostname policy) | Only the named hosts, through gateways pryxd serves for that sandbox: TCP 80 and 443 and DNS on 53, from any address, are redirected to them; nothing else leaves. |
| `egressAddresses: {allow, deny}` | Allow adds exactly the prefixes and ports named; deny takes them away. Deny wins. |
| `network: none` | Nothing, as before; the programs refuse it too. |

Always, whatever the specification: a host address (loopback, any address of this host) reaches
only a socket the sandbox itself made, so its own `localhost` works and another sandbox's ports,
the host's services, its agent's neighbours and pryxd's API don't; a request's allow rule can't
name loopback, link-local, multicast or the unspecified ranges. Sockets other than TCP and UDP
over IPv4 and IPv6 (raw, ICMP echo, SCTP, UDP-Lite, MPTCP) are refused at `socket()`.

A sandbox's view reports `host-firewall: true` once its own programs are attached and verified;
a node announces `host-firewall`, `egress-hostnames` and `egress-addresses` for the process tier
only where the probe proved them. A request for hostname or address policy on a host without the
capability is refused with `kernel-network-policy` named; a placement that requires
`host-firewall` is refused naming it.

## How the programs work

One object (`bpf/network.bpf.c`, built by `bpf/build.sh` and embedded in pryxd), nine programs,
each with one job:

| Program | Hook | Job |
| --- | --- | --- |
| `connect4`, `connect6` | `cgroup/connect4`, `connect6` | Allow, refuse (EPERM) or redirect a TCP or UDP connect |
| `sendmsg4`, `sendmsg6` | `cgroup/sendmsg4`, `sendmsg6` | The same for an unconnected datagram's address |
| `recvmsg4`, `recvmsg6` | `cgroup/recvmsg4`, `recvmsg6` | Put back the resolver's address on a redirected DNS answer |
| `sock_create` | `cgroup/sock_create` | Refuse non-TCP/UDP sockets; mark the rest as the sandbox's |
| `ingress`, `egress` | `cgroup_skb/ingress`, `egress` | Count packets and bytes |

pryxd compiles the policy into an LPM trie of IPv6 prefixes (IPv4 as `::ffff:a.b.c.d`, NAT64's
`64:ff9b::/96` taking the rules of the address it embeds). Each prefix carries its own port ranges
and those of every wider prefix, in the order they apply, and a fallback, so a program does one
lookup and takes the first matching range. A verdict is allow, deny, *owned* (a host address:
allowed only when `bpf_sk_lookup` finds a listener the sandbox's socket-storage map holds) or a
gateway (the destination is rewritten to the sandbox's gateway, after checking that pryxd's
listener is there). No program parses anything the sandbox sent.

Programs are attached through cgroup BPF links and pinned, with their maps, under
`<bpffs>/pryx/<key>/`, where `<key>` names the sandbox within one state directory. A restarted
pryxd finds them by their pins, checks each link is the one it attached, writes the rules again
and serves new gateways. When a sandbox's supervisor is gone, pryxd kills whatever is left in its
cgroup, removes the pins, which detaches the programs, and removes the cgroup.

## Reading the counters

`GET /v1/workloads/{id}/stats` reports the sandbox's `networkRxBytes` and `networkTxBytes` from
the `ingress` and `egress` programs, loopback traffic included. With `bpftool` as root, the map
`map-counters` under the sandbox's pin directory holds four per-CPU `{packets, bytes}` slots:
ingress, egress, refused destinations, refused sockets.

```sh
bpftool map dump pinned /sys/fs/bpf/pryx/<key>/map-counters
bpftool map dump pinned /sys/fs/bpf/pryx/<key>/map-rules
bpftool link show pinned /sys/fs/bpf/pryx/<key>/link-connect4
```

## Kernels

The object needs cgroup BPF links (5.7), socket storage in `sock_create` programs and socket
lookups in address hooks, all present on every kernel the process tier supports (Landlock ABI 3,
6.2 and later). It reads only UAPI context fields and carries BTF, so one build loads on every
kernel; CI loads it on the oldest and newest hosted kernels in its matrix and fails if the
verifier rejects a program.
