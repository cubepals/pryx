# K26: stronger without KVM

This is milestone K26 of PLAN.md, part of the any-host plan (docs/any-host.md). Everything under
"How to run this plan" in PLAN.md applies to it: read AGENTS.md first, above all "How pryx
decides"; work autonomously, never ask, record choices in `docs/decisions.md`, commit with short
subjects and no `@`, never claim a check passed that did not run. Licence first: gVisor and
bubblewrap are checked and recorded before use; bubblewrap runs as a separate, unmodified
program.

```
Goal

On hosts without KVM, two steps up from what they have today: a `gvisor` tier for hosts with a
container engine, and namespaces for the process tier wherever the host allows user namespaces.
Both wrap an existing program. Neither changes what a request for another tier gets.

Context

Read docs/any-host.md sections 3.1, 3.2 and 4.2, K19 (probe, guarantees), K21 (router), K22
(engines), src/runtime/docker/ (container.rs has no runtime field today), src/runtime/process/
(power.rs launches the agent), agent/src/process_policy.rs, docs/process-tier.md. Then, at
recorded versions and licences: gVisor's runsc as a Docker runtime, its systrap platform, its
compatibility notes and its rootless guide; bubblewrap 0.13 (no setuid mode; needs user
namespaces) and Ubuntu's AppArmor profiles for it on 24.04 and 26.04.

What is built

1. The `gvisor` tier: a container started with the engine's `runsc` runtime. The probe finds
   whether the engine has it registered; the node announces `gvisor` only then. The `server`
   preset installs and registers runsc from its pinned release; the `user` preset uses it
   where someone already has. Everything else is the container backend: images, limits,
   ports, the firewall.
2. What gVisor doesn't run is found by the suite, not guessed: each native tool and the
   interpreter templates run against it, and each that fails is refused on this tier by
   capability with the reason.
3. Namespaces for the process tier: where the probe proved user namespaces with capabilities,
   the process tier starts each sandbox through bubblewrap with its own mount, PID and IPC
   namespaces, a private /tmp, and, for `network: none`, its own empty network namespace.
   Landlock, seccomp and the limits stay inside it. The sandbox reports `private-pids` and,
   with network off, `private-network`. Where the probe did not prove them, the tier is
   exactly today's.
4. `pryx doctor` says, on Ubuntu 24.04 and later, that a profile root installs is what would
   turn namespaces on for this binary, and prints it; pryx doesn't install it in the `user`
   preset.
5. docs/tiers.md gains a column for gvisor and the namespaced process mode with measured
   figures; docs/process-tier.md says what each mode gives.

Constraints

- A request naming `container` never gets `gvisor` and the reverse.
- The namespaced mode is a guarantee, not a tier: a `process` request gets whichever the host
  gives, and a request that requires `private-pids` is placed only where it holds.
- No setuid helper is installed, and nothing is added to the seccomp filter that widens what
  an unwrapped process may do.
- The browser's sandbox keeps working inside the namespaced mode; a test covers it.
- Title under 50 characters, no motive, no `@`.

Done when

- In CI: the Docker suite's lifecycle cases pass on the gvisor tier with its refusals
  asserted; the process suite passes in namespaced mode on a runner that allows user
  namespaces and in plain mode in a default `docker run`.
- The guarantee tests pass: a namespaced process sandbox can't see another's processes or
  /tmp; a gvisor sandbox reports a kernel that isn't the host's.
- Measured, 30 samples each, in docs/tiers.md: create, agent `true`, a syscall-heavy command
  and a network transfer on both, beside the tiers they extend.
- `cargo test --locked` and `bun run check` pass; CI green on the pull request and on main.
```
