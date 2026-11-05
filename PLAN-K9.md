# K9: the SDK speaks, and exec is fast

This is milestone K9 of PLAN.md. Everything under "How to run this plan" in PLAN.md applies to it:
read AGENTS.md first, work autonomously, never ask, record choices in `docs/decisions.md`, commit
with short subjects and no `@`, never claim a check passed that did not run.

```
Goal

Make pryx as clean to use as it can be from the first release: the SDKs read like the thing they
do, a command is a string and a result is a value, output streams live, files are strings and
bytes, installing is one line, and the four-command quickstart is literally true and fast: a
sandbox from a template is running in about a second and a trivial command answers in tens of
milliseconds, measured.

Context

Read AGENTS.md, PLAN.md (K4 for the API, K6 for the Python SDK and the installer), README.md,
sdk/typescript/src/index.ts (the thin client: one method per endpoint, exec as a generator of
events, files as base64), sdk/typescript/README.md, sdk/python/, cli/src/, action/,
.github/workflows/release.yml, control/src/workload-api.ts and the exec path through control to
the node, control/openapi/pryx.openapi.json, docs/protocol.md (exec, its Idempotency-Key, and how
a node announces a feature before control may use it), and the node's exec implementation in
src/ (where output is buffered until the command ends).

The quickstart the README opens with, which must work as written against a release:

  curl -fsSL https://github.com/cubepals/pryx/releases/latest/download/install.sh | sh
  sudo pryx up --domain sandboxes.example.com
  pryx run --image alpine:3.22 -- echo hello
  pryx sandbox create --image alpine:3.22 --idle 5m

The install script: shipped with every release and attested like the other assets; detects
Linux x86_64 (and says plainly what it does not support), downloads the release's CLI and
SHA256SUMS, verifies the checksum, installs to /usr/local/bin or ~/.local/bin, and prints the
version. `pryx up` keeps its behaviour and prints, at the end, exactly the three environment
lines a shell needs and the next command to run.

What the TypeScript SDK becomes (`@cubepals/pryx`), with the thin client kept underneath as
`pryx.api` for anyone who wants the raw endpoints:

  const pryx = new Pryx({ url, key })
  const box = await pryx.sandbox({ image: 'node:22', idle: '10m', ttl: '2h' })
  const box = await pryx.sandbox('sbx_…')                  // an existing one, by id

  const r = await box.run('npm test')                      // a string runs through sh -c
  r.stdout, r.stderr, r.code, r.timedOut                   // the whole result, once
  await box.run(['node', 'app.js'], { timeout: '30s', env: { CI: '1' } })   // argv form, no shell
  for await (const line of box.run('npm run build').lines()) …             // live, line by line
  for await (const chunk of box.run('…').stream()) …                        // live, {stream, data}

  await box.write('src/app.ts', source)                    // string or Uint8Array
  const text = await box.read('out.log')                   // string
  const bytes = await box.readBytes('dist.tar')            // Uint8Array
  const snap = await box.snapshot()
  const twin = await snap.fork({ idle: '5m' })             // a Sandbox
  await box.sleep(); await box.wake(); await box.stop(); await box.start(); await box.rest()
  await box.delete()
  box.id, box.state, await box.refresh(), await box.endpoints(), box.logs({ follow: true })
  await pryx.sandboxes()                                   // Sandbox[]
  await pryx.run('node:22', 'node -e "console.log(1)"')    // make, run, delete, in one

Durations are strings ('10m', '30s', '2h') or numbers of seconds, in the SDKs and the CLI alike.
Errors are PryxError as today. Idempotency keys are made by the SDK for make and run unless the
caller passes one. Nothing is retried automatically, as today. The Python SDK (`cubepals-pryx`)
gets the same shape, Pythonic: `box = pryx.sandbox(image=…)`, `r = box.run("npm test")`,
`for line in box.run(…).lines()`, `box.write(path, data)`, `box.read(path)`, `snap.fork()`; sync
and async clients. The CLI's commands keep their names; `pryx sandbox exec` streams live once the
node can.

Live output: today the node buffers a command's output and control emits it when the command
ends. Add a node feature, `exec-stream`, announced in `features`, under which the node streams
stdout and stderr as they are produced (chunked, bounded buffers, backpressure) and control
forwards them as the same NDJSON events as they arrive; where the node lacks the feature, control
behaves as today. Within protocol v1 a change only adds.

Speed: measure, on one box with control, edge and the daemon together, each with n=30, p50 and
p90, and write docs/speed.md with the commands used: (1) `box.run('true')` round trip through the
API; (2) the same through control to the node without the API; (3) sandbox create to running from
a small image, cold and from a template; (4) sleep then wake (suspended), stop then start;
(5) snapshot of 100 MB of data; (6) fork from that snapshot. Then profile the paths and remove
what is slow that isn't the container runtime's own work (connection setup per request, JSON on
the hot path, polling where a response could be awaited, needless database round trips, waiting
on a heartbeat where the node's answer is already in hand), keep the client's connections alive,
and measure again. Budgets: (1) p50 under 50 ms over loopback; (3) from a template on a warm node,
p50 under 1 s to running; (4) wake from suspended, p50 under 1 s. Where a budget is not met after
the work, docs/speed.md says what remains, why, and what would meet it.

Files: `write` and `read` stay over exec and tar in this milestone but lose the base64 and the
48 KiB bound in the SDK (chunk, or stream through exec stdin if the node gains it; say which).

Constraints

- The public API stays additive within v1; the OpenAPI document is committed with every change;
  the thin client and every existing method keep working (`pryx.api.*`).
- Secure as today: a string command runs inside the sandbox through its shell and nowhere else;
  nothing new reaches the host; keys never logged; the install script verifies before it runs
  anything.
- Unit tests for the handle layer in both SDKs against a fake transport; the end-to-end suites in
  CI take a sandbox through the new API, in TypeScript and Python, with live output asserted to
  arrive before the command ends; the one-box CI job runs the quickstart's four commands from the
  build's artifacts.
- Title under 50 characters, no motive, no `@`.

Done when

- README.md opens with the four-command quickstart and each SDK's README opens with the example
  above, and both run in CI.
- `pryx sandbox exec` shows output as it happens against a node with `exec-stream`.
- docs/speed.md holds the six measurements before and after, and each budget's result.
- `bun run check` passes; CI green.
```
