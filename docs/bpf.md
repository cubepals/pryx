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
| pryxd as root, the server preset's `pryxd.service` included | a child of pryxd's own cgroup | `/sys/fs/bpf/pryx` |
| an ordinary user with CAP_BPF and CAP_NET_ADMIN and a delegated cgroup (systemd `Delegate=yes`) | a child of the delegated cgroup | a `pryx` directory it owns on a BPF filesystem |

It is off, with the reason and what would turn it on, without cgroup v2, without a writable
cgroup (a default `docker run`, an undelegated user service), without CAP_BPF and CAP_NET_ADMIN
(any ordinary user they weren't granted to), without a BPF filesystem pryxd can write, or where
the kernel's verifier refuses a program, with the verifier's words. Nothing is approximated
where it is off.

The root unit that the server and node presets install (`deploy/pryxd.service`) grants what it
needs and no more: CAP_BPF and CAP_NET_ADMIN, with CAP_SETUID and CAP_SETGID for the process
tier's uid pool, `Delegate=yes`, and of `/sys` only its own cgroup subtree and
`/sys/fs/bpf/pryx` writable. Process sandboxes live in cgroups below the unit's, so they stop
when pryxd's unit stops, as VMs do with the VM worker; containers keep running. pryxd logs the
proof at each start (`kernel network policy`, with `on` and its evidence). The server preset's
own control plane requires private-network, private-pids, memory-limit and cpu-limit of every
sandbox, which the process tier doesn't give, so it places none there; a control plane a node
joins that places process sandboxes gets them policed.

As root, granting it to a user means: the capabilities on its service
(`AmbientCapabilities=CAP_BPF CAP_NET_ADMIN`), `Delegate=yes`, and
`install -d -o <user> /sys/fs/bpf/pryx` with `chmod o+x /sys/fs/bpf`. pryxd clears its ambient
capabilities for every supervisor it starts, so a sandbox never holds them.

## What a sandbox gets

The same specification as a container's network decides:

| Specification | Reaches |
| --- | --- |
| `network: internet` (public internet on) | Every destination but this host, link-local (169.254.0.0/16, fe80::/10, where clouds serve metadata), multicast, and private addresses on the ports pryxd serves and its API's. DNS reaches the resolvers `/etc/resolv.conf` names. |
| `egress: {allow, deny}` (public internet off, hostname policy) | Only the named hosts, through gateways pryxd serves for that sandbox: TCP 80 and 443 and DNS on 53, from any address, are redirected to them; nothing else leaves. A gateway dials a name only when every address it resolves to is public, none is local to this host as the kernel routes it at that moment, and none is one the sandbox's `egressAddresses.deny` refuses on that port. |
| `egressAddresses: {allow, deny}` | Allow adds exactly the prefixes and ports named; deny takes them away. Deny wins. |
| `network: none` | Nothing, as before; the programs refuse it too. |

Always, whatever the specification: a host address (loopback, any address of this host) reaches
only a socket the sandbox itself made, so its own `localhost` works and another sandbox's ports,
the host's services, its agent's neighbours and pryxd's API don't; a request's allow rule can't
name loopback, link-local, multicast or the unspecified ranges. Sockets other than TCP and UDP
over IPv4 and IPv6 (raw, ICMP echo, SCTP, UDP-Lite, MPTCP) are refused at `socket()`.

From off this host a sandbox is reached as a container is: on the ports pryxd published for it
(its agent's and its spares, where the edge dials it) and nowhere else, so a server it binds to
`0.0.0.0` is reached from this host and through the edge, never directly from another machine.
What arrives from another machine is let in only for TCP to a published port, for a TCP
connection the sandbox dialled off the host (its answers; a socket that dialled and then listened
gets none), or for a UDP socket from a peer, address and port, that socket itself was allowed to
send to; the rest is dropped, so the other machine's connection times out. Each sandbox keeps
its 4096 most recent datagram peers (about 360 KiB of kernel memory); an older peer's datagrams
are dropped until the socket sends to it again. An IPv6 datagram behind extension headers is
dropped too, since the program reads only the fixed IP and UDP headers.

A sandbox's view reports `host-firewall: true` once its own programs are attached and verified;
a node announces `host-firewall`, `egress-hostnames` and `egress-addresses` for the process tier
only where the probe proved them. A request for hostname or address policy on a host without the
capability is refused with `kernel-network-policy` named; a placement that requires
`host-firewall` is refused naming it.

## How the programs work

One object (`bpf/network.bpf.c`, built by `bpf/build.sh` and embedded in pryxd), nine programs,
each with one job, and five maps (rules, config, sockets, peers and counters):

| Program | Hook | Job |
| --- | --- | --- |
| `connect4`, `connect6` | `cgroup/connect4`, `connect6` | Allow, refuse (EPERM) or redirect a TCP or UDP connect |
| `sendmsg4`, `sendmsg6` | `cgroup/sendmsg4`, `sendmsg6` | The same for an unconnected datagram's address |
| `recvmsg4`, `recvmsg6` | `cgroup/recvmsg4`, `recvmsg6` | Put back the resolver's address on a redirected DNS answer |
| `sock_create` | `cgroup/sock_create` | Refuse non-TCP/UDP sockets; mark the rest as the sandbox's |
| `ingress` | `cgroup_skb/ingress` | Drop what arrives from off the host but answers (TCP connections dialled, UDP peers sent to) and published ports; count |
| `egress` | `cgroup_skb/egress` | Count packets and bytes |

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
`map-counters` under the sandbox's pin directory holds five per-CPU `{packets, bytes}` slots:
ingress, egress, refused destinations, refused sockets, refused ingress.

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
verifier rejects a program. `bpf/rejected.bpf.o`, the same source built with `PRYX_REJECTED`,
has a last program the verifier refuses: CI hands it to the probe and checks the guarantee is
off with the verifier's words, nothing of the set stays attached, and a request for address
policy is refused naming the capability.
