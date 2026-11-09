# Code interpreters

A language context is a supervised process inside an isolated native sandbox. Its variables
survive calls. The interpreter templates automatically start warming the default context at agent startup;
`box.python()` or `box.javascript()` reuses it and waits for its Jupyter-ready handshake. HTTP health stays responsive while scientific imports warm. Named contexts are independent. The process
appears in `box.processes()` and its id/PID appears in `box.kernels()`.

```sh
pryx template create python-interpreter
pryx template create node-interpreter
pryx sandbox python -c 'print(2+2)'
```

The CLI without an id creates and deletes a sandbox from the warmed Python template. With an
id it attaches to that sandbox. Without `-c` it keeps a multiline REPL; Jupyter's `is_complete`
reply decides when to submit input. Ctrl-C interrupts the kernel.

```ts
const box = await pryx.sandbox({ template: 'python-interpreter', isolation: 'container' })
try {
  await box.write('sales.csv', 'sales\n10\n20\n')
  const py = await box.python()
  const r = await py.run('import pandas as pd; df = pd.read_csv("sales.csv"); df.describe()')
  console.log(r.text, r.html, r.json)
  const chart = await py.run('plt.plot([1,2,3]); plt.show()')
  await Bun.write('chart.png', chart.png!)
  for await (const event of py.stream('for i in range(3): print(i)')) console.log(event)
  const analysis = await box.python({ context: 'analysis' })
  await analysis.close()
  await py.interrupt(); await py.restart(); await py.close()
} finally { await box.delete() }
```

Python has the same handles in synchronous and asynchronous client contexts. `result.png` is
`bytes`; TypeScript returns `Uint8Array`. Python `kernel.run` in an asynchronous context starts
an eager task, so an interrupt can be issued while awaiting that task. `stream` yields events
as they arrive. Every MIME bundle and its metadata remains available in `result.results` and
`result.events`; the convenience views search the last representation of each MIME type.
`stdout`, `stderr` and structured `error = {name, value, traceback}` remain separate.

## Agent contract

The [Rust-owned agent document](../agent/openapi.json) advertises `kernelsVersion: 1` on health,
independently of agent API version 2. Public control and node protocols are unchanged.

| Request under `/_pryx/kernels` | Effect |
| --- | --- |
| `POST {language: "python" or "javascript", context?: string}` | Start/reuse a context; return id, language, context, processId, pid, state. |
| `GET` | List contexts, including dead contexts. |
| `POST /{id}/execute {code, timeout?: seconds}` | NDJSON, one event per line, ending in `done`. |
| `POST /{id}/interrupt` | Signal the supervised kernel group independently of its execution queue. |
| `POST /{id}/restart` | Stop and reap the old kernel, then start a fresh process with the same context id. |
| `POST /{id}/complete {code}` | Jupyter input-completeness reply, for the REPL. |
| `DELETE /{id}` | Stop the context and remove it; idempotent after removal. |

| Event `type` | Other fields |
| --- | --- |
| `stream` | `stream: stdout or stderr`, `text` |
| `result` | `data` (all MIME representations), `metadata`, `executionCount` |
| `display` | `data`, `metadata`, nullable `displayId`, `update` |
| `error` | `name`, `value`, `traceback: string[]` |
| `done` | Nullable `executionCount`, `status` (`ok`, `error`, or an explicit transport/timeout state). |

Language errors are result data. Missing completion or a broken transport is an unknown outcome;
neither SDK retries code. A dead context stays dead until explicitly restarted. Executions are
serialized per context; interrupt/close do not wait on the execution mutex. Queues and capture
are bounded, timeouts enforce capability limits, and timeout kills the kernel instead of
allowing its code to continue silently. The default is 30 seconds, or the token's lower ceiling
when a project sets one; the SDKs send a timeout only when the caller names it, so the agent's
default applies otherwise. Each sandbox holds at most 16 contexts.
MCP tools `run_python` and `run_javascript` use these same owners and return image MIME output
as MCP image content, alongside text events.

Connection files are mode 0600 in a private temporary directory outside persistent data. Jupyter
channels use a random HMAC key on sandbox-local loopback. The agent launches kernel executables
directly, never a `jupyter` command. Native container isolation keeps these sockets off the host;
the process and wasm tiers explicitly refuse interpreter contexts until they can provide the
same private native transport. Images run as the sandbox's user and existing resource limits.
Snapshots preserve data, not live Python variables; waking a stopped container starts a fresh
kernel. Live state exists only while that kernel survives.

## Versions and licences

| Template/tool | Pinned version | Licence |
| --- | --- | --- |
| Python image | CPython 3.12.14, Debian bookworm | PSF-2.0 and historical grants; Debian package notices retained. |
| ipykernel | 7.4.0 | BSD-3-Clause |
| NumPy | 2.5.4 | BSD-3-Clause AND 0BSD AND MIT AND Zlib AND CC0-1.0; wheel notices include numerical runtimes. |
| pandas | 3.1.1 | BSD-3-Clause |
| matplotlib | 3.11.2 | Matplotlib permissive agreement and PSF-2.0; this is not Mozilla MPL. |
| SciPy | 1.18.1 | BSD-3-Clause and bundled numerical-library notices. |
| scikit-learn | 1.9.1 | BSD-3-Clause |
| requests | 2.34.2 | Apache-2.0 |
| Pillow | 13.0.0 | MIT-CMU |
| Node image | Node 24.22.1, Debian bookworm | MIT and bundled notices; Debian package notices retained. |
| IJavascript / jp-kernel / NEL | 5.2.1 / 2.0.0 / 1.3.0 | BSD-3-Clause |
| Chart.js / napi-rs canvas | 4.6.1 / 1.1.0 | MIT; bundled Skia/Chromium BSD-3-Clause notices retained. |
| ZeroMQ.js | 6.8.0 | MIT AND MPL-2.0, official v5 compatibility API for jmp. |
| Rust jupyter-zmq-client / jupyter-protocol | 3.0.1 / 2.0.3 | BSD-3-Clause |
| Rust zeromq | 0.7.0 | MIT |

The [Python](../templates/python-interpreter/README.md) and
[Node](../templates/node-interpreter/README.md) image descriptions list layer notices and source
availability. The resolved [Python](checks/measurements/k17-python-licences.json),
[Node](checks/measurements/k17-node-licences.json), [Rust](checks/measurements/k17-rust-licences.json)
and [base-package](checks/measurements/k17-base-licences.json) inventories record every new
version/licence. SHA-256 wheel hashes, npm integrity hashes and base manifest digests are pinned.
The Python lock targets Linux x86_64 / CPython 3.12, matching release builds.

Jupyter messaging 5.5 was read at jupyter_client 8.10.0 commit
`978361b3785dcd9cba6c733f4555e833e88fc0df`; the adapter uses the upstream Rust client.
IJavascript 5.2.1 is pinned to published commit `7011c92ebcb858d1aca4edca3adb4e0ea64db174`.
Neither evaluated JavaScript kernel has a recent core release. TSLab 1.0.22 was rejected after
real messages showed it emits values/errors as console text. IJavascript provides Jupyter rich
results and errors; its legacy ZeroMQ dependency is routed to the current official compatibility
API without copying upstream code or implementing a wire server.

Python preloads NumPy, pandas and pyplot, renders inline PNG/SVG with Agg, and registers JSON
representations for scalar values and DataFrame rows. Other installed scientific packages load
on demand. Measurements and their limits are in [speed.md](speed.md). The
[acceptance suite](../control/e2e/interpreter.e2e.test.ts) covers both templates, both SDK modes,
state, named contexts, PNG, traceback, interrupt, restart, dead processes, MCP and the CLI.
