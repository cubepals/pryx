# TypeScript SDK

```ts
import { Pryx } from '@cubepals/pryx'
const { PRYX_API_URL: url, PRYX_API_KEY: key } = process.env
if (!url || !key) throw new Error('Set PRYX_API_URL and PRYX_API_KEY')
const pryx = new Pryx({ url, key })
const box = await pryx.sandbox({ image: 'node:22', idle: '10m' })
try {
  await box.write('hello.js', 'console.log("hello")')
  const result = await box.run('node /data/hello.js')
  console.log(result.stdout); if (result.code !== 0) throw new Error('Command failed')
} finally { await box.delete() }
```

Install `@cubepals/pryx`.

Durations are positive seconds or `s`, `m`, `h`, `d` strings. String commands use `sh -c`;
argv bypasses the shell. Commands default to persistent storage; `cwd` overrides it.
Paths are relative to persistent storage (`/data`). Binary data streams through the agent while
running and through the node while stopped. No shell tools are required for file operations.
Commands receive one generated idempotency key per call; no mutation retries automatically.
Output capture and observation queues are bounded. Overflow is explicit; the result remains
available. A missing exit is an unknown outcome. Disconnect does not terminate the process.
`PryxError` preserves status, code and details. Credentials never enter serialized views.

| Method | Behavior |
| --- | --- |
| `pryx.sandbox(options \| id)` | Create or look up a sandbox; create keys are generated unless supplied. |
| `pryx.sandboxes()` | List project sandbox handles. |
| `pryx.project()` | Project usage and limits. |
| `pryx.run(image, command, options)` | Create, run and delete in a finally block. |
| `box.run(command, options)` | Start immediately; return a native promise of the bounded result. |
| `box.stream(command, options)` | Start immediately; iterate live {stream, data} chunks once. |
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
| `box.readBytes(path)` | Read persistent bytes. |
| `box.readStream(path)` | Read a byte stream without collecting the file. |
| `box.write(path, data)` | Write strings, bytes, streams or async iterables. |
| `box.snapshot()` | Create a snapshot handle. |
| `box.snapshots()` | List snapshot handles. |
| `snapshot.fork(options)` | Create a sandbox from the snapshot. |
| `snapshot.delete()` | Delete the snapshot. |
| `box.endpoints()` | List assigned application endpoints. |
| `box.events(since)` | Read lifecycle event records. |
| `box.logs(options, signal)` | Read or follow agent log history; abort with signal. |
| `box.terminal(options)` | Open a PTY with optional rows and cols. |
| `box.mcpUrl()` | Scoped sandbox-local MCP URL. |
| `box.expose(port)` | Publish a spare port and return its URL. |
| `box.unexpose(port)` | Release an exposed spare. |
| `box.network(mode)` | Replace compute with internet or none policy. |
| `box.toJSON()` | Serialize the public sandbox view. |
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
| `files.readBytes(path)` | Read bytes. |
| `files.readStream(path)` | Stream bytes. |
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

`Sandbox` exposes `id`, `state`, `isolation`, `units` and `files`. Result and option types are
exported with `Pryx`, `Sandbox`, `Snapshot`, `Process`, `Terminal`, `Files` and `PryxError`.
See [agent security](../../docs/agent.md) and [tiers](../../docs/tiers.md).
