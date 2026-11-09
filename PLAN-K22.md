# K22: any container engine

This is milestone K22 of PLAN.md, part of the any-host plan (docs/any-host.md). Everything under
"How to run this plan" in PLAN.md applies to it: read AGENTS.md first, above all "How pryx
decides"; work autonomously, never ask, record choices in `docs/decisions.md`, commit with short
subjects and no `@`, never claim a check passed that did not run.

```
Goal

The container tier works on whatever Docker-compatible engine the host already has, with or
without root: rootful Docker as today, rootless Docker, and Podman through its compatible
socket. What such an engine can't enforce is reported as a guarantee that is off, never
ignored the way the engine itself ignores it.

Context

Read docs/any-host.md sections 3.2 and 4.4, K19's probe and guarantees, K21's router,
src/runtime/docker.rs and src/runtime/docker/ (create, container, egress, native_tools),
src/doctor/docker.rs, src/isolation.rs, src/store.rs (data ownership), control/src/power.ts
(the stop fallback when suspend is refused), docs/decisions.md's notes on rootless Docker
(no freezer, no memory enforcement, vfs storage). Then, at recorded versions: Docker's
rootless documentation and its list of limitations, Podman's Docker-compatible API and its
release notes.

What is built

1. Discovery, in the probe: DOCKER_HOST, the system socket, the user's Docker socket and the
   user's Podman socket, in that order, each asked for `version` and `info`. The first that
   answers is the engine; the configuration can name one.
2. What `info` says becomes guarantees: rootless or not; the cgroup driver (none means
   `memory-limit` and `cpu-limit` are off); the freezer (`memory-sleep`); the storage driver
   (vfs is reported as slow, with the one fix); whether the firewall can exist
   (`host-firewall` needs root's table); whether ports below 1024 can be published.
3. Requests against an engine that can't enforce: a create that names a CPU or memory limit
   there is refused with the reason, as CPU limits on the process tier are; a create that
   names none runs and its `guarantees` say the limits are off.
4. Data ownership without root: on a rootless engine the sandbox's data belongs to the
   daemon's own user and is mapped into the container as the sandbox's user by the engine;
   `workloads.data_owner` is inferred, not required.
5. Sleep on an engine without the freezer takes the stop path at once instead of asking and
   being refused, and the sandbox reports `memory-sleep` off before anyone sleeps it.
6. Podman's differences from Docker's API are handled where the runtime reads answers
   (src/runtime/docker.rs is the one place Docker's answers are read): the errors the rest
   of the crate matches stay RuntimeError's variants.
7. `pryx doctor` says, for an engine it found, what is off and the one thing that would turn
   each on: delegate the cpu controller, install `uidmap`, use the packaged binary on Ubuntu.

Constraints

- Nothing changes for rootful Docker: the hardening, the firewall and the measured budgets
  stay; the existing Docker suite passes untouched.
- pryx installs no engine in the `user` preset and drives no OCI runtime itself.
- The egress hostname policy, bucket mounts and the store lease stay root-only and are
  refused elsewhere with the guarantee named.
- An engine that stops answering makes the tier unavailable, not the node: the other tiers
  keep serving.
- Title under 50 characters, no motive, no `@`.

Done when

- The Docker test suite runs against rootless Docker and against Podman's socket in CI,
  as the runner's own user, with the differences above asserted rather than skipped.
- An end-to-end case on each: create from an image, run, a port, snapshot and fork, sleep
  and wake by the stop path, with `guarantees` matching what the suite then attacks.
- A create naming a memory limit on an engine without cgroups is refused naming
  `memory-limit`; a test covers it.
- docs/tiers.md gains the rootless engines' measured create, exec and network figures;
  `cargo test --locked` passes; CI green on the pull request and on main.
```
