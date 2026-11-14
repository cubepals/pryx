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
| `pryx.sandbox(options \| id)` | Create with env, labels, name, workdir, user, autoDelete, egress, metadata or repo; name-only looks up; lookup wakes unless wake:false. |
| `pryx.sandboxes(filters)` | Filter project handles by name, labels and state. |
| `pryx.project()` | Project usage and limits. |
| `pryx.run(image, command, options)` | Create, run and delete in a finally block. |
| `box.run(command, options)` | Start immediately; return a native promise of the bounded result. |
| `box.stream(command, options)` | Start immediately; iterate live {stream, data} chunks once. |
| `stream.lines()` | Iterate complete stdout/stderr lines. |
| `stream.result()` | Await the same command result without starting it again. |
| `box.spawn(command, options)` | Start a process; stdin accepts one-shot string or bytes. |
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
| `box.events(options)` | Iterate lifecycle records with since and follow. |
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
| `files.upload(local, remote, options)` | Stream caller disk to the sandbox with onProgress. |
| `files.download(remote, local, options)` | Stream to caller disk with onProgress. |
| `box.processes()` | List retained process handles. |
| `box.killAll()` | Signal all running processes once. |
| `process.waitForPort(port, options)` | Await owned TCP or HTTP readiness; timeout and early exit differ. |
| `process.waitForLog(pattern, timeout)` | Await a bounded regular expression match in either stream. |
| `process.waitForExit(timeout)` | Await exit with a deadline. |
| `process.logs()` | Read retained process stdout/stderr chunks. |
| `box.shell(options)` | Open a persistent shell with cwd and env. |
| `shell.run(command, options)` | Run serially with exact separate streams and exit code. |
| `shell.close()` | Close and terminate that session. |
| `files.exists(path)` | Test existence without following links. |
| `files.find(glob)` | Find relative paths within the agent search budget. |
| `files.grep(pattern, options)` | Return path, line and matching text; paths scope the search. |
| `files.replace(pattern, text, options)` | Replace literal text at regex matches; preserve modes. |
| `files.chmod(path, mode)` | Set ordinary permission bits. |
| `files.uploadDir(local, remote, options)` | Stream a bounded tar; preserve files and empty directories. |
| `files.downloadDir(remote, local, options)` | Extract a bounded tar; refuse links and traversal. |
| `box.sync(local, remote)` | Upsert changed content by SHA-256; never delete. |
| `box.git.clone(url, options)` | Clone public/HTTPS credentials with path, branch and depth. |
| `box.git.status()` | Parsed clean flag and entries, plus raw result. |
| `box.git.diff(options)` | Parsed patch, optionally staged, plus raw result. |
| `box.git.branch(name?)` | List or create branches, plus raw result. |
| `box.git.checkout(ref, options)` | Checkout or create a branch, plus raw result. |
| `box.git.add(paths)` | Stage paths without shell interpolation. |
| `box.git.commit(message, options)` | Commit with optional author; return hash and raw result. |
| `box.git.push(options)` | Push with an optional one-call token helper. |
| `box.git.pull()` | Fast-forward pull, plus raw result. |
| `box.git.log(options)` | Parsed commit records with limit, plus raw result. |
| `box.keepAlive(duration)` | Extend access and touch activity within project ceilings. |
| `box.setTtl(duration)` | Set TTL from now within the project ceiling. |
| `box.setIdle(duration)` | Set the idle timeout within the project ceiling. |
| `box.metrics(options)` | Latest node sample, or retained minute series with since. |
| `box.waitUntil(state, timeout)` | Poll the public state until reached or timed out. |
| `pryx.template(options)` | Build copy/run/ready on a node and publish a snapshot version. |
| `template.rebuild()` | Build a new immutable version from the same local context. |
| `pryx.templates()` | List project template versions. |
| `pryx.webhook(options)` | Register lifecycle delivery and show its secret once. |
| `pryx.webhooks()` | List registrations without secrets. |
| `webhook.delete()` | Delete this registration. |
| `webhook.deadletters()` | Inspect exhausted deliveries. |

`Sandbox` exposes `id`, `state`, `isolation`, `units` and `files`. Result and option types are
exported with `Pryx`, `Sandbox`, `Snapshot`, `Process`, `Terminal`, `Files` and `PryxError`.
See [agent security](../../docs/agent.md) and [tiers](../../docs/tiers.md).

Git defaults to `repo`; clone changes that handle’s path. Tokens use a one-call credential helper, never a credential file. Public clone requires permitted internet and Git in the image/userland (the wasm Git package is staged on demand).
Shells use serialized pipes with independent stdout/stderr markers; [measured latency](../../docs/checks/k16.md) includes fresh processes and persistent sessions. Search/tar budgets and link refusals are in [agent security](../../docs/agent.md).
Container hostname egress requires the advertised host firewall feature; process and wasm refuse it with the tier named. Webhook signatures and retry limits are in [the API reference](../../docs/api.md).
Git clone/push/pull default to five minutes; other Git operations use the normal command budget. Every method accepts a timeout override. Wasm clones keep metadata in persistent guest storage and checkout files in data; both survive stop and snapshots. Automatic background maintenance and all Git trace destinations are disabled during helper calls.

Rivet's sandbox provider is a separate entry point, `@cubepals/pryx/provider`:
`pryxProvider()` reads `PRYX_API_URL` and `PRYX_API_KEY`; `pryxPortAddress(id, port)` returns
an edge preview. See [the integration guide](../../docs/integrations/rivet.md) and
[the runnable example](../../examples/rivet-provider.ts), including the current OpenOrange
registration and required-guarantee boundaries.
