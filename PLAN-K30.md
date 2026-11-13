# K30: pryx as a sandbox provider for Rivet and OpenOrange

This is milestone K30 of PLAN.md. It follows K29. Everything under "How to run this plan" in
PLAN.md applies to it: read AGENTS.md first, above all "How pryx decides"; work autonomously,
never ask, record choices in `docs/decisions.md`, commit with short subjects and no `@`, never
claim a check passed that did not run. Licence first, as before.

```
Goal

A project that takes its sandbox from Rivet's sandbox adapter, an agent harness in a Rivet
Actor or the Sandbox binding of an OpenOrange project, uses a pryx install as that sandbox by
adding one import, the install's address and its key. Nothing else in the project changes.

Context

Read sdk/ts (its README, exec with streaming, files, sleep and wake, ports and preview
addresses), docs/api.md, docs/edge.md, docs/providers.md, integrations/, and K19's
guarantees. Then, at recorded versions and dates, with each licence recorded first:
`@rivet-dev/sandbox-adapter` (its `SandboxProvider` and `Sandbox` interfaces, and how its
shipped providers behave on timeout, abort, suspend and a sandbox that no longer exists),
and rivet-dev/open-orange (its Sandbox binding, the methods it marks partial or unsupported
and why, and its Browser Rendering binding's Chrome DevTools endpoint rules). Behaviour is
read from them; no code is copied.

What is built

1. `pryxProvider(options)`, exported from the TypeScript SDK under its own subpath, an
   object that satisfies the adapter's `SandboxProvider`: `create`, `connect` (undefined
   when pryx reports the sandbox gone), `suspend` as pryx's sleep, `destroy`. The sandbox
   it returns gives `exec` with working directory, environment, timeout, abort and output
   as it arrives, and `readFile`, `writeFile`, `mkdir`, `stat`, `readdir`, `exists`.
2. Options are few and inferred: the address and key come from the SDK's usual variables,
   the tier is the host's default, the working directory has one default. A caller may name
   a tier, a template and a required guarantee; nothing else is exposed.
3. No runtime dependency on the adapter. Its types are a development dependency only, and
   CI fails when the provider stops satisfying them.
4. Ports: where the adapter or the binding asks for a sandbox port at a public address,
   it is pryx's own preview address through the edge. Written as a function the provider
   exports, so the binding's port methods have something to call.
5. The browser: if pryx's browser can be given as one Chrome DevTools address that accepts
   a new connection each time, a documented way to do that, checked in CI with a real
   connection. If it can't, the page says so and nothing is claimed.
6. docs/integrations/rivet.md and a connect page on the site: three steps and one block
   each, for an agent harness and for an OpenOrange project. An example under examples/.
7. Prepared, not sent: what OpenOrange needs to offer `pryx` as a named provider, as a
   patch file and a short description under docs/upstream/. No pull request, issue or
   comment is opened on another project.

Constraints

- Wrap, don't fork: the adapter and OpenOrange are used as published. pryx does not bundle
  or run OpenOrange, Rivet's control plane, or any part of them.
- Never a weaker guarantee in silence: a tier the host lacks is refused with the reason,
  through the provider as through the API.
- Nothing in the node protocol, the public API or the edge protocol is renamed or removed.
- No comparison with another provider beyond docs/compare.md. No price, no name of a person.
- Title under 50 characters, no motive, no `@`.

Done when

- In CI, against a real pryx started in the same job: every method of the provider; a
  timeout; an abort; output arriving before exit; suspend, then connect, with files intact;
  connect after destroy returning undefined; a required guarantee the host lacks refused.
- The adapter's own types compile against the provider at the recorded version, and a
  scheduled run does the same against its newest version and reports a break.
- The example runs in CI. Every block on the two pages is extracted and run.
- `bun run check` passes; CI green on the pull request and on main.
```
