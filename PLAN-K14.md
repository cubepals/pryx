# K14: the agentOS tier, and the three tiers compared

This is milestone K14 of PLAN.md. Everything under "How to run this plan" in PLAN.md applies to
it: read AGENTS.md first, work autonomously, never ask, record choices in `docs/decisions.md`,
commit with short subjects and no `@`, never claim a check passed that did not run. This
milestone is built end to end: the tier, its place in every surface, the comparison and the
default, in this task.

```
Goal

Add agentOS (github.com/rivet-dev/agentos: a Linux-compatible sandbox in WebAssembly and V8,
with files, processes, shell, Python, JavaScript and networking, booting in milliseconds, with
no container engine and no root) as a third sandbox tier in pryx, `wasm`, behind the same
runtime seam as the container tier and the process tier, so that one pryx image offers all
three, and then measure the three against each other on one box and make the measured winner
the default for the image that runs without privileges.

Context

Read AGENTS.md, PLAN.md, K11 (pryx-agent and the hot path), K12 (the one image and the process
tier), K13 (agentOS's self-hosting boundary, if already read there), src/runtime.rs and
src/runtime/ (the seam: container, process, fake), agent/, control/src/ (placement by
`isolation`, templates), the SDKs and the CLI. Then agentOS at its current commit, recorded
in docs/decisions.md with the commit: its README, LICENSE (record what it permits; if it
forbids embedding or redistribution in a project under this repository's licence, stop this
milestone at a report in docs/agentos.md saying so, and build nothing), the core library
`@rivet-dev/agentos-core`, how the sidecar is started and spoken to (stdio and a Unix socket),
how it is distributed (a binary, a crate, an npm package), its API groups (process, terminal,
filesystem and mounts, javascript, typescript, python, contexts, network and preview URLs,
software packages, cron), its root filesystem options (in memory, chunked SQLite, chunked S3)
and what persists across recreation, how a VM sleeps, and its self-hosting guide (the control
plane and Rivet Services, which runs the agentOS actor and its VMs).

What is built:

1. The tier. A fourth implementation of the runtime trait, `wasm`: a sandbox is one agentOS VM
   owned by the daemon through the sidecar, with its root filesystem in chunked SQLite inside
   the sandbox's data directory, so that snapshot, export, restore and fork are the data
   directory as on the other tiers; sleep disposes or suspends the VM with its root kept and
   wake recreates it from it (measure both, keep the faster that preserves files); stop and
   start likewise; exec, processes, terminal, files, ports and network map onto agentOS's own
   API groups, with pryx-agent's API served over them so a client cannot tell the tier apart
   except by `isolation`. An `image` on this tier is refused with 422 invalid_request, field
   `image`, problem "this host runs the wasm tier; images are not available"; `software` from
   agentOS's packages is exposed as a spec field `packages` on this tier only, announced as a
   node feature. The daemon embeds the sidecar as a binary beside pryxd in the release and the
   image, verified by checksum at build; if agentOS is published as a Rust crate, link it
   instead and say which in docs/decisions.md.

2. Placement and the API. `isolation: 'wasm'` is a third accepted value; a node announces
   which tiers it provides; placement filters as for the others; the API, both SDKs and the CLI
   accept and display it; templates work on it; projects' limits count it in units like any
   other; the edge routes to its agent and its exposed ports.

3. The image. The one image from K12 ships all three tiers and chooses at start: container when
   a Docker socket is usable, else the default below. `PRYX_DEFAULT_TIER` overrides.

4. The comparison, in docs/tiers.md, measured on one box in CI's one-box job and on a plain
   runner with `docker run` and no privileges, each n=30 with p50 and p90: cold create to
   running; `run('true')` through the agent; first byte of `echo hi`; memory per idle sandbox
   and per running shell; sleep then wake; snapshot and fork of 100 MB; a `pip install` of one
   package and a `npm install` of one package where the tier can; 50 sandboxes made at once,
   time to all running and memory used. Beside the numbers, a table of what each tier can run
   (any image, native binaries, Python, Node, a shell), what isolates it (a kernel per sandbox,
   a virtual kernel, namespaces and cgroups, Landlock and seccomp), and what survives a sleep.

5. The default. Make the measured winner for the no-privilege image the default tier there,
   stated in docs/tiers.md with the numbers that decided it, and keep the other as an option.
   If agentOS's licence or its stability (crashes, missing features) rule it out, say so with
   evidence and keep the process tier as the default.

6. agentOS as pryx's guest, prepared for K13's upstream direction: document in docs/agentos.md
   how a self-hosted agentOS could place its VMs on pryx nodes through pryx's API, with the
   boundary read in K13, as a prepared patch under integrations/rivet/upstream/agentos/ if
   the boundary allows it; prepared, not sent.

Constraints

- The container and process tiers are unchanged; the fake runtime's tests still pass; the wasm
  tier has its own tests that run unprivileged in CI, against the real sidecar.
- Within protocol v1 a change only adds; every new field is behind a node feature.
- Nothing from agentOS is copied into this repository's source: it is a dependency at a pinned
  version with its checksum, under its own licence, with its notice kept.
- The agent's API is identical across tiers from the client's side; a test runs the K11
  end-to-end suite against the wasm tier unchanged.
- No secret of any project in the repository; nothing is sent to another project from inside
  the task.
- Title under 50 characters, no motive, no `@`.

Done when

- `pryx run` on the no-privilege image makes a wasm sandbox and prints hello in CI, in under
  100 ms from create to output at p50, or docs/tiers.md says what the number is and why.
- The K11 end-to-end suite passes on the wasm tier in CI, both SDKs.
- docs/tiers.md holds the full comparison and the chosen default with its reasons;
  docs/agentos.md holds the licence finding, the sidecar's distribution and version, the
  mapping of pryx's verbs onto agentOS's API, and the guest proposal.
- `bun run check` passes; CI green.
```
