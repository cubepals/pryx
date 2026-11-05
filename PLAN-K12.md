# K12: one image that runs anywhere, without root

This is milestone K12 of PLAN.md. Everything under "How to run this plan" in PLAN.md applies to
it: read AGENTS.md first, work autonomously, never ask, record choices in `docs/decisions.md`,
commit with short subjects and no `@`, never claim a check passed that did not run.

```
Goal

Make pryx run as one container image, `docker run` and nothing else, on any platform that runs
containers and gives no privileges: no Docker socket, no root on the host, no new namespaces.
It still gives sandboxes, with a lighter wall, and says so. The same image on a host with
Docker gives the container tier. Hosted platforms that run plain containers, a laptop, and a
compose file all become places pryx runs from the first release.

Context

Read AGENTS.md, PLAN.md (K6 for `pryx up` and the one-box layout, K11 for pryx-agent, which
this milestone builds on), src/runtime.rs and src/runtime/ (the container runtime seam and the
fake), src/isolation.rs, src/tree.rs, src/manager.rs (freeze and resume, snapshots), agent/,
control/src/main.ts, edge/src/main.ts, Dockerfile, and docs/architecture.md (the isolation
levels: container, gvisor, microvm).

Facts this milestone is built on, to verify first and record in docs/decisions.md: inside an
unprivileged container with the default seccomp profile, there is no container engine socket,
no cgroup to write, and no creating of user, mount, pid or network namespaces, so no nested
engine of any kind can run; an unprivileged process can still restrict its own filesystem
with Landlock (kernel 5.13 and later; network rules from 6.7), its own syscalls with seccomp,
its resources with rlimits, and can be stopped and continued with signals; a process inside a
container that runs as the container's root can change to other uids (CAP_SETUID and
CAP_SETGID are in the default capability set) without being root on the host.

What is built:

1. The process tier. A second runtime behind the daemon's runtime seam, `process`: a sandbox
   is a directory under the state directory plus a process tree run by pryx-agent as its
   supervisor. For each sandbox: a uid and gid from a configured pool (or, when the daemon
   cannot change uid, one shared uid, announced); a root filesystem that is pryx's own base
   image (below), read-only; the sandbox's data directory, read-write; Landlock rules that allow
   exactly those and nothing else; a seccomp filter that denies mount, ptrace, keyctl, bpf,
   module and namespace syscalls and the rest of the list in docs/process-tier.md; rlimits for
   address space, open files, processes and CPU time; a watchdog in the agent that kills the
   tree past its memory budget by resident size, sampled, since no cgroup is available; `nice`
   for the CPU share; `network: none` by Landlock network rules where the kernel has them and by
   a seccomp denial of connect, bind and sendto otherwise, each announced. Sleep is SIGSTOP of
   the tree with memory kept, wake is SIGCONT; stop is SIGTERM then SIGKILL; snapshot, export,
   restore and fork are the data directory, with the same safe walk as today. A spec with an
   `image` is refused on this tier with 422 invalid_request, field `image`, problem "this host
   runs the process tier; images are not available"; the sandbox's `isolation` reads `process`
   and the API, CLI and SDKs show it. Public ports are the agent's forwards to spare ports as
   in K11, bound by the agent on the container's address.

2. The base image. One versioned userland for the process tier, built in this repository and
   bundled in the pryx image: a small Debian-based root with sh, coreutils, git, curl, python3,
   node, and build-essential; its version is part of pryx's release and listed in
   docs/process-tier.md. A template on this tier is a snapshot of a prepared data directory,
   as on the container tier.

3. One image. `ghcr.io/cubepals/pryx:<version>`: control, edge, the daemon, pryx-agent and
   the base image, with one entrypoint that starts them all, makes the fleet CA and the
   operator, edge and store credentials on first start into a state volume, prints the first
   API key once, enrolls the local daemon, and chooses the tier: container when a Docker socket
   is mounted and usable, process otherwise, printed on start. Postgres from DATABASE_URL when
   given; otherwise an embedded Postgres under the state volume, so `docker run` alone works.
   The object store likewise: S3_* when given, otherwise a local store under the volume, with
   rest and restore working on it. The edge inside the image serves the HTTP front on the
   container's port; TLS is the platform's, so TLS and raw port fronts are off and documented.
   The image runs as root inside the container by default so it can change to per-sandbox uids;
   started with `--user`, it runs with one uid and announces the weaker isolation. The same
   image is what `pryx up` installs on a host, so there is one build.

4. Ways in. docker-compose.yml at the root for a laptop; a template for one hosted platform
   that runs plain containers with a Postgres add-on, chosen as the one with public template
   support and documented in docs/deploy.md with the exact steps; the Dockerfile for anyone
   else. `pryx up --image` installs from the image rather than from release binaries, as an
   option.

5. Honesty in the product. docs/process-tier.md states exactly what the process tier is and
   isn't: the kernel is the host container's; filesystem and syscall isolation are kernel
   enforced; memory is a budget enforced by sampling, not a hard cap; network off is a filter,
   not a namespace; a kernel escape from a process is an escape from the container. A node
   on this tier announces `isolation: ["process"]` and placement refuses a sandbox that asks
   for `container`.

6. Measurements, in docs/speed.md: on a GitHub runner, `docker run` of the image with no flags,
   time from start to the API answering; `pryx run` of `echo hello`; `box.run('true')` through
   the agent on the process tier; sleep and wake by signal; snapshot and fork of 100 MB. Same
   budgets as K11 for the agent path; start of the image under 10 s.

Constraints

- The container tier is unchanged in behaviour; the fake runtime's tests still pass; the
  process tier is a third implementation of the same trait, with its own tests that run
  unprivileged in CI.
- CI runs the image with `docker run` and no privilege flags on a plain runner, with the
  default seccomp profile, and the quickstart's commands against it; that job is the proof.
  Never add `--privileged`, a socket mount or a seccomp override to make it pass.
- No secret is printed but the first API key, once; credentials live in the state volume with
  mode 0600.
- Title under 50 characters, no motive, no `@`.

Done when

- `docker run -p 8080:8080 ghcr.io/cubepals/pryx:<version>` on a plain runner answers the
  API, and `pryx run --image` is refused with the documented message while `pryx run` without an
  image prints hello, in CI, from the build's image.
- docker-compose.yml works on a laptop as written; docs/deploy.md's hosted steps are complete.
- docs/process-tier.md and docs/speed.md are written with the measurements and limits.
- `bun run check` passes; CI green.
```
