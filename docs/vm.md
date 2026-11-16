<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Virtual machines

The `vm` tier runs each sandbox as a virtual machine with a Linux kernel of its own, made from
an ordinary OCI image. [Firecracker](https://github.com/firecracker-microvm/firecracker) is the
monitor and KVM runs the guest. The sandbox's agent runs inside the guest and serves exec,
files, logs, stats and ports as on every other tier.

Ask for it by name, `isolation: "vm"` in the SDKs or `pryx run --isolation vm --image alpine
-- uname -r`, or require one of its guarantees. A `vm` request is placed only on a node that
announces the tier; where none does, it is refused with `no_room` naming `vm`. It is never run
on another tier. A request that leaves the tier open and has an image goes to a container first,
then a VM, in control's default order ([tiers](tiers.md)).

A VM sandbox reports these [guarantees](host-capabilities.md):

| Guarantee | On a VM | Why |
| --- | --- | --- |
| `own-kernel` | Yes | The guest boots its own kernel, Linux 6.18.56 |
| `memory-limit` | Yes | The guest's RAM is the admitted memory |
| `cpu-limit` | Yes | The guest has exactly the admitted number of vCPUs |
| `private-network` | Yes | The guest has its own network stack, or none |
| `private-pids` | Yes | The guest kernel sees only its own processes |
| `memory-sleep` | As an ordinary user, once its start test passes | Sleep writes the guest's memory beside its disk; wake restores it on a host of the same class |
| `separate-users`, `host-firewall`, `browser-sandbox` | No | These name process and container mechanisms |

## What a host needs

`pryx doctor` prints `tier.vm` as on or off, with the reason and what would change it. The tier
is on when all of these hold:

- **x86_64 Linux.** The published monitor and kernel are x86_64 only. Images must be
  `linux/amd64`.
- **KVM this user can open.** `/dev/kvm` opens read/write for pryxd's user, and a VM can be
  created on it: the probe runs a one-page guest that halts, and pryxd creates a VM at start. A
  device file alone is not enough. Nothing falls back to emulation.
- **An ordinary user, or root's VM worker.** pryxd running as root serves the tier only through
  the separate `pryxd-vm-worker.service`; without it, a container's root included, the tier is
  off with "The root VM worker is not serving". See [As root](#as-root).
- **Landlock ABI 3,** fully enforced, as for the process tier: it confines the monitor.
- **A signed installation.** It carries `vm-bundle.json`, a small read-only descriptor, beside
  its `pryx-agent`, and pryxd finds it there; `[vm] bundle = "<path>"` names another place. The
  descriptor and agent must be owned by pryxd's user or root, in directories no other user can
  write. A build from source has no descriptor and leaves the tier off.

The descriptor pins one release asset by size, SHA-256 and inventory. The asset is fetched on
first use and cached under pryxd's state directory; installing asks nothing more. Every file in
it is checked against the inventory (size, digest and mode) before any of its programs runs. A
mismatch refuses the work and runs nothing.

### What the bundle holds

| Part | Version and source | Licence | Does |
| --- | --- | --- | --- |
| Monitor | Firecracker 1.18.0, built by the release recipe from its source at `eaf83bb` | Apache-2.0 | Runs the guest on KVM |
| Jailer | Firecracker 1.18.0's published `jailer` | Apache-2.0 | Confines the monitor as root |
| Kernel | Kata Containers 4.3.1's published `vmlinux-6.18.56-208`, unchanged | GPL-2.0 | The guest's kernel |
| Filesystem tools | Alpine 3.24's erofs-utils 1.9.1 and e2fsprogs 1.47.4, unchanged | GPL-2.0, with LGPL, BSD and MIT parts | Build EROFS layers and ext4 disks |
| Packet helper | gvisor-tap-vsock 0.9.0 in pryx's `network-proxy` | Apache-2.0 | The ordinary user's guest network |
| Guest firewall | nftables 1.1.6, in the read-only bootstrap image | GPL-2.0 | Guest rules set before image code |
| Root TAP | CNI plugins 1.10.0's `tap` | Apache-2.0, one MPL-2.0 dependency | Root's TAP device |
| Guest init and agent | The static `pryx-agent`, also the host launcher | FSL-1.1-ALv2 | PID 1 and the sandbox supervisor |

Each licence was recorded in [decisions](decisions.md) before use. The kernel and every GPL
program are separate files, never linked into pryxd. The monitor is built from source so that
its C library matches the launcher's confinement. It keeps Firecracker's own restrictive
seccomp policy; only the libc-specific thread-signal, wait, terminal, file-open and
file-metadata rules are translated (`openat` where the published policy allows `open`, `statx`
where it allows `stat`, both met first in snapshot creation), and a read of its own PID is added ([how to rebuild it](../templates/vm-tools/monitor-REBUILD.md)).

The kernel is pinned in [kernel-selection.json](../templates/vm-tools/kernel-selection.json)
with the built-in options the guest needs: EROFS, overlayfs, ext4, virtio block and MMIO, vsock
and devtmpfs mounting. Its source is Linux 6.18.56 from
[kernel.org](https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.18.56.tar.xz), pinned in
[kernel-source.json](../templates/vm-tools/kernel-source.json), with its configuration and
patches in Kata's
[tools/packaging/kernel at 4.3.1](https://github.com/kata-containers/kata-containers/tree/4.3.1/tools/packaging/kernel).
A release publishes the complete corresponding source, `pryx-vm-sources.tar.gz`, beside the
runtime asset; the runtime's `notices/source-distribution.json` names it by digest. pryx builds
no kernel.

## How an image becomes a disk

There is no image daemon. pryxd runs `pryx-vm-image`, a separate helper from the bundle:

1. It pulls the manifest and layers with [oci-client](https://crates.io/crates/oci-client),
   anonymously, and checks each layer against its digest and its uncompressed `diff_id`.
   Layers are gzip or plain tar, at most 128 of them.
2. `mkfs.erofs --tar` turns each layer into a read-only EROFS file, keeping its owners,
   modes and times. Files are cached by layer digest and shared by every sandbox that uses
   them. A cached layer counts only once its file is complete and its record written.
3. The sandbox gets `vm.ext4`, a sparse file the size of its storage, made by `mke2fs`. It
   holds the overlay's writable layer and the data directory.

The guest boots from a fixed read-only bootstrap image. Its PID 1 is the static agent, which
receives its configuration and secrets over vsock into memory, never onto a disk. It mounts the
layers and the ext4 disk, joins them as an overlay, binds the data directory at the mount path,
mounts `/tmp`, `/run` and `/dev/shm` at the spec's temporary size, sets the hostname and the
guest's PID limit, applies the guest firewall, and only then starts the agent as the spec's
non-root user with the image's environment, entrypoint and working directory. The host's
`allowed_images` policy applies as on the container tier.

## Reaching the guest

pryxd reaches the agent over vsock, through the monitor's Unix socket. Exec, files, logs and
stats go there, authenticated as on other tiers. Declared TCP ports are forwarded by host-owned
relays over the same socket to the guest, and the edge reaches them as on any tier; the
host-side listeners are private. UDP ports are refused.

A VM's declared and spare ports share 64 simultaneous connections; one more is closed at once,
and the guest never sees it. The binding pryxd marks as reserved, the agent's own, has 64 of its
own that no public connection can use, so exec, files and stats keep working while a public port
is full.

While a VM is stopped or asleep, its files and logs are still readable and writable: pryxd
boots a short-lived file-only guest on the same disk, with fresh credentials, no network and no
workload, and shuts it down after.

## Networking

### As an ordinary user

The guest has no TAP device on the host and needs no user namespace or root. A virtual
interface inside the guest carries Ethernet frames over vsock to the packet helper, an
unprivileged gvisor-tap-vsock process that makes outbound connections as pryxd's user.

- **The default network** reaches public IPv4 addresses. DNS goes to a host-owned gateway.
  Non-public IPv4 ranges, private, loopback, link-local and metadata among them, are dropped,
  as is IPv6.
- **A hostname allowlist** (`egress`) lets out HTTP, TLS and DNS only, through host-owned
  gateways that apply the names; everything else is dropped. The node announces
  `egress-hostnames` for the `vm` tier itself, so control sends a VM request's policy even when
  the node's first tier (usually process) has no such gateways, and refuses it for a tier that
  doesn't announce it. The signed installation's acceptance runs one such rule
  (`deploy/vm-installed-acceptance.ts`).
- **`network: none`** gives the guest no network device and starts no packet helper. Its
  firewall allows loopback alone.

The trusted init installs the guest rules before any image code runs. The workload runs as a
non-root user with no capabilities, so it cannot change them, and every process it starts is
refused new vsock sockets, so it cannot reach the packet transport around them. The packet
helper itself runs under the launcher's Landlock and seccomp policy: it may open outbound
Internet sockets, and no listeners or Unix sockets.

The packet helper also decides on the host, so a guest kernel that drops its own rules does not
widen them. Before it dials any TCP, UDP or ICMP flow it refuses loopback, link-local and
metadata, private, multicast and broadcast destinations. pryxd passes every guest's egress
policy, and the helper refuses to start without one, so it also reaches the gateway address only
at the gateway ports pryxd names (HTTP, TLS and DNS under a hostname allowlist, DNS alone
otherwise), refuses the agent's other non-public networks, and on a hostname allowlist refuses
every public address. The ordinary user's helper runs no resolver of its own, so its guest's DNS
reaches only pryxd's gateway. As root, the bridged packet stack gets the same policy with no
gateway port; its guest resolves at the stack's own gateway, which answers from the host's
resolvers.

### As root

As root, the tier is served by a separate service, `pryxd-vm-worker.service`, which
`pryxd join` writes beside pryxd's unit and pryxd's unit wants. pryxd's own unit keeps its
sandbox (no devices, no namespaces, four capabilities); the worker runs `pryxd vm-worker` as
root, holds the VM authority and answers pryxd over two private root-owned sockets in pryxd's
state directory. It serves one pryxd at a time: when that pryxd stops, the VMs it served stop
with it, and the worker starts again for the next one. On a host without the tier it skips
itself, and pryxd reports the tier off with its name. Each
guest gets a TAP device in its own anonymous network namespace behind default-drop nftables
rules, read back before the monitor starts. Hostname policy uses the same gateways, listening
inside that namespace; the default network crosses a filtered bridge into a confined ordinary
packet stack. The host's own routes, forwarding settings and firewall are not changed. A
release lists `vm-bundle.json`, and the server preset (`sudo pryx up`) installs it beside the
root agent, where the worker finds it; the runtime it pins is fetched on the first VM create.
An install that names no `--allow-image` then admits, besides `alpine:` and `node:`, exactly the
images the bundle is proven with ([image-inputs.json](../templates/vm-tools/image-inputs.json)),
each by its full digest reference, which admits that digest and no other. They are ordinary
`allowed_images` entries in `/etc/pryxd/pryxd.toml`: an operator who removes them or narrows the
list narrows the tier with it, and adds prefixes there for other images, as for containers.

## Sleep, snapshot, fork, export, restore, move

- **Sleep with memory**, for an ordinary user's tier. At start, pryxd sleeps and wakes a
  file-only guest on a scratch disk; once that passes, the tier reports `memory-sleep` and
  control's sleep suspends instead of stopping. A suspend asks the guest's init, with a request
  signed for that boot, to freeze its workload's cgroup, flush its disk and hold a connection to
  the host; the monitor is then paused, writes a full snapshot of its state and memory, and exits.
  The workload runs again only once a resume has stepped the clock, reseeded it and brought its
  network back, or once the host gives the sleep up. The two files go into
  pryxd's private state for the VM tier, owned by pryxd's user and readable by no one else, as
  the disk is; they never go into a snapshot, an export, a rest or a transfer.
- **Wake with memory** restores that snapshot on the same node, when the host's class still
  matches: the CPU model and its features, the host kernel and the monitor's build, all
  recorded at sleep, and the disk is the very file the sleep left. Processes, terminals and
  `/tmp` carry on. The monitor's vsock reset ends init's held connection; init then takes a
  signed resume from the host, steps the wall clock to the host's, mixes host entropy into its
  pool, points its redirects at the gateways this wake listens on (their ports are new, and the
  resume names them), starts its network helper again and reopens its sleep listener. Open TCP
  connections don't survive. A host of another class, a changed disk or a restore that fails is never
  retried: the memory is discarded, the guest boots from its disk with everything written before
  the sleep, and the start answers `resumed: false`. Memory never leaves its node, so a wake
  anywhere else is a cold boot.
- **Sleep** without memory (root's VM worker, or before the start test passes) stops the guest
  and keeps its disk. **Wake** boots it again from that disk, on an API call or a held edge
  connection. A suspend request is refused, and control takes the stop path without asking.
- A file session or a cold boot of a guest asleep with memory discards that memory first;
  stopping it ends the memory as a kill would. A snapshot or an export of it copies its disk as
  the sleep flushed it, unrepaired (a boot of the copy checks it first), and leaves it asleep
  with its memory. Removing a sandbox removes its memory, and
  pryxd's start removes any memory file no sandbox's record names. Differential snapshots are not used: their
  upstream calls them a preview.
- **Fork with memory**, on one node: a guest asleep with its memory gives a new sandbox a copy of
  its disk and the same memory file, and the fork wakes running the source's processes under its
  own sandbox, agent key, boot nonce and hostname, which its resume hands it before its agent
  serves anything. Its init then has the kernel rekey its random generator from host entropy, so
  each fork draws random bytes of its own (the pinned kernel has no VM generation ID driver). A
  fork keeps everything of its source's spec the memory holds: image, sizes, mounts, user,
  network, variables, secrets and ports. Through the APIs it is a fork from a snapshot: control
  snapshots a sleeping VM sandbox without waking it (the snapshot names the memory), places a fork
  from that snapshot on the node that holds it when that node has room, and the node's restore
  (`fork`, feature `local-forks`) takes the memory too while the source still sleeps with it and
  the fork's spec differs only in what is each sandbox's own; the fork's first start then resumes
  (`resumed: true`). Any other fork, and every fork on another node, copies the disk and boots it.
- **Stop** sends the guest's init a signed request over vsock, carrying the spec's stop grace.
  Init stops the workload with SIGTERM or SIGINT (other stop signals are refused), gives it that
  whole grace before killing what remains, remounts its disks read-only and flushes them, then
  writes a signed record, bound to that boot, of the clean disk and the exit status; pryxd
  reads it from the console. pryxd waits the grace plus 15 seconds for that; a guest still
  running then is killed and reported as killed, and one that exits without that record has an
  unknown exit.
- **Snapshot, fork, export, restore and move** copy the stopped disk with the same file walk the
  other tiers use, preserving holes. A running VM is stopped for the copy and booted again
  after it. Export can't exclude paths, since it carries a disk image.
- **Recovery.** Every start first checks the existing disk read-only. Checks never write to it.
  A disk that needs repair is repaired on a staged copy, which replaces the original only after
  an independent check; one that needs more than automatic repair is refused, and the original
  is left as it was.

## Confinement of the monitor

As an ordinary user, the monitor is started by the static launcher, which takes the sandbox's
lifetime lease and then confines it:

- **Landlock**, fully enforced: read only its own program, the kernel, the bootstrap image, the
  image's layers, its configuration and a few `/proc`, `/sys` and `/dev` entries; write only its
  disk, its boot directory, `/dev/kvm` and `/dev/null`.
- **seccomp**: the process tier's dangerous-syscall denials, plus no network sockets, no
  signals or resource-limit changes for other processes, no io_uring, and no way to close or
  unlock its lease. Firecracker's own filter stays on; `--no-seccomp` and `--seccomp-filter`
  are refused.
- **Landlock's abstract-socket scope**, where the kernel has it (Landlock ABI 6): no abstract
  Unix socket outside its own domain.
- It is not dumpable, and the kernel kills it when pryxd exits. Its one-shot bootstrap socket
  hands the guest's secrets only to a peer with the monitor's own PID.

As root, the monitor runs under Firecracker's unchanged jailer, with fixed non-root host IDs
that the host chooses and a workload never can, in a chroot holding only its staged inputs,
under the same lease and signal bounds, inside the root worker above.

The filesystem programs (`mke2fs`, `e2fsck`, `mkfs.erofs`) read disks a guest wrote and layers
a tenant chose, so the same launcher confines them too, as root or not: Landlock reads of their
own component and inputs and writes of one pre-created output and a private directory only;
no socket of any kind; no capability, even as root; no core dump; and no file written past the
output's admitted size.

## Limits

- No KVM, no tier: there is no emulation, and the tier is x86_64 only.
- Root nodes serve the tier only through the VM worker.
- Memory sleep is for an ordinary user's tier, restored only on the same node and host class.
  Snapshots and exports of a running VM restart it. A fork copies the disk, and takes the memory
  too only from a snapshot of a VM asleep with it, on that VM's node.
- CPU is whole vCPUs: `cpuMillis` must be a multiple of 1000, and one vCPU is given without
  it. CPU weights are refused.
- Memory is whole MiB, the guest's RAM. The monitor's own memory on the host is outside it.
- The disk's size is fixed when it is made; a changed storage size is refused.
- Only TCP ports are forwarded. The guest network is IPv4.
- Images are pulled anonymously; registry credentials are not used on this tier.
- The workload runs as a non-root numeric user. `packages` (the Wasm tier's registry
  packages) are refused.
- When pryxd stops or crashes, its guests stop with it; their disks are kept and checked.
- Bucket mounts are refused. Desktop and SSH are not announced on this tier, and the browser and
  interpreter tools have no VM proof yet. Software in the image runs as in any image.

## Where KVM exists

pryx measures the host; a provider's name decides nothing.

- **Bare-metal x86_64 servers and Linux laptops or desktops** whose CPU has Intel VT-x or AMD-V
  turned on in firmware: the `kvm_intel` or `kvm_amd` module creates `/dev/kvm`. A `vmx` or
  `svm` flag in `/proc/cpuinfo` shows the CPU offers it.
- **Who can open it** comes from the distribution's udev rules, often a `kvm` group. `pryx doctor`
  shows whether pryxd's user can.
- **A virtual machine** has KVM only when its hypervisor exposes nested virtualisation. The
  plan's research ([any-host.md](any-host.md#35-virtual-machines)) found it opt-in on AWS
  (some Intel families), Google Cloud and Azure, and absent on Hetzner Cloud and on arm64
  virtual machines. Bare-metal cloud instances expose it directly.
- **GitHub's hosted `ubuntu-24.04` runners** have `/dev/kvm`; pryx's VM CI runs on them, granting
  the runner's user access in the job, on every pull request that changes VM code.
- **A container** can be given `/dev/kvm` with `--device /dev/kvm`, without `--privileged`;
  pryxd inside must run as an ordinary user for this tier.
- **arm64 hosts** keep the tier off.

## What is proven, and what is not yet

The [progress record](any-host-progress.md) holds every commit and run. In brief:

- [VM experiments 43668106319](https://github.com/cubepals/pryx/actions/runs/43668106319), at
  `b222bf6`, on real KVM: the runtime on Alpine 3.24.4 and Debian trixie-slim as an ordinary
  user (create, exec, files, logs, stats, stop, cold wake, copy and fork, daemon loss, network
  policies), strict monitor shutdown, root jailer boots, root lifecycle with both network
  policies, and the node API over mutual TLS
  ([summary](checks/measurements/k25/public-root-b222bf6.json)).
- [43617667633](https://github.com/cubepals/pryx/actions/runs/43617667633), at `f1a56d5`: the
  configured ordinary-user tier, its discovery in auto mode, its announced guarantees, and
  snapshot and restore through the node API ([summary](checks/measurements/k25/factory-f1a56d5.json)).
- [Any host 43721826335](https://github.com/cubepals/pryx/actions/runs/43721826335), at
  `f31a5b1`: the signed ordinary-user installation through both SDKs and the edge on both images
  (files, a declared edge port, cold wake from the edge, snapshot, fork and its own edge port,
  delete), one hostname egress rule sent by control to the vm tier of a node whose first tier
  is process, and the guarantee checks: a guest can't read a host-only file, can't signal
  pryxd, and reports a kernel other than the host's (`deploy/vm-installed-acceptance.ts`). They
  also pass in the root lifecycle (`src/runtime/vm/instance/root_acceptance.rs`). On normal CI's
  ordinary image, which has no KVM, a `vm` create is refused with `no_room` naming it
  (`control/bench/mixed-tiers.ts`).
- [VM experiments 43730018633](https://github.com/cubepals/pryx/actions/runs/43730018633), at
  `df53ca1`: the whole VM job again on real KVM with the filesystem programs confined, and each
  start's stages (in [decisions](decisions.md)).
- [Any host 43764836228](https://github.com/cubepals/pryx/actions/runs/43764836228) and [VM experiments 43764836235](https://github.com/cubepals/pryx/actions/runs/43764836235), both green at `0bd7fe3`
  (and earlier [43749214185](https://github.com/cubepals/pryx/actions/runs/43749214185) and
  [43749214194](https://github.com/cubepals/pryx/actions/runs/43749214194) at `c7a620b`):
  the same installed acceptance (`deploy/vm-installed-acceptance.ts`) as the signed ordinary user
  (bundle job) and as root through the server preset and its VM worker (sudo bootstrap job,
  `deploy/server-vm-acceptance.sh`, on the node configuration as the install wrote it): Alpine and Debian, run, a file round trip, a declared port
  through the edge, sleep and cold wake from the edge, snapshot and fork, delete, the guarantee
  checks, the default network's firewall (a public address answers, a listener on the host's
  own address does not) and one hostname egress rule; the purge after the ordinary user's run;
  and the whole VM job on real KVM.
- The thirty-sample cohorts ([tiers](tiers.md#vm-tier-measurements)) in Any host 43749214185,
  with no failed start.
- The kernel and the ordinary-user network path were chosen by measured spikes
  ([checks](checks/k25-vm-spikes.md)). Their boot figure, 717 ms at p50 from launch to a
  validated guest report, is a minimal fixture, not a sandbox create.

Not yet proven: a move between two VM nodes, and a published release carrying the
descriptor (the release workflow lists it, and runs only on `main`). A start right after a stop
failed twice in about sixty samples before starts stopped consulting the registry; it has not
recurred in the four installed cohort runs since (about 360 starts), and its cause was not
found.
