<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->
# Process isolation

The release image runs the process tier when no usable Docker socket is present. It requires
no host privilege. It has two modes, chosen by the host, never by a request: where the host lets
the sandbox's user create user namespaces, each sandbox starts in namespaces of its own (below);
elsewhere, as in a default `docker run`, it creates no user, mount, PID or network namespace,
exactly as before K26. Independently, where the host delegates cgroup v2 controllers to pryxd,
each sandbox gets a cgroup of its own whose limits the kernel enforces ([Limits](#limits)); where
it doesn't, as in a default `docker run`, pryx writes no cgroup.

## Limits

Where pryxd's cgroup can give its children the `memory` and `cpu` controllers (and `pids`), each
process sandbox runs in a cgroup of its own below it, `pryx-<key>`, the one its network policy's
programs attach to as well ([bpf.md](bpf.md)). Before its supervisor joins it, pryxd writes and
reads back:

| File | From | Means |
| --- | --- | --- |
| `memory.max`, `memory.swap.max` 0 | `resources.memoryMb` | The kernel's hard limit, with no swap beyond it, as a container's. Past it the kernel reclaims, then OOM-kills inside the sandbox's cgroup only |
| `cpu.max` | `resources.cpuMillis`, when asked | A ceiling: that many thousandths of a CPU per 100 ms period |
| `pids.max` | `resources.pidsLimit` or the node's default | Processes and threads in the sandbox, whatever their user |

A sandbox runs with exactly these or not at all. The tier then announces `memory-limit` and
`cpu-limit`, and each sandbox reports what it started with: one started before its host delegated
the controllers reports them off until it starts again. The agent's sampled resident budget is set
past reach there, since summing resident pages counts shared ones more than once and would kill a
sandbox the kernel lets run. Stats come from the cgroup: memory charged, CPU time, throttled
periods and process count. `resources.cpuWeight` stays a container's: it is refused on this tier.

Where they are delegated:

| pryxd runs | Its cgroup |
| --- | --- |
| As the server preset's unit, as root | The unit's own (`Delegate=yes`): memory, cpu and pids |
| As the user preset's unit, an ordinary user | The user service's own (`Delegate=yes`), with what systemd gives users: memory and pids, and cpu from systemd 252 |
| In a default `docker run`, or from a shell | None: off, with the reason |

cgroup v2 lets a cgroup give its children controllers only while it holds no process of its own,
so pryx's processes live in the `pryx-daemon` leaf of that cgroup: both units name it
(`DelegateSubgroup=pryx-daemon`, systemd 254 and later); before that, pryxd moves itself there
when it is its cgroup's only process, and never moves another. Where none of this holds,
`process-limits` in `pryx doctor` and the node's host document is off with what is missing and
what turns it on, the tier announces neither guarantee, memory stays the sampled budget below,
and a request naming `resources.cpuMillis` is refused naming `cpu-limit`. Control places a CPU
ceiling only on a tier announcing `cpu-limit`, and a deployment requiring `memory-limit` only on
one announcing it.

## Two modes

| | Plain | Namespaced |
| --- | --- | --- |
| When | The probe could not start the installation's helper in user namespaces | `process-namespaces` is on in `pryx doctor` |
| PIDs | One host PID view; Landlock keeps other sandboxes' `/proc` unreadable | PID 1 of its own PID namespace; other sandboxes' processes don't exist there (`private-pids`) |
| `/tmp` | Not granted; `TMPDIR` is the data directory's `.tmp` | A private `/tmp` of its own, granted to it alone, holding at most the node's `/tmp` size (`tmp_size_mb`) and never more than the sandbox's memory |
| IPC | The host's | Its own IPC namespace |
| `network: none` | A seccomp and Landlock filter in the shared network | Also an empty network namespace of its own (`private-network`) |
| Landlock, seccomp, limits | As below | The same, installed inside the namespaces |

The namespaced mode runs the installation's unmodified bubblewrap 0.13.1 (`namespaces/bin/bwrap`
beside the agent, LGPL-2.1-or-later, never setuid) as `--unshare-user --unshare-pid --unshare-ipc
--as-pid-1 --cap-drop ALL`, with the host's view bound whole and only `/proc`, `/dev` and `/tmp`
new; `network: none` adds `--unshare-net`, and the node binds the supervisor's listeners in the
host's network and passes them in. The probe is that same launch, run once per agent executable;
a namespaced launch whose helper fails is refused, never started plain. Each sandbox reports what
it started with: `private-pids` in the namespaced mode, `private-network` only there with network
none. A request requiring `private-pids` is placed only on nodes that announce it for the tier.

On Ubuntu 24.04 and later only binaries an AppArmor profile names may use user namespaces.
`pryx doctor` then prints the one profile root installs for this installation's helper and the
`apparmor_parser -r` that loads it; the `user` preset never installs it.

## Isolation
The API reports `isolation: process`; placement refuses a `container` request on this tier.
An image request returns 422 `invalid_request`, field `image`, problem
`this host runs the process tier; images are not available`.

Each sandbox has an agent supervisor and persistent data directory. The default image runs
as container root and allocates separate UIDs/GIDs from the node's configured pool. A fresh
image started with `--user` uses one shared UID and explicitly announces that weaker mode.
Public views also report `guarantees["separate-users"]`; creation can require it, and
placement refuses a shared-UID node. The host's credential-drop probe must succeed before
the uid-pool guarantee is announced. `pryxd doctor` checks Landlock and uses the same state
owner as `serve`, without requiring a container engine. See [host capabilities](host-capabilities.md).
Keep a persistent volume with the same owner when restarting. Changing the container user
does not grant access to a previous user's credentials.

The userland is versioned with pryx: Debian 13, Node 24, Python 3, sh, coreutils, git, curl,
and build-essential. Reviewed build-image digests are in Dockerfile; package versions come
from that Debian release's maintained repositories. Templates snapshot prepared data, not
arbitrary images. Commands start in their data directory. HOME and TMPDIR point there;
absolute `/data` and `/tmp` paths are not remapped into a private namespace.

Landlock ABI 3 or later (Linux 6.2 or later with Landlock enabled) is required. pryx refuses
to start sandbox compute if filesystem policy cannot be fully installed. System userland is
read-only, sandbox data is read-write, and other sandbox data and control credentials are
inaccessible. PTY device access and only the agent's own proc task directory are granted.
The memory sampler is a dedicated trusted thread created before filesystem restriction;
it never executes requests. Untrusted commands inherit the restriction. Their launcher adds a supervisor guard before
executing user code. It denies legacy thread signals, pidfd signals, scheduler changes,
cross-process rlimit access, joining the supervisor's process group, and signals to its
thread group. Legacy kill and queued process signals allow the command's initial PID and separate process groups,
but refuse other positive PIDs because Linux also accepts supervisor thread IDs there.
Use the agent's process signal API for individual child PIDs; guest tgkill remains available
for other thread groups. The trusted supervisor retains its process controls.

The syscall filter denies mount, unmount, pivot_root, chroot, ptrace, cross-process memory
operations, pidfd_getfd descriptor copying, keyctl/add_key/request_key, bpf, kernel modules, unshare/setns, reboot/kexec,
open_by_handle_at/name_to_handle_at, perf_event_open, userfaultfd, io_uring_setup,
swapon/swapoff, UID/GID changes and capability changes. Clone flags creating namespaces are
denied. clone3 returns ENOSYS so libc can use the checked clone fallback. no_new_privs
prevents setuid executables gaining privileges. The existing platform seccomp policy stays active.

Host policy fixes rlimits for virtual address space, descriptors, per-UID processes and CPU
time per process, beside the cgroup's limits where it has them. The virtual ceiling allows V8's
large virtual reservations; it is separate from the resident budget. Nice is a best-effort
process priority, not a per-sandbox CPU weight. `resources.cpuWeight` is refused with 422 before
compute starts, and so is `resources.cpuMillis` where the sandbox's cgroup can't hold it
([Limits](#limits)). Units reserve `cpuReservationMillis` for admission. Without a cgroup's
`memory.max`, the agent samples the entire descendant tree's RSS every 100 ms and kills it when
its memory budget is exceeded. That is not a hard memory cap: allocations can overshoot between
samples, and shared pages may be counted more than once. Admission uses the enclosing
container's readable memory allowance, not unrestricted host RAM. The enclosing platform's own
memory limit still applies.

Existing native specifications naming a CPU ceiling must be recreated as unit/admission-only
specifications before restarting. Explicit hard-limit requests are never silently downgraded
to reservations; persistent data is retained.

Network-off is a filter, not a namespace. Landlock TCP connect filtering is used where
available; the announced seccomp fallback also denies connect, bind, sendto, sendmsg and
sendmmsg in command launchers. The authenticated supervisor's listener remains reachable.
On this mode commands cannot start new application listeners. Internet-enabled processes
share the enclosing network namespace. The process tier has no container network boundary.
Where the host does not grant `kernel-network-policy`, all Internet-enabled process workloads
must therefore be mutually trusted, including when they have separate UIDs: guest code can dial
host and neighboring application listeners.

Where it does (pryxd as root, the server preset's unit included, or an ordinary user with
CAP_BPF, CAP_NET_ADMIN, a delegated cgroup and a BPF filesystem directory of its own), each
process sandbox runs in a cgroup of its own whose eBPF programs decide every destination in the
kernel ([bpf.md](bpf.md)): with public internet on it reaches what a container reaches with the
host firewall loaded, never this host's services (at any address it has, as its addresses
change), another sandbox's ports, link-local or AWS's IPv6 metadata or private addresses on the
served ports; hostname egress (`egress`) goes only through gateways
pryxd serves for that sandbox; address rules (`egressAddresses`) allow or deny exactly the
prefixes and ports named. Its own listeners stay reachable over loopback; from another machine
only its published ports are, as a container's, and a UDP socket hears only peers it sent to.
The sandbox reports `host-firewall` once its programs are attached and verified, and they
survive any daemon restart the sandbox survives; one whose programs can't be verified after a
restart is stopped, and while one started before the host had them runs, the tier reports
`host-firewall` off. A
host without the capability refuses `egress` and `egressAddresses` on this tier, naming it;
nothing is approximated.

The agent's spare-port proxy independently verifies target ownership. A bounded port-only
channel asks a trusted worker, created before Landlock, to find an owned listening socket,
connect, and verify the actual server-side socket's inode and full TCP tuple against the
supervisor's descendant descriptors. PID start identities are checked throughout. No request
bytes are sent before this proof; missing or inaccessible kernel metadata fails closed.
A shared UID, a matching port, or an owned listener on a different address cannot authorize
a foreign peer. This worker never executes guest code and exposes no general filesystem or
network request interface. This protects agent forwarding, not direct guest TCP connections.

Sleep retains memory. A sandbox with a cgroup of its own is frozen whole (`cgroup.freeze`, so
nothing forks past it) and thawed on wake; one without has its tree stopped with SIGSTOP and
continued with SIGCONT. Stop thaws it, sends TERM, then KILL after the configured grace; a kill
of a sandbox with a cgroup is `cgroup.kill`, asleep or not, and whatever is left in its cgroup
once its supervisor is gone is killed before the cgroup is removed. Signals use checked kernel start identities and pidfds.
The supervisor adopts descendants; snapshot, fork, export and restore copy data with the same
safe filesystem walk as the container tier. Private signing material and runtime journals
are excluded. A daemon restart adopts surviving supervisors; a platform container restart
requires booting them again from retained data.

The kernel is the host container's kernel. A kernel escape from one process is an escape
from that container. Separate UIDs improve access control; shared UID mode also shares
per-UID resource limits and permits signaling other sandboxes or same-user services. Use
shared UID mode only for mutually trusted workloads; its supervisor guard protects its own
agent but does not create a separate user identity. The network trust requirement above also
applies to separate-UID process sandboxes. Choose container or Wasm isolation for mutually
untrusted workloads, accounting for the capabilities and shared-worker limits in [tiers.md](tiers.md).

Docker acceptance, kernel facts and n=30 measurements are collected in CI. The development
container rejects the required Landlock policy, so no successful local process-isolation or
Docker acceptance result is claimed. See [speed.md](checks/speed.md) for recorded cohorts and budgets.

The public `/etc/mime.types` file is readable for Python HTTP and MIME lookup. This grants no
read access to the rest of `/etc`; private service credentials remain outside the sandbox rules.
