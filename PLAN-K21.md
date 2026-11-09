# K21: several tiers on one node

This is milestone K21 of PLAN.md, part of the any-host plan (docs/any-host.md). Everything under
"How to run this plan" in PLAN.md applies to it: read AGENTS.md first, above all "How pryx
decides"; work autonomously, never ask, record choices in `docs/decisions.md`, commit with short
subjects and no `@`, never claim a check passed that did not run.

```
Goal

One pryxd serves every tier its host supports at once, so one install gives a person processes,
WebAssembly and, where an engine answers, containers, without choosing. A request names a tier
or leaves it open; a named tier is never replaced.

Context

Read docs/any-host.md sections 4.2 and 4.3, K19's probe and guarantees, src/runtime.rs (the
ContainerRuntime seam), src/main.rs (how one runtime is built), src/config/process.rs,
src/runtime/process.rs, src/runtime/wasm.rs, src/runtime/docker.rs, src/manager/,
src/protocol/spec/, src/protocol.rs, control/src/isolation.ts, placement.ts, node-choice.ts,
public-model.ts, runtime-engine.ts, deploy/image-tier.ts, docs/tiers.md, docs/protocol.md.

What is built

1. The spec gains an optional `isolation`, announced by the node as `multi-tier` and sent only
   to nodes that announce it. A node that receives none uses its default tier: today's
   behaviour, byte for byte.
2. `runtime = "auto"` (the default for new configurations) builds every backend the probe
   allows and routes each workload to its own behind ContainerRuntime: the tier is stored with
   the workload's record and never changes for its life. `runtime = "container" | "process" |
   "wasm"` keeps meaning exactly one. The node announces a feature per tier it serves.
3. Control reads a set of tiers per node. Placement matches the request's tier against the
   set; capacity, reservations and the host floor stay the node's, shared by its tiers.
4. An unnamed tier: control takes the first of `container`, `vm`, `process`, `wasm` that some
   healthy node serves and that meets `require`, and the sandbox's view says which was taken.
   PRYX_DEFAULT_ISOLATION still fixes one, and a list sets a different order. The image's own
   tier choice (deploy/image-tier.ts) is replaced by this.
5. `pryxd doctor` and `pryx doctor` list the tiers that are on and, for each that is off, why.
6. Per-tier limits that differ today stay per tier: CPU limits refused on process and wasm,
   images refused there, each with the message it has now.

Constraints

- The node protocol only adds; the `contract` job against Cubepals' main passes unchanged.
- Existing configurations with an explicit `runtime` behave exactly as before; a test pins a
  process-only and a container-only node.
- A workload's tier can't be changed by a later ensure; an ensure naming another tier for the
  same workload is refused with the tier named.
- The store-lease, the firewall and the native-tools settings keep applying to the container
  backend only.
- No new abstraction beyond the router: the three backends are not rewritten.
- Title under 50 characters, no motive, no `@`.

Done when

- The ordinary image with a Docker socket serves container, process and wasm sandboxes side
  by side in the end-to-end suite, each reporting its own tier and guarantees.
- A request naming a tier no node serves answers `no_room` naming the tier; a request naming
  none reports the tier it got; both have tests.
- The tier comparison's collectors (docs/tiers.md) still run and their budgets still pass.
- `cargo test --locked` and `bun run check` pass; CI green on the pull request and on main.
```
