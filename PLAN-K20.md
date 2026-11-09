# K20: one command for an ordinary user

This is milestone K20 of PLAN.md, part of the any-host plan (docs/any-host.md). Everything under
"How to run this plan" in PLAN.md applies to it: read AGENTS.md first, above all "How pryx
decides"; work autonomously, never ask, record choices in `docs/decisions.md`, commit with short
subjects and no `@`, never claim a check passed that did not run. Licence first: every program
or archive pulled in is checked and recorded before use.

```
Goal

Installing pryx is one command that asks nothing, on a host where the person has no root, no
Docker and no systemd as surely as on a server with all three. The installer finds out where
it is, takes a preset and starts; it ends by printing the URL, the key, what is on and what is
off with what would turn each on.

Context

Read docs/any-host.md sections 4.1 and 4.7, K19's probe, deploy/install.sh, deploy/image.ts,
image-tier.ts and image-state.ts (the release image's entrypoint: supervision, embedded Postgres
and object store, the first key), Dockerfile, cli/src/up.ts, up-host.ts, up-image.ts,
control/tools/release.ts, .github/workflows/release.yml, docs/deploy.md, docs/operations.md.
Then, each at a recorded version and licence: relocatable PostgreSQL binaries
(theseus-rs/postgresql-embedded or zonkyio/embedded-postgres-binaries), the object store the
image already embeds, the XDG base directory specification, systemd's user units and linger.

What is built

1. Presets. `pryx up` takes `user`, `container`, `server` or `node` from the probe: not root
   gives `user`; inside a container gives `container`; root with systemd gives `server`.
   `--preset` overrules, `--check` prints the plan and changes nothing. No preset asks a
   question. A domain is optional; without one sandboxes are reached through the path aliases
   the image already serves.
2. The `user` preset: the release image's entrypoint run on the host. A release bundle holds
   pryxd, pryx-agent, the control and edge binaries, the agentOS distribution, PostgreSQL and
   the object store. `pryx up` fetches it, verifies its digest and signature, unpacks under
   ~/.local/share/pryx, keeps configuration, state and sockets in the XDG directories, listens
   above port 1024, and supervises its children. Where a systemd user session exists it writes
   a user unit and turns on linger for itself; elsewhere it runs in the foreground and says how
   to keep it running. The supervision code is the image's, moved so both use it.
3. The `server` preset is today's `pryx up`, unchanged in what it installs, with its Docker and
   firewall steps no longer tied to apt: on a distribution it can't install for, it names the
   two packages and stops before changing anything.
4. install.sh runs `pryx up` after installing the CLI, so the documented command is one line.
   `curl … | sudo sh` is the server; `curl … | sh` is the user. PRYX_NO_UP=1 installs the CLI
   alone.
5. The summary: every preset ends with the URL, the key once, the tiers on, and each
   capability that is off with the one thing that would turn it on (from the probe). The `user`
   preset adds one sentence: its sandboxes share a user and have no enforced limits, so it is
   for one person's work, not for strangers' code.
6. docs/install.md: one section per kind of host (a laptop or VM without root, a server with
   root, inside a container, a platform that runs images, a node joining a fleet), each with
   its one command and what it gives. The README's first screen is that one command.

Constraints

- Nothing is written outside the home directory by the `user` preset, and nothing is
  downloaded that isn't pinned by digest in the release.
- PostgreSQL refuses to run as uid 0: the `container` preset as root keeps the image's
  dedicated user; a test covers it.
- Upgrades and uninstall work for the `user` preset: `pryx upgrade` replaces the bundle and
  keeps state; `pryx down --purge` removes everything it wrote and says what that was.
- The release grows by the bundle; say in the pull request by how much CI and the release
  lengthen.
- Title under 50 characters, no motive, no `@`.

Done when

- On a fresh unprivileged user in a booted Debian, Ubuntu, Fedora and Alpine system, and in a
  default `docker run` of each, the one command ends in a working `pryx run -- echo hello`
  with zero questions; the scheduled matrix workflow (docs/any-host.md section 7) runs these
  cells and records seconds to hello.
- The same command with sudo on a systemd host gives today's server, and the existing one-box
  job still passes.
- A host no preset can serve gets a message naming the requirement, and the test asserts the
  host is unchanged.
- docs/install.md exists and the README's first command is the one line; CI green on the pull
  request and on main, the release included.
```
