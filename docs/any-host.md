<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Any Linux host: research and plan

A plan. Nothing in it is built unless section 1 says so. It follows from
[How pryx decides](../AGENTS.md#how-pryx-decides): one command, presets, rootless first, everything
by tier, never a weaker guarantee in silence, wrap before writing. The milestones are the briefs
`PLAN-K19.md` to `PLAN-K27.md` at the repository's root; this page is why they are what they are.

Read on 2026-11-09 against `main` at `eec6936`. Versions and licences of outside projects were
read from their own repositories that day. Where a fact could not be confirmed from a primary
source it says **unverified**, and the milestone that depends on it starts by checking it.

## 1. What pryx is today

Checked against the code, not the documentation.

This section records the research baseline above. K19's implementation and its acceptance
evidence are tracked in [any-host progress](any-host-progress.md); its current command and
guarantee contract is documented in [host capabilities](host-capabilities.md).

**One backend per node.** A `pryxd` runs exactly one of three backends, set by the `runtime` key
(`src/config/process.rs`, `src/main.rs`): `container` (Docker), `process` (a restricted process
tree) or `wasm` (agentOS virtual machines in WebAssembly). Control derives one tier per node from
the features the node announces (`control/src/isolation.ts`). Nothing in `pryxd` looks at the host
to choose. The release image's entrypoint is the only detection there is: it takes `container` when
a Docker socket answers and `process` otherwise (`deploy/image-tier.ts`).

| | Container | Process | Wasm |
| --- | --- | --- | --- |
| Needs | A Docker daemon; root for the firewall and for data ownership | Landlock ABI 3 (Linux 6.2) and seccomp; no root | x86-64 glibc and the pinned agentOS sidecar; no root |
| Images | Yes | No | No; registry packages |
| Limits | cgroup memory, CPU, pids | Sampled memory; CPU limits refused | Virtual heap bounds; CPU limits refused |
| Sleep keeps memory | Yes (freezer) | Yes (SIGSTOP) | No; the VM is disposed |
| Network boundary | Its own namespace; nftables rules installed by root | None; shares the host's | The VM's network policy |
| Browser, desktop, SSH, mounts, kernels | Yes, with host opt-ins for the browser and mounts | Refused by name | Refused by name |

**What already runs without root.** The release image starts with no privilege, no engine socket
and no seccomp override, and gives the process and wasm tiers with Postgres and an object store
inside it (`docs/deploy.md`). CI runs the wasm suite with `pryxd` as an ordinary user. So the
unprivileged core exists. It is reachable only through `docker run`, Compose or Render.

**What installing is today.** `install.sh` puts the CLI on the machine. `sudo pryx up` then needs
Linux x86_64, systemd and root, installs Docker and nftables through `apt`, and gives the
container tier only. On another distribution the person sees `sh failed` or `apt-get failed`
(`cli/src/up-host.ts`). Every artefact is x86_64.

**What a request is promised today.** A request that names `isolation` gets that tier or
`no_room`; placement matches exactly (`control/src/placement.ts`). That part is right. These are
not:

- A process node started without root shares one uid between all its sandboxes. It announces
  `process-shared-uid`, and nothing in control reads it, so a request can't ask for separate users
  and isn't told it didn't get them.
- `pryxd serve` never checks that the firewall `pryxd isolate` installs is there. A container node
  started outside its unit runs without it and announces nothing different.
- Fields a node doesn't announce (`egress`, `network`, `user`, `workdir`) are dropped from the
  spec rather than refused (`control/src/workload-api.ts`). New placements are protected by
  placement's own check; a wake on a node that lost a feature is **unverified**.
- No node announces whether it can run the browser. A browser sandbox placed on a host without
  user namespaces fails at run time with "check host tool requirements".
- Every node announces `suspend`, and on wasm a suspended sandbox keeps no memory.
- `pryxd doctor` checks only what the container tier needs, whatever the node runs.

**What isn't there at all.** No virtual machine backend, and no decision for or against one. No
rootless container engine as a backend. No browser, desktop, SSH, mount or language kernel off
the container tier. No arm64. No report of what the host can do.

## 2. The distance to the goal

| The goal | Today | The gap |
| --- | --- | --- |
| One command, asks nothing, any supported host | Two commands; root, systemd, apt and x86_64 for the second | A preset for an ordinary user, chosen by the installer; clear refusals elsewhere |
| pryx finds out what the host can do | The image checks for Docker | A probe in `pryxd`, reported to control and printed by `pryx doctor` |
| Every backend the host supports, at once | One backend per node | A node that serves several tiers; the spec carries the tier |
| Never a weaker guarantee in silence | True for the tier's name; not for what a tier gives on this host | Guarantees named, reported with every sandbox, and requirable |
| Containers without root | Root Docker only | Any Docker-compatible engine, with what it can't enforce said |
| Browser, desktop, SSH, kernels without Docker | Container tier only | A tool bundle and private transports for the process tier |
| Virtual machines | None | A `vm` tier on KVM |
| Stronger than a container without KVM | None | gVisor as a container runtime |
| More than x86_64 | x86_64 only | arm64 artefacts; wasm stays x86_64 while its sidecar does |

## 3. What the research found

### 3.1 What a host lets an ordinary user do

The same four facts decide almost everything: user namespaces, cgroup delegation, Landlock's
version, and `/dev/kvm`.

| Host | User namespaces | Landlock | Writable cgroups | Notes |
| --- | --- | --- | --- | --- |
| Debian 12, 13; Fedora; RHEL 9, 10 | Yes | ABI 2 on 6.1, 6 on 6.12 | With a systemd user session: memory, pids and, from systemd 252, cpu | Rootless Podman or Docker works once root has installed `uidmap` and subordinate ids |
| Ubuntu 22.04 | Yes | ABI 1 (5.15), 4 on the 6.8 kernel | memory and pids only (systemd 249) | Below the process tier's Landlock floor on its first kernel |
| Ubuntu 24.04, 26.04 | Only for binaries at fixed paths with an AppArmor profile (`/usr/bin/podman`, `rootlesskit`, `runc`, `crun`); anything else gets a namespace with no capabilities | 4 and later | cpu, memory, pids | A binary in a home directory cannot use user namespaces without a profile root wrote |
| A default Docker container | No: the default seccomp profile refuses `unshare` and namespace `clone` without `CAP_SYS_ADMIN` | The host's; `landlock_*` and `seccomp` are allowed | No | Only Landlock and seccomp confinement works here |
| A Kubernetes pod, baseline or restricted | No | The host's | No | As a default Docker container |
| GitHub Actions hosted runner | Yes (with sudo) | Yes | Yes | `/dev/kvm` on x86_64 after a udev rule |
| WSL2 | Yes | Yes | With systemd | KVM depends on the Windows host |

Read from Docker's and Podman's seccomp profiles, systemd's `user@.service`, Ubuntu's `apparmor`
package and the kernel's Landlock documentation.

**The floor.** Where there are no user namespaces, no capabilities, no KVM and no writable
cgroups, the strongest thing a process can do for its children is Landlock, a seccomp filter,
`no_new_privs` and rlimits, all under one uid. That is pryx's process tier as it is built. Nothing
stronger exists for those hosts: gVisor's rootless mode, bubblewrap, rootless Podman and Sysbox
all need user namespaces or the host's administrator.

### 3.2 Containers without root

| Option | Licence | What it needs | Verdict |
| --- | --- | --- | --- |
| [Rootless Docker](https://docs.docker.com/engine/security/rootless/) 29.9 | Apache-2.0 | `newuidmap`, 65,536 subordinate ids, user namespaces; cgroup v2 with systemd for limits | Use, through its socket |
| [Podman](https://docs.podman.io) 6.1, Docker-compatible API 1.44 | Apache-2.0 | The same; pasta for networking | Use, through its socket |
| [runc](https://github.com/opencontainers/runc) or [crun](https://github.com/containers/crun) driven directly | Apache-2.0; GPL-2.0 | pryx would pull, unpack, layer, map ids and network itself | Skip: it rebuilds Podman for few hosts |
| [bubblewrap](https://github.com/containers/bubblewrap) 0.13 | LGPL-2.0-or-later, run as a program | User namespaces (its setuid mode is gone since 0.12) | Use to add namespaces to the process tier where they exist |
| [gVisor](https://gvisor.dev/docs/) `runsc` | Apache-2.0 | Registered as a Docker runtime; its systrap platform needs no KVM | Use as a tier on engine hosts |
| [PRoot](https://proot-me.github.io) 5.5 | GPL-2.0 | ptrace only | Not a tier: it isolates nothing, and measured 5 times slower on a browser start and 32 times on file-heavy work |

Rootless engines lose things, and Docker loses them silently: without a delegated cgroup,
`--memory` and `--cpus` are ignored; there is no freezer, so no sleep with memory; ports and
outbound traffic go through a user-mode network. pryx has to read `info` (`Cgroup Driver`,
`rootless`) and say so.

### 3.3 Browsers

- Chromium's sandbox needs user namespaces with capabilities inside them, or a setuid-root helper.
  With neither it stops with "No usable sandbox!". There is no seccomp-only mode, and
  `--no-sandbox` removes the seccomp filter along with the namespaces (measured on headless shell
  157: `Seccomp: 0` against `2`).
- So the browser's own sandbox is impossible, without root, in a default Docker container, in a
  Kubernetes pod with the default seccomp profile, and on Ubuntu 24.04 or later for a browser
  installed outside the paths Ubuntu profiles.
- The binary and its libraries need no root: Chrome for Testing's headless shell (278 MB) with
  about 25 libraries from a relocatable prefix rendered a page in a root filesystem holding only
  glibc. Playwright's `install-deps` is the step that needs root, and it isn't needed.
- Firefox keeps a seccomp filter without user namespaces (from Mozilla's documentation;
  **unverified** by test). It speaks WebDriver BiDi; CDP was removed in Firefox 141.
- Cost, measured: a headless shell is about 173 MB and reaches a ready CDP endpoint in about
  80 ms warm; a headful Chrome on a virtual display is about 437 MB.

| Piece | Project | Licence |
| --- | --- | --- |
| Browser | [Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/) headless shell, as pryx's browser template uses through [Playwright](https://playwright.dev) 1.65 | BSD-3-Clause; Apache-2.0 |
| Fallback engine | [Firefox](https://firefox-source-docs.mozilla.org) over BiDi | MPL-2.0 |
| Not used | Browserless (SSPL); Lightpanda (AGPL, no screenshots) | |

### 3.4 Desktops

- A virtual X display, input and screenshots all run as an ordinary user: Xvfb (15 MB), TigerVNC's
  `Xvnc` (16 MB), xdotool. Tested as uid 65534.
- X11 stays the target. Wayland has no input injection that works across compositors without
  `/dev/uinput`, which is root's; every reference implementation for computer use is on X11.
- What needs root's help once: GPU rendering (`/dev/dri`), `/dev/uinput`, and a full desktop
  environment's system services. Software rendering is enough for an agent.
- A desktop with a browser is 550 to 650 MB: about two for each gigabyte.

| Piece | Project | Licence |
| --- | --- | --- |
| Display | [Xvfb](https://www.x.org/releases/current/doc/man/man1/Xvfb.1.xhtml), or [TigerVNC](https://tigervnc.org) `Xvnc` (display and VNC in one process) | MIT; GPL-2.0, run as a program |
| Input, screenshots | [xdotool](https://github.com/jordansissel/xdotool), as today | BSD-3-Clause |
| Viewing in a browser | [noVNC](https://novnc.com) with [websockify](https://github.com/novnc/websockify), as today; [Selkies](https://docs.selkies.io) if video is ever needed | MPL-2.0; LGPL-3.0, run as a program; MPL-2.0 |

### 3.5 Virtual machines

- **With KVM.** [Firecracker](https://github.com/firecracker-microvm/firecracker) 1.18
  (Apache-2.0) is one static binary that runs as an ordinary user who can open `/dev/kvm`; only
  its jailer needs root. It publishes at most 125 ms to the guest's init and at most 5 MiB of
  overhead. It reaches the guest over vsock as a Unix socket on the host, so the in-guest agent
  needs no network device. Its limits: no shared filesystem, so an image becomes a block device;
  network devices are TAP only; a memory snapshot restores only on the same CPU model and host
  kernel, and differential snapshots are a developer preview.
- [Cloud Hypervisor](https://github.com/cloud-hypervisor/cloud-hypervisor) 54 (Apache-2.0 and
  BSD-3-Clause) adds live migration between hosts, restore on demand and virtio-fs, with more
  moving parts and no published boot figures.
- [libkrun](https://github.com/containers/libkrun) (Apache-2.0) is the easiest to embed and has
  no pause or snapshot on Linux upstream; the projects that have them carry forks.
- **Without KVM.** A VM is then QEMU's software emulation. Measured against the same host:
  Python 36 times slower, shell spawns 54 times, a compile about 126 times, three seconds to
  boot. That is not a tier.
- **Where KVM is.** Bare metal; opt-in nesting on AWS (Intel 7th and 8th generation families),
  GCP and Azure; not on Hetzner Cloud or arm64 virtual machines. `/dev/kvm` can be passed to a
  container with `--device`, without `--privileged`.
- **Images without root.** `mkfs.erofs --tar=f` turns an image layer into a read-only filesystem
  keeping its owners, and `mke2fs -d` makes the writable one, both as an ordinary user (tested).
- **The kernel.** A guest kernel is GPL-2.0: shipped as its own file with its source offered, it
  is not part of `pryxd`. The cost is its security updates, not its licence.

## 4. The design

Nothing here replaces a part. Each item names the seam it uses.

### 4.1 The probe

`pryxd` gains a probe that tests the host instead of reading settings, at start and on a timer:
create a user namespace and use a capability in it; ask Landlock for its ABI; read its own
cgroup's controllers; open `/dev/kvm` and run a guest that halts; open `/dev/fuse`; find Docker
and Podman sockets and read their `info`; check the firewall table is loaded; find systemd and a
user session. The result is one JSON document.

- `pryxd probe --json` prints it. `pryx doctor` prints it for a person: one line per capability
  with on or off, why, and the one thing that would turn it on.
- The node announces what the probe proved through `features`, which only adds
  (`src/protocol.rs`, `src/manager/store_lease.rs`).
- The probe is the test oracle in CI (section 7): each environment asserts its exact output.

### 4.2 Tiers and guarantees

A tier's name says how a sandbox runs. A guarantee says what this sandbox has on this host. Both
are reported with every sandbox; a request can require either.

| Tier | `isolation` | Exists | Needs |
| --- | --- | --- | --- |
| Process | `process` | Yes | Landlock ABI 3 |
| WebAssembly | `wasm` | Yes | The agentOS sidecar (x86-64) |
| Container | `container` | Yes | A Docker-compatible engine |
| gVisor | `gvisor` | K26 | An engine with `runsc` registered |
| Virtual machine | `vm` | K25 | `/dev/kvm` |

| Guarantee | Means | Off when |
| --- | --- | --- |
| `separate-users` | The sandbox has a uid no other sandbox has | Process tier without root |
| `memory-limit`, `cpu-limit` | The kernel enforces the limit | Process and wasm; an engine with no delegated cgroup |
| `private-network` | Its own network stack | Process tier without namespaces |
| `host-firewall` | Sandboxes can't reach the host, metadata addresses or other nodes | The firewall isn't loaded |
| `own-kernel` | A kernel of its own | Everything but `vm` |
| `memory-sleep` | Sleep keeps processes | Wasm; rootless engines; `vm` until K27 |
| `browser-sandbox` | The browser's own sandbox is on | No usable user namespaces |
| `private-pids` | It can't see other sandboxes' processes | Process tier without namespaces (K26) |

- A sandbox's view gains `guarantees`. A create gains an optional `require` list. A deployment has
  a default `require`, which a preset writes. Placement refuses a node that lacks one and says
  which; the answer is `no_room` with the reason, as for a tier today.
- A request that sets a limit the tier can't enforce is refused, as CPU limits on the process tier
  are today. That rule extends to memory on an engine without cgroups.
- When a request names no tier, control takes the first of `container`, `vm`, `process`, `wasm`
  that the deployment offers and meets `require`, and the response says which. A preset can set a
  different order. A named tier is never replaced.

### 4.3 One node, several tiers

The node protocol's spec gains an optional `isolation`, sent only to nodes that announce
`multi-tier`. `pryxd` builds every backend the probe allows and routes each workload to its own
behind the existing `ContainerRuntime` seam (`src/runtime.rs`). A control plane that doesn't send
the field gets the node's default tier, which is today's behaviour. Control reads a set of tiers
per node instead of one (`control/src/isolation.ts`, `placement.ts`). Capacity stays the node's.

### 4.4 Any engine

The container backend already speaks the Docker API. It gains discovery (`DOCKER_HOST`, the
user's Docker socket, the user's Podman socket) and reads `info`: rootless or not, the cgroup
driver, the security options. What is missing becomes guarantees that are off: no `memory-limit`
without cgroups, no `memory-sleep` without the freezer (control already falls back to stop), no
`host-firewall` without root. Nothing about the root path changes.

### 4.5 Tools without Docker

The browser, the desktop, SSH and the language kernels are refused off the container tier because
they talk over loopback TCP, which process sandboxes share. The fix is transport, not a new
component:

- The browser is driven over `--remote-debugging-pipe`; the agent owns the pipe and serves the
  same authenticated proxy as today.
- Kernels use the Jupyter protocol's `ipc` transport, sockets under the sandbox's own directory.
- The VNC server and websockify listen on Unix sockets under the sandbox's directory.
- Landlock already keeps one sandbox out of another's directory, so a pathname socket there is
  private. The X server's own socket is the open point: it lives in `/tmp/.X11-unix`, which
  process sandboxes share. K24 starts by testing a socket path of its own (`DISPLAY` as a path);
  if that fails, the desktop on the process tier needs user namespaces, and says so.

The programs themselves come from a **tool bundle**: a relocatable directory with the browser, its
libraries and fonts, the display server, xdotool, noVNC and websockify, built in CI from pinned
packages against an old glibc, published with the release, verified by digest and fetched the
first time a tool is asked for. The container templates stay as they are.

**The browser's sandbox.** It is on wherever the host allows it. Where it can't be, the browser
runs without it only inside a tier whose own boundary holds it (`container`, `gvisor`, `vm`), and
the sandbox reports `browser-sandbox` off. On the process tier without user namespaces the
browser is refused with the reason. A request can require `browser-sandbox`.

**SSH** today needs the edge's raw TCP front, which the one-listener presets don't have. It gains
a second way in: `pryx sandbox ssh` connects through the agent's authenticated WebSocket as
OpenSSH's `ProxyCommand`, which works wherever HTTP does.

### 4.6 Virtual machines

A `vm` backend that drives Firecracker over its API socket.

- **Make.** Pull the image without a daemon, turn each layer into an EROFS file cached by digest,
  add a small ext4 file for writes, and boot with a tiny init that mounts the overlay and starts
  `pryx-agent`. `mkfs.erofs` and `mke2fs` run as programs from the VM bundle, beside the monitor
  and the kernel.
- **Reach.** The agent listens on vsock; `pryxd` dials the monitor's Unix socket. Ports go the
  same way, through the agent's existing forwarding.
- **Network.** In order, by what the probe finds: a TAP device and the existing firewall as root;
  a TAP device inside a user namespace bridged by [pasta](https://passt.top); a user-mode network
  over vsock with [gvisor-tap-vsock](https://github.com/containers/gvisor-tap-vsock)
  (Apache-2.0; with Firecracker **unverified**). K25 starts with a measured spike of the last two.
- **Sleep, wake, snapshot, fork, move.** At first exactly what the other tiers do with data: stop
  to disk and boot again, copy the disk. `memory-sleep` is off and said so. K27 adds memory
  snapshots where a restore can be trusted: the same node, or nodes of one compatibility class
  (CPU model, host kernel, monitor version).
- **The kernel.** Pin a kernel another project builds and publishes (Kata Containers' release
  kernel or Firecracker's CI kernel), verified by digest and a boot test, with its source linked.
  pryx builds its own only if neither has what the guest needs.
- **As root** the monitor runs under Firecracker's jailer. As an ordinary user it runs under the
  process tier's own Landlock and seccomp policy.

### 4.7 Installing

One command, the same on every host:

```sh
curl -fsSL https://github.com/cubepals/pryx/releases/latest/download/install.sh | sh
```

It installs the CLI and runs `pryx up`, which probes, takes a preset and starts. It asks nothing
and ends by printing the URL, the key, the tiers that are on and the capabilities that are off
with what would turn each on. `--preset` overrules the choice; `--check` prints the plan and
changes nothing.

| Preset | Chosen when | What it does |
| --- | --- | --- |
| `user` | Not root | Binaries under `~/.local`, state under the XDG directories, Postgres and the object store as supervised children, ports above 1024, a systemd user unit with linger where there is one and a foreground supervisor where there isn't. Every tier the probe allows. |
| `container` | Inside a container | The same as `user`, in the foreground: today's release image. `docker run` stays its one command. |
| `server` | Root with systemd | Today's `pryx up`: the engine, the firewall, the units. Adds `runsc` and the `vm` tier when the probe allows. Run by putting `sudo` before `sh`. |
| `node` | `pryx node join <token>` | A host joining a fleet, by whichever of the above fits it. |

- A domain is optional. Without one, sandboxes are reached through the path aliases the release
  image already serves.
- The `user` preset is the release image's entrypoint (`deploy/image.ts`) run on the host: the
  same supervision, the same embedded stores, from a bundle instead of an image layer. Postgres
  comes as relocatable binaries
  ([postgresql-embedded](https://github.com/theseus-rs/postgresql-embedded), MIT or Apache-2.0,
  or [zonky's](https://github.com/zonkyio/embedded-postgres-binaries), Apache-2.0).
- On a host a preset can't serve, the installer says which requirement is missing and the one
  command that fixes it, before changing anything.

## 5. What works where

| | Ordinary user, no user namespaces (default container, pod, Ubuntu 24.04 home install) | Ordinary user with user namespaces | With a rootless engine | With `/dev/kvm` | Root (`server`) |
| --- | --- | --- | --- | --- | --- |
| Process tier | Yes, shared uid | Yes, shared uid; namespaces through bubblewrap (K26) | Yes | Yes | Yes, separate users |
| Wasm tier (x86-64) | Yes | Yes | Yes | Yes | Yes |
| Container tier | No | No | Yes; limits and memory sleep only with delegated cgroups | Through an engine | Yes, all guarantees |
| gVisor tier | No | No | Where `runsc` is registered | The same | Yes |
| VM tier | No | No | No | Yes | Yes, where KVM is |
| Browser | Refused on process; no usable sandbox | Yes, sandbox on | Yes | Yes | Yes |
| Desktop | Depends on K24's socket test | Yes | Yes | Yes | Yes |
| Language kernels, SSH | Yes | Yes | Yes | Yes | Yes |
| Bucket mounts (FUSE) | No | No | No | In a VM | Yes, with the host opt-in |
| Hostname egress policy | No | No | No | In a VM, through its proxy | Yes |
| Memory and CPU limits | No | No | With delegated cgroups | In a VM | Yes |

What needs root once, and nothing else gives: separate uids, the host firewall, cgroup controllers
systemd doesn't delegate, ports below 1024, an AppArmor profile on Ubuntu, `uidmap` and
subordinate ids for a rootless engine, access to `/dev/kvm`, `/dev/fuse` and `/dev/dri`.

## 6. Where the goal should bend

- **No VM tier without KVM.** At 36 to 130 times slower it would be a tier nobody can use. Where
  KVM is missing the stronger tier is gVisor, and pryx says KVM is what would give VMs.
- **Rootless is for trying, developing and CI.** One uid, no enforced limits and a shared network
  are not a boundary between tenants. The `user` preset says so once, in its summary, and the
  `server` preset's default `require` refuses those sandboxes.
- **Memory snapshots are not portable.** A VM that sleeps with its memory can wake only on a
  matching host. "Sleep with memory, wake anywhere" is not promised; cold boot is, everywhere.
- **Ubuntu's AppArmor rule has no rootless answer.** From 24.04 a binary in a home directory can't
  use user namespaces. pryx reports it and names the one profile root would install; it doesn't
  pretend.
- **A guest kernel is upkeep for as long as the tier exists.** Pinning another project's build
  keeps it small. Building pryx's own is refused until something forces it.
- **Left out on purpose:** Wayland desktops, PRoot as a tier, driving runc directly, libkrun,
  macOS and Windows hosts, GPU passthrough. Each adds a second way to do something a first way
  already does, or serves a host pryx doesn't target. Each can come back with a reason.
- **"Most complete" is checked, not said.** `docs/compare.md` stays the place where each claim
  links to code that CI runs.

## 7. Proving it

- **The matrix.** A scheduled workflow, not a pull request's gate, installs with the one command
  and runs `hello` in each cell: distributions as booted system containers
  ([Incus](https://linuxcontainers.org/incus/docs/main/), Apache-2.0) with a real user session;
  `docker run` with the default profile, with `--cap-drop ALL`, with `--user`, with a profile that
  refuses Landlock; a pod under the restricted standard in a local cluster; and, on runners with
  KVM, cloud images of the distributions whose kernels differ (Ubuntu's AppArmor rule, Debian
  12's Landlock).
- **The probe is the oracle.** Each cell asserts the exact `pryxd probe --json`. A capability that
  changes without a change to the expectation fails the cell.
- **Each guarantee is attacked.** For every guarantee a cell reports on, the suite tries to break
  it: write outside the sandbox, connect to a forbidden port, signal `pryxd`, exceed the memory
  limit, read another sandbox's socket. Reported on and broken is a failure; reported off and
  required is a refusal with the reason.
- **Time to hello.** Each cell records the seconds from the command to the first `hello`, and the
  number of questions asked, which is zero.
- **Refusals are tested too.** A cell where a preset can't work asserts the message and that
  nothing on the host changed.

## 8. The milestones

Ordered by what a person gets and by what depends on what. Each is one brief and one pull request
or a short series, independently testable. Acceptance and completion are in each brief.

| | Brief | A person gets | Needs |
| --- | --- | --- | --- |
| K19 | [The host says what it can do](../PLAN-K19.md) | `pryx doctor`; guarantees reported and requirable; no silent weakening | Nothing |
| K20 | [One command for an ordinary user](../PLAN-K20.md) | Install with no root, no Docker, no questions | K19 |
| K21 | [Several tiers on one node](../PLAN-K21.md) | Every tier the host has from one install | K19 |
| K22 | [Any container engine](../PLAN-K22.md) | Images without root, on rootless Docker or Podman | K19, K21 |
| K23 | [Browsers and kernels without Docker](../PLAN-K23.md) | A browser and a code interpreter on the process tier | K19, K20 |
| K24 | [Desktops and SSH everywhere](../PLAN-K24.md) | Computer use and SSH on every preset | K23 |
| K25 | [Virtual machines](../PLAN-K25.md) | A `vm` tier with its own kernel, as an ordinary user with KVM | K19, K21 |
| K26 | [Stronger without KVM](../PLAN-K26.md) | A gVisor tier; namespaces for the process tier | K21, K22 |
| K27 | [Memory for virtual machines, and arm64](../PLAN-K27.md) | VMs that sleep with memory on matching hosts; arm64 artefacts | K25 |

## 9. Decisions taken, and what could overrule them

They are recorded here. The milestone that acts on one copies it into
[decisions.md](decisions.md) with what it measured.

1. Tiers keep their names and gain `gvisor` and `vm`; strength within a tier is a guarantee, not a
   new tier.
2. One `pryxd` serves several tiers; the alternative, one daemon for each tier, doubles every
   operation an operator does.
3. Rootless containers come from an engine's socket. pryx doesn't drive an OCI runtime itself.
4. No VM without KVM, no PRoot tier, no Wayland.
5. Firecracker first for VMs; Cloud Hypervisor is evaluated in K27 only for moving a running VM.
6. VMs boot cold before they keep memory.
7. The guest kernel is another project's published build, pinned.
8. Tools reach the process tier through pipes and pathname sockets, from a bundle fetched on
   first use.
9. A browser without its own sandbox runs only inside a container, gVisor or VM tier, and is
   reported; on the process tier it is refused.
10. The installer runs `pryx up`; root is chosen by typing `sudo`, never asked for.
11. The `user` preset is the release image's entrypoint run on the host.
12. The matrix is a scheduled workflow, so it adds nothing to a pull request's CI.

What would change them: Firecracker dropping unprivileged use; a host class with user namespaces
but no engine becoming common enough to justify driving a runtime; upstream libkrun gaining
snapshots; a published kernel that stops being maintained.

Nothing here waits on a maintainer. A maintainer is asked, by the rule, only if a milestone finds
it needs money, a dependency whose licence changes what pryx may ship, or something that can't be
undone.
