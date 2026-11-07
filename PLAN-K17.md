# K17: the code interpreter

This is milestone K17 of PLAN.md. Everything under "How to run this plan" in PLAN.md applies to
it: read AGENTS.md first, work autonomously, never ask, record choices in `docs/decisions.md`,
commit with short subjects and no `@`, never claim a check passed that did not run. Licence
first: every dependency pulled in here is checked and recorded before it is used, copyleft stays
a dependency and is never copied, and the kernel protocol is reused, not reinvented.

```
Goal

Give a sandbox a live language kernel, so that a caller sends snippets, not files, into a
process that keeps its variables between calls, and gets every value back in its richest form:
a chart as an image, a table as rows, a number as a number, an error with its traceback apart.
Python first, JavaScript second, each in a template with the usual libraries preinstalled, over
the same agent and SDK handle as everything else.

Context

Read AGENTS.md, PLAN.md, K11 (pryx-agent and `/_pryx/`), K15 (the one surface), K16
(templates from code, `spawn`, `waitForPort`), agent/src/, sdk/typescript/src/, sdk/python/,
cli/src/, control/src/templates*, docs/api.md and docs/agent.md. Then, at a recorded commit
and version: the Jupyter kernel messaging protocol (jupyter-client's documentation, the
`execute_request` and `execute_reply` messages, `display_data`, `execute_result`, `stream`,
`error`, and the `iopub` channel), ipykernel and its licence, and a JavaScript kernel (ijavascript
or tslab, whichever is maintained and permissively licensed; record which and why).

What is built:

1. In the agent: a `/_pryx/kernels` API. `POST` starts a kernel of a language in the sandbox
   (the kernel's process is a sandbox process like any other, visible in `processes()`),
   `POST /{id}/execute` runs a snippet and streams events as NDJSON as the kernel produces
   them: `stream` (stdout or stderr text), `result` (the final value, with every MIME form the
   kernel gave: text/plain, text/html, image/png, image/svg+xml, application/json), `display`
   (the same shape, for things shown mid-snippet), `error` (name, value, traceback as a list),
   and `done` (execution count, status). `POST /{id}/interrupt`, `POST /{id}/restart`, `DELETE
   /{id}`, `GET` lists kernels. The agent speaks the Jupyter wire protocol to the kernel over
   ZeroMQ or the kernel's documented transport; it never shells out to `jupyter`. A kernel that
   dies is reported as such, not hung. One kernel per language per sandbox by default;
   `contexts` as named kernels for callers who want separate state.
2. Templates, built in this repository with K16's template-from-code and published with the
   release: `python-interpreter` (Python 3 with ipykernel, numpy, pandas, matplotlib, scipy,
   scikit-learn, requests, pillow, and an agg backend so plots render without a display) and
   `node-interpreter` (Node with the chosen JavaScript kernel). `pryx template create
   python-interpreter` warms it. A sandbox made from either has its kernel already started.
3. The SDK, both languages, on the handle:
     const py = await box.python()                       // starts or reuses the kernel
     const r = await py.run('import pandas as pd; df = pd.read_csv("sales.csv"); df.describe()')
     r.text, r.html, r.json, r.png, r.svg                // whichever the value had
     r.stdout, r.stderr, r.error                         // error: {name, value, traceback}
     for await (const event of py.stream('for i in range(3): print(i)')) …
     await py.interrupt(); await py.restart(); await py.close()
     const ctx = await box.python({ context: 'analysis' })   // separate state
     const js = await box.javascript()
   Python SDK: `box.python()`, `ctx.run(...)`, `r.png` as bytes, sync and async. The CLI gets
   `pryx sandbox python -c '...'` and `pryx sandbox python` as a REPL over the kernel.
4. MCP: the per-sandbox MCP server from K13 gains `run_python` and `run_javascript` tools whose
   results carry images as MCP image content, so an agent client shows the chart.
5. Measurements in docs/speed.md, n=30, p50 and p90: first `run` after `box.python()` on the
   warm template; a trivial `1+1`; a snippet that renders a matplotlib chart to PNG; kernel
   restart. Budget: `1+1` under 50 ms through the agent over loopback; a chart under 1 s.

Constraints

- Nothing in pryxd, control or the edge changes for this; it is the agent, templates, the SDKs
  and the CLI. The node protocol and the public API are unchanged, except the OpenAPI document
  for the agent's new routes.
- The kernel runs as the sandbox's user with the sandbox's limits; its sockets bind only inside
  the sandbox; the agent is the only thing that speaks to it.
- Each dependency's licence is recorded in docs/decisions.md before use; the templates' image
  layers list their licences in the template's README.
- Unit tests for the event mapping against recorded kernel messages; the end-to-end suite in
  CI starts a sandbox from each template, runs a snippet that keeps state across two calls,
  renders a chart and asserts a PNG comes back, raises an error and asserts the traceback,
  interrupts a loop, and restarts; both SDKs; the MCP tool returns an image.
- Title under 50 characters, no motive, no `@`.

Done when

- The SDK example above runs in CI on both templates, both SDKs.
- `pryx sandbox python -c 'print(2+2)'` prints 4 against the end-to-end control.
- docs/interpreter.md documents the kernels, the event shapes, the templates and their
  libraries with versions, and the measurements.
- `bun run check` passes; CI green.
```
