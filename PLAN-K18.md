# K18: the rest of the field, and the comparison page

This is milestone K18 of PLAN.md. Everything under "How to run this plan" in PLAN.md applies to
it: read AGENTS.md first, work autonomously, never ask, record choices in `docs/decisions.md`,
commit with short subjects and no `@`, never claim a check passed that did not run. Licence
first, as in K17: every tool pulled in is checked and recorded before use, and nothing copyleft
is copied.

```
Goal

Close the remaining gaps between pryx and the hosted sandbox products, each by wrapping the
best open-source piece rather than writing one: a desktop a sandbox can show and an agent can
drive, SSH into a sandbox through the edge, an object-store bucket mounted inside a sandbox, a
browser template, and a public comparison page in the repository that says, feature by
feature and with a link to the code or the doc for each cell, what pryx has and what it
doesn't yet, so the claim "the most comprehensive open-source sandbox platform" is checkable
by anyone.

Context

Read AGENTS.md, PLAN.md, K11, K13 (MCP), K15, K16 (templates from code, `expose`, `spawn`),
K17 (the interpreter templates), agent/src/, edge/src/ (the port front and the TLS front),
control/src/ (templates, placement, the public API), the SDKs, cli/src/, docs/api.md,
docs/agent.md, docs/tiers.md. Then, each at a recorded version and licence: Xvfb and a
lightweight window manager, x11vnc or TigerVNC, noVNC and websockify; OpenSSH's server;
mountpoint-s3 and rclone; Playwright's official image and its browsers.

What is built:

1. Desktops. A `desktop` template: Xvfb at a configurable size, a small window manager, a VNC
   server, and noVNC, all started by the template's `start` command. `box.desktop()` returns
   `{ url }` for the noVNC page through `expose`, and `box.desktop.screenshot()` (PNG),
   `click(x, y)`, `type(text)`, `key(combo)`, `move(x, y)`, `scroll(dx, dy)` over xdotool in
   the agent, so an agent can drive it without a browser. The per-sandbox MCP server gains
   `screenshot`, `click`, `type` and `key` tools with image content. Measured: time from
   create on the warm template to a first screenshot.
2. SSH. `box.ssh()` returns `{ host, port, user, key }`: the agent starts an OpenSSH server
   inside the sandbox bound to a spare port with a key pair made for that call, control
   registers a port-front route through the edge (the raw TCP front from K5, with its hold
   and wake, so SSH into a sleeping sandbox wakes it), and the private key is returned once and
   never stored. `pryx sandbox ssh <id>` opens the session. Revoked by `box.ssh.revoke()` or
   when the sandbox sleeps past `idle`.
3. Mounts. `pryx.sandbox({ mounts: [{ bucket: 's3://name/prefix', path: '/mnt/data', readOnly,
   credentials }] })` mounts an S3-compatible bucket inside the sandbox through mountpoint-s3
   where the tier allows FUSE, and rclone's mount otherwise, with credentials passed to the
   agent at start and never written to the sandbox's data. On tiers where neither works, the
   create is refused with the tier named. `box.mounts()` lists them.
4. Browsers. A `browser` template from Playwright's official image, with `box.browser()`
   returning a CDP endpoint URL through `expose` and `box.browser.screenshot(url)` as a
   shortcut; measured from create to a rendered page.
5. The comparison page, docs/compare.md, linked from the README: one row per capability, one
   column each for pryx, E2B, Daytona and Cloudflare's sandbox SDK, with each pryx cell linking
   to the code or the doc that implements it, and each competitor cell citing the public doc
   page it was read from, with the date. Rows cover at least: create from image, from
   snapshot, from template; templates from code; run, stream, spawn, stdin, signals, PTY,
   shell sessions, waitForPort; files read, write, list, watch, find, grep, upload, download,
   sync; git; ports, preview URLs, tunnels; egress allowlist with DNS; internet off; sleep with
   memory, pause, auto-resume on request; snapshot, fork, move between hosts; TTL, idle, keep
   alive; labels, metadata, lookup; metrics; events, webhooks; code interpreter with charts;
   desktops and computer use; SSH; bucket mounts; browser; MCP server; GitHub Action; CLI;
   TypeScript and Python SDKs; self-hosted; runs without root; multi-tenant projects and
   quotas; machine providers and autoscale; open source and licence. An empty pryx cell reads
   "not yet" with a link to the plan file that will fill it; nothing is claimed that does not
   link to code that CI runs.

Constraints

- Each piece is a template, an agent route or an SDK method; nothing in pryxd, control or the
  edge changes except the public API's additive fields for mounts and the SSH route, with the
  OpenAPI document committed.
- Licences recorded in docs/decisions.md before use; the templates list their layers'
  licences; nothing copyleft is copied into this repository.
- Nothing weakens isolation: the VNC and SSH servers bind only inside the sandbox and are
  reached only through the edge with a sandbox token or the per-call key; mount credentials
  never touch the sandbox's data directory; the desktop runs as the sandbox's user.
- Unit tests per item; the end-to-end suite in CI takes a sandbox through: desktop screenshot
  and click, an SSH session that runs a command and wakes a sleeping sandbox, a mount that
  reads and writes an object, a browser screenshot of a page; both SDKs where a method exists;
  and a check that every link in docs/compare.md resolves.
- Title under 50 characters, no motive, no `@`.

Done when

- The four end-to-end cases pass in CI with their measurements in docs/speed.md.
- docs/compare.md exists with every row filled and every link checked, and the README links
  it in its first screen.
- `bun run check` passes; CI green.
```
