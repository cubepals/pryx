# Python SDK

Install `cubepals-pryx` (Python 3.12+). Its generated models and HTTP methods come from the
committed public OpenAPI with openapi-python-client 0.26.0; the small wrapper supplies NDJSON
exec/log handling, error envelopes and the lifecycle conveniences. It performs no automatic
mutation retries. Generated client reprs suppress credentials.

```python
import os, uuid
from cubepals_pryx import Pryx
with Pryx(os.environ["PRYX_API_URL"], os.environ["PRYX_API_KEY"]) as pryx:
    sandbox = pryx.make({"image": "alpine:3.22", "entrypoint": ["sh", "-c", "while :; do sleep 1; done"]}, str(uuid.uuid4()))
    try:
        for event in pryx.exec(sandbox.id, ["sh", "-c", "echo hello > /data/message; cat /data/message"], key=str(uuid.uuid4())):
            print(event)
        snapshot = pryx.snapshot(sandbox.id)
        fork = pryx.fork(snapshot.id)
        try:
            print(pryx.get(fork.id).state)
        finally:
            pryx.delete(fork.id)
    finally:
        pryx.delete(sandbox.id)
```

Keep the same key for retries of one create or exec. A keyed exec whose result is uncertain
answers operation_unknown, including after a control restart; inspect it before using a new key.
Exec events arrive after the daemon's bounded command completes. Logs stream as they arrive.
Client timeout defaults to none; callers can provide httpx.Timeout. File transport has the same
48 KiB write bound and image-tool requirements as the TypeScript SDK.

Regenerate or check deterministically:

```sh
python3 -m venv .venv-python
.venv-python/bin/pip install -r sdk/python/generator-requirements.lock
.venv-python/bin/pip install -e sdk/python
PATH="$PWD/.venv-python/bin:$PATH" python3 sdk/python/generate.py generate
PATH="$PWD/.venv-python/bin:$PATH" python3 sdk/python/generate.py check
.venv-python/bin/python -m unittest discover -s sdk/python/test -v
```

The generator does not understand application/x-ndjson and reports that omission for exec/logs;
the wrapper implements those responses without changing the contract. The generate script
applies a deterministic repr=False credential transformation. The e2e test skips visibly without
PRYX_API_URL/PRYX_API_KEY and runs against the actual one-box daemon in CI.
