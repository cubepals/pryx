# K31: process-tier network policy in the kernel

This is milestone K31 of PLAN.md. It follows the any-host plan (K19 to K27) and builds on K26's
namespaces for the process tier. Everything under "How to run this plan" in PLAN.md applies to
it: read AGENTS.md first, above all "How pryx decides"; work autonomously, never ask, record
choices in `docs/decisions.md`, commit with short subjects and no `@`, never claim a check passed
that did not run. Licence first: libbpf and every header and tool used are checked and recorded
before use.

```
Goal

A process-tier sandbox with internet access gets the same egress and ingress policy a container
gets, enforced in the kernel, on every host where pryx may attach eBPF programs to a cgroup of
its own: root, or a user with CAP_BPF and a delegated cgroup subtree. Today such a sandbox shares
the host's network with no policy. Where the host can't give it, the guarantee stays off with the
reason, exactly as today.

Context

Read docs/any-host.md sections 3.1, 3.2 and 4.2, docs/process-tier.md, docs/compare.md,
src/runtime/process.rs and src/runtime/process/, agent/src/process_policy.rs (the seccomp and
Landlock policy), the egress policy the container tier enforces (src/runtime/docker/egress.rs and
the hostname gateways) and K19's probe and guarantees. Then, at recorded versions and licences:
libbpf and its CO-RE and skeleton docs; the kernel's cgroup BPF hooks (cgroup/connect4,
connect6, sendmsg4, sendmsg6, sock_create, cgroup_skb ingress and egress); the BPF verifier's
limits; and either libbpf-rs or aya as the loader from Rust.

What is built

1. A cgroup v2 directory per process sandbox, where the host allows one (root, or a delegated
   subtree under the user's systemd manager). The probe reports which; nothing else changes
   where it can't.
2. The kernel programs, in C, under bpf/: connect and sendmsg hooks that allow or refuse each
   destination by address, prefix and port from a per-sandbox map, a sock_create hook that
   refuses socket families the policy forbids, and cgroup_skb counters for traffic accounting.
   Built with clang for the BPF target, CO-RE, libbpf skeletons, `-Wall -Werror`, and the
   verifier as the safety proof. Every C file carries `SPDX-License-Identifier: GPL-2.0-only OR
   BSD-2-Clause` and declares the "Dual BSD/GPL" licence to the kernel; the Rust that loads them
   stays FSL.
3. The loader in pryxd: attaches the programs to the sandbox's cgroup at start, fills the policy
   map from the same egress specification containers use (public internet on or off, hostname
   policy through the existing gateways, explicit allow and deny prefixes and ports), detaches
   and removes everything at stop, and survives a daemon restart by finding its own pinned
   programs.
4. The node announces `network-policy` for the process tier only when the programs are attached
   and verified for that sandbox. A request that requires the guarantee on a host that can't give
   it is refused with the reason; it never runs with a weaker policy.
5. docs/process-tier.md, docs/tiers.md and docs/compare.md say what the process tier now gives,
   by host, and docs/bpf.md says how the programs work and how to read their counters.

Constraints

- No silent fallback: where the hook can't attach, the tier's network guarantee is reported
  off, not approximated.
- The C stays small: one job per program, no parsing of sandbox-provided data outside the
  kernel, no user-space C.
- Nothing in the node protocol, the public API or the edge protocol is renamed or removed.
- No new CI job without saying its cost; a job that adds more than a minute says so in the PR.
- Title under 50 characters, no motive, no `@`.

Done when

- In CI, as root and as an ordinary user with a delegated cgroup: a process sandbox with public
  internet off can't reach the internet, its host's loopback services, link-local or private
  ranges; with an allow rule it reaches exactly that destination; with hostname policy it reaches
  only the named host through the gateway; and a sandbox can't reach another sandbox's ports.
- On a host without the capability the guarantee is off with its reason, and a request that
  requires it is refused naming it.
- The programs load on the oldest and newest kernels in the CI matrix; a test fails when the
  verifier rejects a program.
- `cargo test --locked` and `bun run check` pass; CI green on the pull request and on main.
```
