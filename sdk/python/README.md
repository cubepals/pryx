# Python SDK

```python
import os
from cubepals_pryx import Pryx
with Pryx(url=os.environ['PRYX_API_URL'], key=os.environ['PRYX_API_KEY']) as pryx:
    box = pryx.sandbox(image='node:22', idle='10m')
    try:
        box.write('hello.js', 'console.log("hello")')
        result = box.run('node /data/hello.js')
        print(result.stdout)
        assert result.code == 0
    finally: box.delete()
```

Install `cubepals-pryx` on Python 3.12+.

Durations are positive seconds or `s`, `m`, `h`, `d` strings. String commands use `sh -c`;
argv bypasses the shell. Commands default to persistent storage; `cwd` overrides it.
Paths are relative to persistent storage (`/data`). Binary data streams through the agent while
running and through the node while stopped. No shell tools are required for file operations.
Commands receive one generated idempotency key per call; no mutation retries automatically.
Output capture and observation queues are bounded. Overflow is explicit; the result remains
available. A missing exit is an unknown outcome. Disconnect does not terminate the process.
`PryxError` preserves status, code and details. Credentials never enter serialized views.

Use `async with Pryx(url=..., key=...) as pryx` for asynchronous handles with the same names.
Await I/O methods and `run`; use `async for` on streams. `stream.result()` is awaited.
Sync `run` returns `RunResult` immediately after completion; it needs no `wait()` call.
Python result fields use snake_case (`timed_out`, `stdout_truncated`, `duration_ms`).
Create/process identity uses `key=`, options use keyword arguments, and `timeout=` accepts durations.

| Method | Behavior |
| --- | --- |
| `pryx.sandbox(options \| id)` | Create or look up a sandbox; create keys are generated unless supplied. |
| `pryx.sandboxes()` | List project sandbox handles. |
| `pryx.project()` | Project usage and limits. |
| `pryx.run(image, command, options)` | Create, run and delete in a finally block. |
| `box.run(command, options)` | Start now; return the result (a native asyncio Task on the async handle). |
| `box.stream(command, options)` | Start now; iterate live chunks once, synchronously or asynchronously. |
| `stream.lines()` | Iterate complete stdout/stderr lines. |
| `stream.result()` | Await the same command result without starting it again. |
| `box.spawn(command, options)` | Start a process and return its handle. |
| `box.start()` | Start sandbox compute. |
| `box.stop()` | Stop compute; retain data. |
| `box.sleep()` | Suspend or stop while retaining data. |
| `box.wake()` | Resume or recreate compute. |
| `box.rest()` | Archive data and release compute. |
| `box.refresh()` | Refresh the view and invalidate its cached capability. |
| `box.delete()` | Delete sandbox and owned data. |
| `box.read(path)` | Read UTF-8 persistent data. |
| `box.read_bytes(path)` | Read persistent bytes. |
| `box.read_stream(path)` | Read a byte stream without collecting the file. |
| `box.write(path, data)` | Write strings, bytes, streams or async iterables. |
| `box.snapshot()` | Create a snapshot handle. |
| `box.snapshots()` | List snapshot handles. |
| `snapshot.fork(options)` | Create a sandbox from the snapshot. |
| `snapshot.delete()` | Delete the snapshot. |
| `box.endpoints()` | List assigned application endpoints. |
| `box.events(since)` | Read lifecycle event records. |
| `box.logs(**options)` | Read or follow agent logs. |
| `box.terminal(options)` | Open a PTY with optional rows and cols. |
| `box.mcp_url()` | Scoped sandbox-local MCP URL. |
| `box.expose(port)` | Publish a spare port and return its URL. |
| `box.unexpose(port)` | Release an exposed spare. |
| `box.network(mode)` | Replace compute with internet or none policy. |
| `process.wait()` | Wait for its terminal status. |
| `process.events()` | Read agent output/exit frames. |
| `process.stream()` | Read decoded stdout/stderr chunks. |
| `process.lines()` | Read complete lines. |
| `process.stdin.write(data)` | Send string or bytes. |
| `process.stdin.close()` | Close input. |
| `process.signal(signal)` | Signal the process. |
| `process.kill()` | Kill the process. |
| `process.resize(cols, rows)` | Resize its PTY. |
| `terminal.stdin.write(data)` | Send terminal input. |
| `terminal.resize(cols, rows)` | Resize the terminal. |
| `terminal.signal(signal)` | Signal the terminal process. |
| `terminal.events()` | Read terminal frames. |
| `terminal.stream()` | Read terminal output. |
| `terminal.close()` | Close the terminal. |
| `files.read(path)` | Read UTF-8 text. |
| `files.read_bytes(path)` | Read bytes. |
| `files.read_stream(path)` | Stream bytes. |
| `files.write(path, data)` | Write persistent data. |
| `files.list(path)` | List directory entries. |
| `files.stat(path)` | Read file metadata. |
| `files.mkdir(path)` | Create directories. |
| `files.move(from, to)` | Move a persistent entry. |
| `files.delete(path)` | Delete a persistent entry. |
| `files.watch(path)` | Ready frame, then bounded changes. |
| `files.url(path, method)` | Grant one file operation for at most one minute. |
| `files.upload(path, url)` | Download an HTTP(S) URL into the sandbox. |
| `files.download(path, url)` | Upload sandbox content to an HTTP(S) URL. |

Handles expose `id`, `state`, `isolation`, `units` and `files`. Option and result models are
exported alongside the seven handle/error classes. Models come from the committed OpenAPI;
`python sdk/python/generate.py check` checks generation with the pinned generator lock.
See [agent security](../../docs/agent.md) and [tiers](../../docs/tiers.md).
