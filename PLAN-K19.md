# K19: the host says what it can do

This is milestone K19 of PLAN.md, the first of the any-host plan (docs/any-host.md). Everything
under "How to run this plan" in PLAN.md applies to it: read AGENTS.md first, above all "How pryx
decides"; work autonomously, never ask, record choices in `docs/decisions.md`, commit with short
subjects and no `@`, never claim a check passed that did not run.

```
Goal

Make pryx find out what its host can do, say it to a person and to control, and report with
every sandbox what that sandbox actually has, so that nothing a host can't give is ever given
in silence. No new backend. This is the ground every later milestone stands on.

Context

Read docs/any-host.md sections 1, 4.1 and 4.2. Then src/doctor.rs and src/doctor/, src/infer.rs,
src/host.rs, src/manager/node.rs, src/manager/store_lease.rs (dynamic features), src/protocol.rs
(FEATURES), src/isolation.rs, src/runtime/docker/egress.rs, agent/src/process_capabilities.rs,
control/src/isolation.ts, placement.ts, node-choice.ts, workload-api.ts, public-model.ts,
api-schemas.ts, sandboxes.ts, power.ts, cli/src/ (up.ts, up-host.ts), docs/tiers.md,
docs/process-tier.md, docs/native-tools.md.

What is built

1. The probe, in pryxd. It tests, never reads a setting where a test exists: a user namespace
   is created and a capability used inside it; Landlock's ABI is asked for; the daemon's own
   cgroup's controllers are read; /dev/kvm is opened and a guest that halts is run, timing its
   exits; /dev/fuse is opened; Docker and Podman sockets are found and their info read; the
   firewall table is looked up; systemd, a user session and linger are detected; the
   architecture and the C library are named. `pryxd probe --json` prints one document with,
   for each capability, on or off, the evidence, and the one action that would turn it on.
2. `pryx doctor`, in the CLI: the same document for a person, one line per capability. `pryxd
   doctor` checks what the node's own runtime needs (Landlock for process, the distribution for
   wasm) instead of Docker's facts on every node, and agrees with `serve` about data ownership.
3. Announced facts. The node's features gain what the probe proved, additively: `host-firewall`
   only while the table is loaded (serve checks at start and on the heartbeat's timer),
   `browser-sandbox` while user namespaces are usable, the Landlock ABI as `landlock-abi-N`,
   `memory-sleep` only where sleep keeps processes (not wasm), `memory-limit` and `cpu-limit`
   where the kernel enforces them, `separate-users` for the process tier's uid pool.
4. Guarantees, in control and the public API, additively: a sandbox's view gains `guarantees`
   (the names in docs/any-host.md 4.2, each true or false); create gains an optional `require`
   list; a deployment default comes from PRYX_REQUIRE. Placement refuses a node lacking a
   required guarantee and the refusal names it. OpenAPI and both SDKs follow.
5. No silent drops. control/src/workload-api.ts refuses instead of leaving out `egress`,
   `network`, `user`, `workdir`, `packages` when the target node lacks the feature, on create
   and on wake alike. A test wakes a sandbox on a node that lost the feature.
6. Installer messages. `pryx up` on a host it can't serve says which requirement is missing
   and what to run, before it changes anything; the architecture is checked first.
7. The documentation's six disagreements with the code listed in docs/any-host.md section 1
   and in the research are corrected.

Constraints

- The node protocol, the public API and the edge protocol only add.
- No behaviour changes for a request that names no `require` on a host where every guarantee
  holds; a test pins that.
- The probe changes nothing on the host and needs no privilege; what it can't test as this user
  it reports as unknown with the reason, never as off.
- New Rust dependencies have their licences recorded first; no libseccomp (LGPL), the existing
  seccomp builder stays.
- Title under 50 characters, no motive, no `@`.

Done when

- `pryxd probe --json` and `pryx doctor` exist, with unit tests over fixture trees and a test
  per capability that runs for real in CI's runner and in the ordinary-image job.
- A sandbox's `guarantees` are true in the end-to-end suite for every one the suite then
  attacks: a write outside the sandbox, a signal to pryxd, a connection the firewall forbids.
- A create that requires what no node has answers `no_room` naming the guarantee; a process
  node without root is refused for `separate-users`; both have tests.
- `bun run check` and `cargo test --locked` pass; CI green on the pull request and on main.
```
