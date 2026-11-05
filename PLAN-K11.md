# K11: the agent inside, and the fast path

This is milestone K11 of PLAN.md. Everything under "How to run this plan" in PLAN.md applies to
it: read AGENTS.md first, work autonomously, never ask, record choices in `docs/decisions.md`,
commit with short subjects and no `@`, never claim a check passed that did not run.

```
Goal

Put a small agent inside every sandbox and let clients talk to it directly, so that running a
command, streaming its output, keeping a process alive, opening a terminal, exposing a port and
watching files never pass through control or Postgres. Control does lifecycle: make, place,
sleep, wake, copy, delete. The agent does everything that happens inside. A trivial command
answers in milliseconds, and the sandbox surface matches what an agent builder expects: processes
with stdin and signals, a PTY over a websocket, live output, files of any size, a port exposed at
runtime with a URL, and internet on or off.

Context

Read AGENTS.md, PLAN.md (K3 for placement, K4 for the API and exec, K5 for the edge and its
routes, K9 for the SDK handle and the exec-stream feature, K10 for projects), src/ (the daemon:
runtime.rs, manager.rs, the exec path, ports.rs, isolation.rs, tree.rs), docs/protocol.md
(features, the execution lease, ports and audiences, secrets), control/src/ (workload-api.ts,
the public API, the edge routes), edge/src/ (fronts, the hold, routes), sdk/typescript/,
sdk/python/, cli/src/, and integrations/ (the Rivet sandbox-agent provider from K8, for the
shape of an in-sandbox agent API that agent frameworks already consume).

What is built:

1. pryx-agent. A static binary in this repository (a second crate in a Cargo workspace,
   `agent/`), built and released with pryxd. The daemon mounts it read-only into every container
   it starts and runs it as the container's first process; it runs the spec's entrypoint as its
   child, reaps, forwards signals, and exposes an HTTP and websocket API on one port that the
   daemon publishes with audience `edge` (the node protocol gains the feature `agent`,
   announced before control relies on it). The daemon passes it a per-sandbox secret through the
   spec's secrets; every request carries a sandbox token that control issues
   (`POST /v1/sandboxes/{id}/token`, scoped to that sandbox, with an expiry) and the agent
   verifies against the secret without calling anyone. The API, under `/_pryx/`:
   - processes: start (argv or a shell string, env, cwd, stdin on or off, pty on or off), list,
     get, write stdin, signal, resize, wait; one-shot run is start plus wait; output streams as
     NDJSON over HTTP or frames over a websocket, from the first byte, with bounded buffers and
     backpressure; a process outlives the client that started it.
   - terminal: a PTY over a websocket, with resize.
   - files: list, stat, read, write, mkdir, move, delete, of any size, streamed; watch a
     directory over a websocket; upload and download by URL with a short-lived token.
   - ports: expose a port and get its public hostname, unexpose, list; health.
   - mcp: reserved for K13.
   Where a sandbox image lacks a shell or coreutils, everything above still works: the agent
   needs nothing from the image.

2. The edge routes `https://<sandbox>.<PRYX_PUBLIC_DOMAIN>/_pryx/*` to the sandbox's agent port
   (a route per sandbox for the agent, beside the routes for its public ports), with the hold and
   wake as for any route: a request to a sleeping sandbox wakes it and completes. Websockets pass
   through. Nothing else of the sandbox is reachable on that hostname unless exposed.

3. Ports at runtime. The spec gains `sparePorts` (default 3, configurable): host ports the
   daemon publishes at create with audience `edge` and no target. `expose(port)` makes the agent
   forward a spare port to the target inside the container and makes control register the
   route and hostname (`<sandbox>-<port>.<domain>`); unexpose releases the spare. No restart, no
   spec change. Past the spares, expose answers 409 no_spare_port and the SDK says so.

4. Internet on or off. The spec gains `network: 'internet' | 'none'` (default `internet`): with
   `none` the sandbox joins an internal bridge with no route out, and the agent port is still
   reachable from the node. A node feature, `network-policy`, announced first. Per-host egress
   rules beyond that stay a later milestone.

5. Files through the daemon too. The node protocol gains `files` (a feature): read, write, list
   and stat of a sandbox's data, done by the daemon on the host with the safe walk in tree.rs,
   so files can be read and written while the sandbox is stopped or sleeping, and without the
   agent. The public API's files endpoints use it; the SDK uses the agent when the sandbox is
   running and the daemon otherwise, and the caller never knows.

6. Control stops carrying the hot path. The public API keeps `exec` and `logs` for
   compatibility, implemented by forwarding to the agent through the edge. The SDKs talk to the
   agent directly after one call to control for the token; they cache the token and renew it
   before expiry. `pryx sandbox exec`, `pryx sandbox terminal` (an interactive shell), `pryx
   sandbox expose`, `pryx sandbox cp` (both directions, directories too) use the same path.

7. The SDK surface, both languages, on the K9 handle:
     const p = await box.start('npm run dev', { env, cwd })   // Process: id, pid
     p.lines(), p.stream(), p.stdin.write(), p.signal('TERM'), await p.wait(), p.kill()
     await box.run('npm test')                                  // start + wait, as K9
     const term = await box.terminal({ cols: 120, rows: 40 })  // websocket PTY
     const url = await box.expose(3000)                         // 'https://sbx-…-3000.<domain>'
     await box.files.list('.'), box.files.watch('src')          // async iterator of changes
     await box.write('big.tar', stream), await box.readStream('out.log')
     await box.network('none')                                  // or at create
   The thin client stays as `pryx.api` and gains the agent's endpoints as `pryx.agent(id)`.

8. Measurements, in docs/speed.md, n=30, p50 and p90, on one box: `box.run('true')` through
   the edge to the agent; the same from the node's loopback; first byte of output of
   `echo hi`; `box.start` of a web server to `expose` to a 200 through the hostname; a
   websocket terminal round trip; `write` and `read` of 100 MB. Budgets: run under 20 ms through
   the edge over loopback; first byte under 10 ms; expose to 200 under 200 ms. Where a budget
   is missed after the work, say what remains.

Constraints

- Within protocol v1 a change only adds; every new behaviour is behind a feature the node
  announces; a control against an older node behaves as before.
- The agent is reachable only through the edge with a sandbox token, or from the node; it
  never trusts the image; it runs as the sandbox's user with the sandbox's limits; a sandbox
  cannot reach another sandbox's agent. Tokens are scoped to one sandbox and expire; the secret
  never leaves the node and the container.
- The hot path never touches Postgres: proven by a test that counts queries during run, start,
  terminal, files and expose.
- If K9 ran before this, its `exec-stream` node feature is kept for older nodes and the SDK
  prefers the agent; if K9 did not run, do not build exec-stream, build this.
- Unit tests for the agent (its API against a real process tree), the edge route, the token,
  spare ports and the network switch; the end-to-end suite in CI takes a sandbox through every
  item in 7, in TypeScript and Python, with output asserted to arrive before the command ends,
  a port exposed and fetched through the hostname, and a sleeping sandbox woken by a request to
  its agent.
- Title under 50 characters, no motive, no `@`.

Done when

- Every call in 7 works against the end-to-end control in CI, both SDKs, and `pryx sandbox
  terminal` gives a working shell.
- docs/speed.md holds the measurements and the budgets' results; the query-count test shows
  zero queries on the hot path.
- docs/agent.md documents the agent's API, the token, the edge route, spare ports and the
  network switch; control/README.md and docs/api.md are updated.
- `bun run check` passes; CI green.
```
