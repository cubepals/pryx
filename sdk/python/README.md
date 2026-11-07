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
| `pryx.sandbox(options \| id)` | Create with env, labels, name, workdir, user, autoDelete, egress, metadata or repo; name-only looks up; lookup wakes unless wake:false. |
| `pryx.sandboxes(filters)` | Filter project handles by name, labels and state. |
| `pryx.project()` | Project usage and limits. |
| `pryx.run(image, command, options)` | Create, run and delete in a finally block. |
| `box.run(command, options)` | Start now; return the result (a native asyncio Task on the async handle). |
| `box.stream(command, options)` | Start now; iterate live chunks once, synchronously or asynchronously. |
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
| `box.read_bytes(path)` | Read persistent bytes. |
| `box.read_stream(path)` | Read a byte stream without collecting the file. |
| `box.write(path, data)` | Write strings, bytes, streams or async iterables. |
| `box.snapshot()` | Create a snapshot handle. |
| `box.snapshots()` | List snapshot handles. |
| `snapshot.fork(options)` | Create a sandbox from the snapshot. |
| `snapshot.delete()` | Delete the snapshot. |
| `box.endpoints()` | List assigned application endpoints. |
| `box.events(options)` | Iterate lifecycle records with since and follow. |
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
| `files.upload(local, remote, options)` | Stream caller disk to the sandbox with onProgress. |
| `files.download(remote, local, options)` | Stream to caller disk with onProgress. |
| `box.processes()` | List retained process handles. |
| `box.kill_all()` | Signal all running processes once. |
| `process.wait_for_port(port, options)` | Await owned TCP or HTTP readiness; timeout and early exit differ. |
| `process.wait_for_log(pattern, timeout)` | Await a bounded regular expression match in either stream. |
| `process.wait_for_exit(timeout)` | Await exit with a deadline. |
| `process.logs()` | Read retained process stdout/stderr chunks. |
| `box.shell(options)` | Open a persistent shell with cwd and env. |
| `shell.run(command, options)` | Run serially with exact separate streams and exit code. |
| `shell.close()` | Close and terminate that session. |
| `files.exists(path)` | Test existence without following links. |
| `files.find(glob)` | Find relative paths within the agent search budget. |
| `files.grep(pattern, options)` | Return path, line and matching text; paths scope the search. |
| `files.replace(pattern, text, options)` | Replace literal text at regex matches; preserve modes. |
| `files.chmod(path, mode)` | Set ordinary permission bits. |
| `files.upload_dir(local, remote, options)` | Stream a bounded tar; preserve files and empty directories. |
| `files.download_dir(remote, local, options)` | Extract a bounded tar; refuse links and traversal. |
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
| `box.keep_alive(duration)` | Extend access and touch activity within project ceilings. |
| `box.set_ttl(duration)` | Set TTL from now within the project ceiling. |
| `box.set_idle(duration)` | Set the idle timeout within the project ceiling. |
| `box.metrics(options)` | Latest node sample, or retained minute series with since. |
| `box.wait_until(state, timeout)` | Poll the public state until reached or timed out. |
| `pryx.template(options)` | Build copy/run/ready on a node with from_, publish a snapshot version. |
| `template.rebuild()` | Build a new immutable version from the same local context. |
| `pryx.templates()` | List project template versions. |
| `pryx.webhook(options)` | Register lifecycle delivery and show its secret once. |
| `pryx.webhooks()` | List registrations without secrets. |
| `webhook.delete()` | Delete this registration. |
| `webhook.deadletters()` | Inspect exhausted deliveries. |

Handles expose `id`, `state`, `isolation`, `units` and `files`. Option and result models are
exported alongside the seven handle/error classes. Models come from the committed OpenAPI;
`python sdk/python/generate.py check` checks generation with the pinned generator lock.
See [agent security](../../docs/agent.md) and [tiers](../../docs/tiers.md).

Git defaults to `repo`; clone changes that handle’s path. Tokens use a one-call credential helper, never a credential file. Public clone requires permitted internet and Git in the image/userland (the wasm Git package is staged on demand).
Shells use serialized pipes with independent stdout/stderr markers; [measured latency](../../docs/checks/k16.md) includes fresh processes and persistent sessions. Search/tar budgets and link refusals are in [agent security](../../docs/agent.md).
Container hostname egress requires the advertised host firewall feature; process and wasm refuse it with the tier named. Webhook signatures and retry limits are in [the API reference](../../docs/api.md).
Git clone/push/pull default to five minutes; other Git operations use the normal command budget. Every method accepts a timeout override. Wasm clones keep metadata in persistent guest storage and checkout files in data; both survive stop and snapshots. Automatic background maintenance and all Git trace destinations are disabled during helper calls.
