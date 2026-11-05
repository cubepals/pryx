# Python SDK

```python
import os
from cubepals_pryx import Pryx
with Pryx(url=os.environ["PRYX_API_URL"], key=os.environ["PRYX_API_KEY"]) as pryx:
    box = pryx.sandbox(image="node:22", idle="10m", ttl="2h")
    try:
        source = 'console.log("hello")\n'
        box.write("src/app.js", source)
        box.write("package.json", '{"scripts":{"test":"node src/app.js"}}')
        r = box.run("npm test --prefix /data", env={"npm_config_cache": "/tmp/npm"})
        print(r.stdout, r.stderr, r.code, r.timed_out)
        assert r.code == 0
        box.run(["node", "/data/src/app.js"], timeout="30s", env={"CI":"1"}).wait()
        for line in box.run("echo first; sleep 1; echo last").lines(): print(line)
        for chunk in box.run("echo chunk").stream(): print(chunk["stream"], chunk["data"])
        assert box.read("src/app.js") == source
        assert box.read_bytes("src/app.js") == source.encode()
        snap = box.snapshot()
        twin = snap.fork(idle="5m")
        try: print(twin.id, twin.state)
        finally: twin.delete()
        box.sleep(); box.wake(); box.stop(); box.start(); box.rest(); box.wake()
        print(box.refresh().state, box.endpoints(), len(pryx.sandboxes()))
        for entry in box.logs(follow=False): print(entry)
        print(pryx.sandbox(box.id).id)
    finally: box.delete()
    print(pryx.run("node:22", 'node -e "console.log(1)"').stdout)
```

Install `cubepals-pryx` on Python 3.12+. Commands are lazy, execute once, and return a run value:
reading its result fields or calling `wait()` collects it. `lines()` and `stream()` consume live
output once, then `wait()` returns the collected result. A string runs through the sandbox's
shell; argv does not. Commands use the image's working directory. File paths start at persistent
storage (`/data` by default), with bounded exec/tar chunks and no total 48 KiB SDK limit.
The image needs sh, realpath, mkdir, mktemp, dd, tar and base64. Multi-chunk writes are not atomic.

The async client has the same operations; awaiting a run collects its value:

```python
import asyncio, os
from cubepals_pryx import AsyncPryx
async def main():
    async with AsyncPryx(os.environ["PRYX_API_URL"], os.environ["PRYX_API_KEY"]) as pryx:
        box = await pryx.sandbox(image="node:22", idle="5m")
        try:
            r = await box.run('node -e "console.log(1)"')
            assert r.code == 0
            async for line in box.run("echo live; sleep 1; echo done").lines(): print(line)
            await box.write("message", b"hello")
            assert await box.read("message") == "hello"
        finally: await box.delete()
asyncio.run(main())
```

Durations accept positive seconds or `s`, `m`, `h`, `d` strings. Make/run generate idempotency
keys unless `key=` is passed. No operation is retried automatically. `PryxError` preserves status,
code and details. An incomplete stream has an unknown outcome; inspect it before starting another
command. Disconnect does not imply termination. Output is live on nodes with `exec-stream` and
through the agent on current nodes, and buffered on older nodes. `pryx.api` and every existing
sync raw method/constructor still work.

Models and defaults are generated from the committed OpenAPI with openapi-python-client 0.26.0.
Install `sdk/python/generator-requirements.lock`, then run `python sdk/python/generate.py generate`
or `check`. NDJSON is implemented by the wrapper because the generator does not support it.
`python -m unittest discover -s sdk/python/test -v` runs units; the real one-box tests require
PRYX_API_URL/PRYX_API_KEY and run in CI. Client reprs suppress credentials; configure httpx.Timeout
if a caller needs a deadline independent of the node's command timeout.

Both handles support `start(command, env=..., cwd=..., stdin=True)` returning a process with
`id`, `pid`, `stream`, `lines`, `stdin.write`, `stdin.close`, `signal`, `kill` and `wait`.
Omitting the command retains lifecycle start. `terminal(cols=120, rows=40)` returns a websocket
PTY with `stdin.write`, `resize`, `stream` and `close`; async clients await its I/O methods.
`expose(port)` returns a public URL, `unexpose` frees a spare, and `network('none')` replaces
compute while retaining data. Mutations are sent once.

`write` accepts bytes, file objects and iterables (async iterables on the async handle).
`read_stream` yields bytes without collecting the file. Files provide `list`, `stat`, `mkdir`,
`move`, `delete`, `watch`, `upload`, `download` and short-lived `url(path, method='GET')`.
Watches first yield `ready`. Data access selects the node automatically while compute is stopped.
`pryx.agent(id)` exposes the direct transport. Tokens are cached and never send the project key
to an agent. See [agent.md](../../docs/agent.md) for security and stream limits.
