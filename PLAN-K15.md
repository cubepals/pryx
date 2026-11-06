# K15: one surface

This is milestone K15 of PLAN.md. Everything under "How to run this plan" in PLAN.md applies to
it: read AGENTS.md first, work autonomously, never ask, record choices in `docs/decisions.md`,
commit with short subjects and no `@`, never claim a check passed that did not run. This is a
deletion milestone: it removes more than it adds, and nothing it removes has a user.

```
Goal

pryx has no users and no old versions in the field, so it carries no compatibility. Delete every
layer, path and sentence that exists for an older client, an older node or an older API, until
each SDK has one surface, every node runs the agent, a command starts when it is called, and a
stranger reads the README in a minute and the SDK's README in two.

Context

Read AGENTS.md, PLAN.md, K9 and K11, sdk/typescript/src/ (index.ts, api.ts, handles.ts, run.ts,
agent.ts, files.ts), sdk/typescript/README.md, sdk/python/ (the same shape), cli/src/,
control/src/ (the public API's exec, logs and files routes and their forwarding), docs/ and
README.md.

What is deleted:

1. The thin client. `pryx.api` and the `Api` class the handle extends; the methods `make`,
   `exec`, `putFile`, `getFile`, `action`, `list`, `get`, `delete`, `snapshot`, `snapshots`,
   `deleteSnapshot`, `fork`, `endpoints`, `events`, `template`, `templates` on `Pryx`; the old
   `{baseUrl, apiKey}` constructor; `pryx.agent(id)` as a public export. The handles call HTTP
   through a private transport. Raw access is the OpenAPI document, said once in the README.
2. The older-node paths. Exec and logs through control, files through exec and tar and base64,
   the 48 KiB bound, the `exec-stream` node feature's client side, and every branch that asks
   "does this node have the agent". Every node runs the agent, as K11 made true; the SDK, the
   CLI and control assume it. The public API's exec, logs and files routes stay for the CLI
   and curl, implemented by forwarding to the agent, with nothing else behind them. The node
   protocol keeps the old endpoints (within v1 a change only adds); the clients stop using them.
3. The sentences. Every "on agent nodes", "on older nodes", "requires sh, realpath, dd, tar",
   "preceding nodes", "previous top-level methods still work" in any README or doc.
4. The Python SDK's mirror of all of the above.

What changes shape:

5. Eager commands. `box.run(cmd)` starts the command when called and resolves to the result;
   `box.stream(cmd)` starts it when called and is an async iterator of `{stream, data}` with
   `.lines()` and `.result()`; neither is lazy, neither implements `then` by hand; the Run
   class and its PromiseLike go. A command runs once per call, with an idempotency key the SDK
   makes.
6. One name per thing. `box.start()` with no argument starts the sandbox; `box.spawn(cmd)`
   starts a process and returns it; `box.start(cmd)` is removed. `box.shell` is reserved for
   K16. Python: `box.run`, `box.stream`, `box.spawn`.
7. The README. The root README is the install line, the four-command quickstart, one
   TypeScript example of ten lines, the three tiers in three lines, and links. Under 60 lines.
   Each SDK README opens with the ten-line example and lists the surface as a table, one row
   per method, one line each; under 120 lines. Operations, architecture, protocol, tiers,
   speed, checks and decisions stay under docs/, with speed cohorts and check reports moved
   under docs/checks/ and linked from one line in docs/README.md.

Constraints

- The public HTTP API and the node protocol are unchanged (additive within v1); only clients
  and docs change. The OpenAPI documents are regenerated and show no change.
- Both SDKs' READMEs' examples run in CI against the end-to-end control, as before.
- The K11 end-to-end suite and the K14 tier suite pass unchanged in behaviour, on the new
  surface.
- Lines deleted exceed lines added, shown in the pull request as the diff's totals.
- Title under 50 characters, no motive, no `@`.

Done when

- `grep -rn -i "older node\|agent node\|preceding node\|thin client\|previous top-level" sdk
  docs README.md` prints nothing.
- Each SDK exports exactly: `Pryx`, `Sandbox`, `Snapshot`, `Process`, `Terminal`, `Files`,
  `PryxError` and the option and result types; nothing else.
- `box.run('true')` with no await still runs; proven by a test that observes the side effect.
- README.md under 60 lines, each SDK README under 120, both run in CI.
- `bun run check` passes; CI green.
```
