<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->
# Process isolation

The release image runs the process tier when no usable Docker socket is present. It creates
no user, mount, PID or network namespace, writes no cgroup, and requires no host privilege.
The API reports `isolation: process`; placement refuses a `container` request on this tier.
An image request returns 422 `invalid_request`, field `image`, problem
`this host runs the process tier; images are not available`.

Each sandbox has an agent supervisor and persistent data directory. The default image runs
as container root and allocates separate UIDs/GIDs from the node's configured pool. A fresh
image started with `--user` uses one shared UID and explicitly announces that weaker mode.
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
time per process, not an aggregate tree CPU allowance. The virtual ceiling allows V8's large
virtual reservations; it is separate from the resident budget. Nice is a best-effort process
priority, not a per-sandbox CPU weight or quota. A process spec requesting `resources.cpuMillis`
or `resources.cpuWeight` is refused with 422 before compute starts. Use a container host for
those controls. Units reserve `cpuReservationMillis` for admission; stats report no enforced
CPU ceiling for this tier. The agent samples the
entire descendant tree's RSS every 100 ms and kills it when its memory budget is exceeded.
This is not a hard memory cap: allocations can overshoot between samples, and shared pages
may be counted more than once. Admission uses the enclosing container's readable memory
allowance, not unrestricted host RAM. The enclosing platform's own memory limit still applies.

Existing native specifications naming a CPU ceiling must be recreated as unit/admission-only
specifications before restarting. Explicit hard-limit requests are never silently downgraded
to reservations; persistent data is retained.

Network-off is a filter, not a namespace. Landlock TCP connect filtering is used where
available; the announced seccomp fallback also denies connect, bind, sendto, sendmsg and
sendmmsg in command launchers. The authenticated supervisor's listener remains reachable.
On this mode commands cannot start new application listeners. Internet-enabled processes
share the enclosing network namespace. The process tier has no container network boundary.
All Internet-enabled process workloads must therefore be mutually trusted, including when
they have separate UIDs: guest code can dial host and neighboring application listeners.

The agent's spare-port proxy independently verifies target ownership. A bounded port-only
channel asks a trusted worker, created before Landlock, to find an owned listening socket,
connect, and verify the actual server-side socket's inode and full TCP tuple against the
supervisor's descendant descriptors. PID start identities are checked throughout. No request
bytes are sent before this proof; missing or inaccessible kernel metadata fails closed.
A shared UID, a matching port, or an owned listener on a different address cannot authorize
a foreign peer. This worker never executes guest code and exposes no general filesystem or
network request interface. This protects agent forwarding, not direct guest TCP connections.

Sleep stops the tree with SIGSTOP and retains memory. Wake sends SIGCONT. Stop sends TERM,
then KILL after the configured grace. Signals use checked kernel start identities and pidfds.
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
