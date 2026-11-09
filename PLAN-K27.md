# K27: memory for virtual machines, and arm64

This is milestone K27 of PLAN.md, the last of the any-host plan (docs/any-host.md). Everything
under "How to run this plan" in PLAN.md applies to it: read AGENTS.md first, above all "How
pryx decides"; work autonomously, never ask, record choices in `docs/decisions.md`, commit with
short subjects and no `@`, never claim a check passed that did not run. Licence first, as
before.

```
Goal

Two things that widen what the earlier milestones built. A virtual machine that sleeps with its
memory and wakes where that can be trusted. And pryx on arm64, with every tier the architecture
has.

Context

Read docs/any-host.md sections 3.5, 4.6 and 6, K25 (the vm backend) and its recorded
decisions, control/src/power.ts, placement.ts and transfers.ts, src/protocol.rs (the suspend
feature), docs/architecture.md (suspended, stopped, resting). Then, at recorded versions:
Firecracker's snapshot documentation and its stated limits (the same CPU model and host
kernel, vsock reset on restore, VMGenID, differential snapshots as a preview), Cloud
Hypervisor's snapshot, restore and live migration documentation. For arm64:
.github/workflows/release.yml, control/tools/release.ts, deploy/install.sh, deploy/agentos.json
(the sidecar is x86-64 only), control/src/providers/hetzner.ts (the x86 filter).

What is built

1. Memory sleep for `vm`: pause, write the monitor's state and memory beside the disk, stop.
   Wake restores on the same node and falls back to a cold boot from the disk when the restore
   is refused or the snapshot's class doesn't match. The agent has a resume hook: it steps the
   clock, reseeds what the guest's kernel doesn't, and reopens its vsock listener. The node
   announces `memory-sleep` for `vm` only when a restore test passed at start.
2. Compatibility classes: a snapshot records the CPU model, the host kernel and the monitor's
   version. Control places a wake with memory only on a node of the same class; any other wake
   is a cold boot, and the sandbox's view says which it was.
3. Fork with memory, on one node: copies share the memory file and the disk copy-on-write and
   each gets a new identity in its resume hook. Across nodes a fork copies the disk, as now.
4. An evaluation, recorded with numbers, of Cloud Hypervisor for moving a running VM between
   nodes of one class. It becomes a second monitor only if it gives a move Firecracker can't
   and the cost of a second monitor is stated; otherwise the record says why not.
5. arm64: the release builds pryxd, pryx-agent, the control and edge binaries, the CLI, and
   the tool and VM bundles for aarch64; install.sh and the presets take the architecture from
   the host; the Hetzner provider offers arm64 shapes. The wasm tier is off on arm64 with the
   reason, until its sidecar exists there. The image is published for both architectures.

Constraints

- Sleep with memory is never promised across unlike hosts, and a failed restore never loses
  the disk or leaves two copies running: the epoch rules hold.
- No secret that the guest's memory holds is written anywhere the disk's files aren't already
  protected.
- Differential snapshots are not used while their upstream calls them a preview.
- A second monitor is not added without the recorded evaluation.
- Title under 50 characters, no motive, no `@`.

Done when

- In CI on a runner with KVM: a `vm` sandbox sleeps with a process running and wakes with it
  still running, with 30 timed samples of sleep and wake in docs/tiers.md; a restore made to
  fail falls back to a cold boot with the data intact; a fork with memory gives two sandboxes
  with different identities.
- A wake on a node of another class is a cold boot, asserted, and reported in the view.
- The one command installs and runs `hello` on an arm64 runner in the scheduled matrix, with
  the process, container and, where KVM exists there, `vm` tiers, and wasm reported off.
- The Cloud Hypervisor evaluation is in docs/decisions.md.
- `cargo test --locked` and `bun run check` pass; CI green on the pull request and on main,
  the release included.
```
