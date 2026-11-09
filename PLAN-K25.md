# K25: virtual machines

This is milestone K25 of PLAN.md, part of the any-host plan (docs/any-host.md). Everything under
"How to run this plan" in PLAN.md applies to it: read AGENTS.md first, above all "How pryx
decides"; work autonomously, never ask, record choices in `docs/decisions.md`, commit with short
subjects and no `@`, never claim a check passed that did not run. Licence first: the monitor,
the kernel and every tool in the VM bundle are checked and recorded before use; GPL programs and
the kernel ship as separate files with their sources linked, never linked into pryxd.

```
Goal

A `vm` tier: a sandbox with a kernel of its own, made from an ordinary image, for an ordinary
user who can open /dev/kvm, and for root. It boots cold, in a fraction of a second, and does
everything a sandbox does with data. It does not yet keep memory across sleep, and says so.

Context

Read docs/any-host.md sections 3.5 and 4.6, K19 (the KVM probe, guarantees), K21 (the router),
src/runtime.rs and one existing backend end to end (src/runtime/process.rs and its directory
is the closest), src/store.rs, agent/src/ (how the agent is started and reached per tier),
src/runtime/docker/egress.rs (the egress gateways), docs/architecture.md, docs/protocol.md.
Then, at recorded versions and licences: Firecracker (getting started, the API's OpenAPI
document, vsock, the jailer, the kernel policy, network setup); erofs-utils (`mkfs.erofs
--tar`) and e2fsprogs (`mke2fs -d`); a Rust registry client (oci-client); gvisor-tap-vsock and
passt's pasta; Kata Containers' released kernel and Firecracker's CI kernels and their configs.

What is built

1. Two spikes first, each a few days, each ending in a recorded decision with numbers:
   the unprivileged network path (a TAP device in a user namespace bridged by pasta, against a
   user-mode network over vsock with gvisor-tap-vsock: does each work with Firecracker, what
   does each cost, where does each not run); and the kernel (which published kernel boots the
   guest with EROFS, overlayfs, virtio and vsock, verified by a boot test).
2. The VM bundle: the monitor, the pinned kernel as its own file with its source linked,
   `mkfs.erofs` and `mke2fs`, and the network helper the spike chose. Pinned by digest in the
   release, fetched on first use, verified before anything runs.
3. Images without root: pull the layers with no daemon, turn each into an EROFS file cached
   by its digest, add an ext4 file for writes sized from the spec's storage, and boot a small
   init (part of pryx-agent) that mounts the overlay, sets the hostname and environment, and
   runs the agent as the sandbox's supervisor. The allowed-images policy applies as on the
   container tier.
4. The backend behind ContainerRuntime: start and stop through the monitor's API socket; the
   agent reached over vsock through the monitor's Unix socket; declared and spare ports
   forwarded by the agent as on other tiers; memory and CPU from the spec, enforced by the
   guest's size. The node announces `vm`, `own-kernel`, `memory-limit`, `cpu-limit`.
5. The network, by what the probe finds: as root, a TAP device behind the existing firewall,
   with the hostname egress policy; without root, the path the spike chose. `network: none`
   gives the guest no device at all.
6. Sleep stops the guest and keeps its disk; wake boots it. Snapshot, fork, export, restore
   and move copy the disk files with the walk the other tiers use. `memory-sleep` is reported
   off; control takes the stop path without asking.
7. Confinement of the monitor: as root, Firecracker's jailer; as an ordinary user, the
   process tier's Landlock and seccomp policy around the monitor's process.
8. docs/vm.md (what it is, what it needs, its limits, where KVM exists), docs/tiers.md and
   docs/compare.md.

Constraints

- No tier without KVM: where the probe finds none, `vm` is off with the reason, and nothing
  falls back to emulation.
- A `vm` request is never placed on another tier, and no other tier's request on `vm` unless
  the request left the tier open.
- The node protocol only adds. No Cubepals contract change.
- No kernel is built by pryx in this milestone; if neither published kernel serves, stop and
  record what is missing.
- Title under 50 characters, no motive, no `@`.

Done when

- In CI on a runner with KVM, as the runner's own user: create from alpine and from a
  Debian-based image, run, a file round trip, a declared port through the edge, sleep and
  wake, snapshot and fork, delete; then the same as root with the firewall and one egress
  rule.
- Measured, 30 samples each, in docs/tiers.md beside the other tiers: cold create to running,
  create to first output, agent `true`, wake, snapshot and fork of 100 MB, idle memory.
- The guarantee tests pass: the guest can't reach the host's files or signal pryxd, and its
  kernel version differs from the host's.
- On a runner without KVM the tier is off and a `vm` create answers `no_room` naming it.
- `cargo test --locked` and `bun run check` pass; CI green on the pull request and on main.
```
