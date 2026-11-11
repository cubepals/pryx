# K28: one click on a hosting platform

This is milestone K28 of PLAN.md. It follows the any-host plan (docs/any-host.md) and starts
after K27. Everything under "How to run this plan" in PLAN.md applies to it: read AGENTS.md
first, above all "How pryx decides"; work autonomously, never ask, record choices in
`docs/decisions.md`, commit with short subjects and no `@`, never claim a check passed that did
not run. Licence first, as before.

```
Goal

A person with an account on a hosting platform presses one button and has pryx running at a
public HTTPS address, having typed nothing. Opening that address shows a welcome page: pryx
runs here, where its key is, and how to connect each kind of caller. One button per major
platform, and one file for any rented machine.

Context

Read docs/any-host.md sections 4.1, 4.2 and 4.7 (the probe, guarantees, presets), docs/deploy.md
(the release image, its one public port, PRYX_PUBLIC_URL, the first key in the log, the
existing Render Blueprint), render.yaml, Dockerfile, deploy/, control's public listener and
its use of PRYX_BOOTSTRAP_API_KEY, docs/mcp.md, action/action.yml, both SDK READMEs. Then, at
recorded dates, each platform's own documentation for its deploy button or template format,
generated secret variables, persistent volumes, health checks, the user and filesystem a
service runs with, and what its kernel allows an unprivileged service (Landlock, seccomp,
user namespaces): Railway, Render, Fly.io, DigitalOcean App Platform, Koyeb, Coolify, and
cloud-init for a rented machine.

What is built

1. The first key comes from the platform. Where a platform can generate a secret variable,
   the template asks it to, and pryx takes that value as its first key. The key is then in
   the owner's own dashboard and nowhere else. Where a platform can't, the key is printed
   once in the log, as now.
2. A welcome page at the root of the public address. Before a key is entered it says only
   that pryx runs here, its version, and where this install's key is, by name: which
   variable in which settings page, or the first lines of the service log. It never shows a
   key or any part of one.
3. The same page after the key is entered, held in that browser tab only: what this host can
   do and what it can't, with the reason (the probe's report), and one section per caller with
   a block to copy that already carries this install's address: TypeScript, Python, the CLI,
   the GitHub Action, agents over MCP, and plain HTTP. Blocks name the key's variable; they
   never contain the key.
4. One directory per platform under deploy/platforms/, holding the file that platform reads,
   and docs/hosting.md with one section each: the button, what pryx gives there by tier and
   guarantee, and what it can't with the reason. The preset is chosen from the host, as
   everywhere. render.yaml stays where Render reads it and gains the generated key.
5. A cloud-init file for any rented machine: the one command, as root, with the server
   preset, ending at the same welcome page.
6. Where a platform's template is published from an account rather than a file, everything
   that account needs is prepared and written down as a short list of fields to paste. No
   account is created and nothing is published.

Constraints

- No secret is ever served to a caller that hasn't presented it. The page's public half
  reveals nothing about the host beyond the version.
- A platform whose kernel can't give a tier's policy gets that tier off, with the reason, on
  the welcome page and in docs/hosting.md. Never a weaker tier under the same name.
- No platform account, paid plan or marketplace listing is created. No price is written in
  the repository; docs/hosting.md links to each platform's own pricing.
- The page is served by control, with no new service, no build step at install and no
  script or font fetched from another origin.
- Nothing in the node protocol, the public API or the edge protocol is renamed or removed.
- Title under 50 characters, no motive, no `@`.

Done when

- In CI, for each platform: its file passes that platform's own validator where one can run
  without an account, and the release image starts under that platform's recorded
  constraints (its user, its filesystem, its port variable, one volume, no added
  privilege), serves the welcome page, accepts the generated key and runs `hello`.
- A test asserts the page's public half contains no key, and that the unlocked half is
  refused without one.
- Every block on the welcome page is run in CI against that same instance, so a block that
  stops working fails the build.
- The cloud-init file is run on a booted distribution in the scheduled matrix and ends at
  the welcome page.
- docs/hosting.md says, per platform, what ran in CI, and marks a live deploy as not run
  until one is recorded with its date.
- `cargo test --locked` and `bun run check` pass; CI green on the pull request and on main.
```
