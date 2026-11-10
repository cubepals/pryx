# K24: desktops and SSH everywhere

This is milestone K24 of PLAN.md, part of the any-host plan (docs/any-host.md). Everything under
"How to run this plan" in PLAN.md applies to it: read AGENTS.md first, above all "How pryx
decides"; work autonomously, never ask, record choices in `docs/decisions.md`, commit with short
subjects and no `@`, never claim a check passed that did not run. Licence first: every program
in the bundle is checked and recorded before use; GPL programs run as separate programs.

```
Goal

A desktop an agent can see and drive, and SSH into a sandbox, on every preset: the same
`box.desktop()`, `screenshot`, `click`, `type`, `key`, `move`, `scroll`, `box.ssh()` and
`pryx sandbox ssh` as today, without Docker and without the edge's raw TCP front.

Context

Read docs/any-host.md sections 3.4 and 4.5, K23 (the tool bundle and private transports),
docs/native-tools.md, agent/src/tools/ (desktop, ssh, state), templates/desktop/,
templates/native-tools/, edge/src/ (front.ts, sandbox-proxy.ts, agent.ts and its HTTP-only
mode), deploy/image.ts (PRYX_EDGE_HTTP_ONLY), cli/src/ (the ssh command). Then, at recorded
versions and licences: Xvfb and TigerVNC's Xvnc, xdotool, noVNC and websockify (Unix socket
targets), OpenSSH's ProxyCommand and its server's options for a private socket directory.

What is built

1. First, a test that decides the design, recorded with its result: can the display server
   and its clients (Chromium headful, xdotool) use a socket under the sandbox's own directory
   instead of /tmp/.X11-unix, on the process tier as built? If yes, the desktop runs on the
   process tier everywhere. If no, the desktop on the process tier requires user namespaces
   (a private /tmp through bubblewrap, one program run unmodified), announces
   `desktop` only where the probe proved them, and is refused elsewhere with the reason.
2. The desktop in the tool bundle: the display server, xdotool, noVNC and websockify, added
   to K23's bundle with their notices. One display per sandbox, VNC and websockify on Unix
   sockets under the sandbox's directory, the same expiring capability in the URL as today.
   No window manager by default; the template environment can name one the bundle carries.
3. SSH over the agent's connection: `pryx sandbox ssh` and the SDKs' `ssh()` gain a transport
   that carries the session through the sandbox's authenticated WebSocket, used as OpenSSH's
   ProxyCommand, so SSH works wherever the HTTP edge does, the one-listener presets included.
   The raw front stays the default where it exists. Keys, expiry, revocation and wake on
   connect are as today.
4. The SSH server on the process tier: OpenSSH from the bundle, listening on a Unix socket
   under the sandbox's directory, as the sandbox's own user. In shared-uid mode it is refused
   unless the probe proved it can't reach a neighbour's files, and says why.
5. The node announces `desktop` and `ssh` from what the tier and the probe allow; placement
   uses them; `pryx doctor` lists them.
6. docs/native-tools.md and docs/compare.md follow.

Constraints

- Nothing a neighbouring sandbox can connect to or read, in any uid mode.
- Software rendering only; no device is opened that root would have to grant.
- X11 only; no Wayland path.
- The container tier's desktop and SSH are unchanged and their budgets stay.
- Title under 50 characters, no motive, no `@`.

Done when

- On the `user` preset in CI: a desktop screenshot, a click and typed text observed in an
  application, the noVNC page reached through the edge; an SSH session that runs a command
  and one that wakes a sleeping sandbox, both through the HTTP edge; 30 timed samples each
  in docs/speed.md.
- The design test's result is in docs/decisions.md, and the path it ruled out is refused
  with a tested message.
- The neighbour tests pass in shared-uid mode; both SDKs' desktop and SSH tests run on the
  process tier.
- `bun run check` and `cargo test --locked` pass; CI green on the pull request and on main.
```
