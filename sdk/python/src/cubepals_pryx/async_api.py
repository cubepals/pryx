# SPDX-License-Identifier: FSL-1.1-ALv2
"""Asynchronous thin API with one persistent HTTP client and no automatic mutation retries."""
import json
from uuid import uuid4
import httpx
from .api import _checked
from .generated.models import Sandbox, Snapshot
class AsyncApi:
    def __init__(self, url, key, *, transport=None, timeout=None):
        self._http = httpx.AsyncClient(base_url=url.rstrip('/'), headers={"authorization": f"Bearer {key}"}, transport=transport, timeout=timeout)
    async def close(self): await self._http.aclose()
    async def request(self, path, method="GET", body=None, key=None):
        return _checked(await self._http.request(method, path, json=body, headers={"Idempotency-Key": key} if key else {})).json()
    async def make(self, body, key=None): return Sandbox.from_dict(await self.request('/v1/sandboxes', 'POST', body, key if key is not None else str(uuid4())))
    async def get(self, sandbox): return Sandbox.from_dict(await self.request(f'/v1/sandboxes/{sandbox}'))
    async def delete(self, sandbox): return await self.request(f'/v1/sandboxes/{sandbox}', 'DELETE')
    async def list(self): return await self.request('/v1/sandboxes')
    async def action(self, sandbox, verb):
        if verb not in ('start','stop','sleep','wake','rest'): raise ValueError('Invalid sandbox action')
        return await self.request(f'/v1/sandboxes/{sandbox}/{verb}', 'POST')
    async def snapshot(self, sandbox): return Snapshot.from_dict(await self.request(f'/v1/sandboxes/{sandbox}/snapshots', 'POST'))
    async def fork(self, snapshot, body=None, key=None): return Sandbox.from_dict(await self.request(f'/v1/snapshots/{snapshot}/sandboxes', 'POST', body or {}, key if key is not None else str(uuid4())))
    async def exec(self, sandbox, command, *, timeout=None, env=None, key=None):
        key = key if key is not None else str(uuid4())
        body={"command": command, **({"timeout": timeout} if timeout is not None else {}), **({"env": env} if env is not None else {})}
        async for event in self._lines(f'/v1/sandboxes/{sandbox}/exec', 'POST', body, {"Idempotency-Key": key} if key else {}): yield event
    async def _lines(self, path, method="GET", body=None, headers=None):
        async with self._http.stream(method, path, json=body, headers=headers) as response:
            if response.is_error: await response.aread(); _checked(response)
            async for line in response.aiter_lines():
                if line.strip(): yield json.loads(line)
    def logs(self, sandbox, **query): return self._lines(f'/v1/sandboxes/{sandbox}/logs?{httpx.QueryParams({k: str(v).lower() if isinstance(v,bool) else str(v) for k,v in query.items() if v is not None})}')
    async def snapshots(self, sandbox): return await self.request(f'/v1/sandboxes/{sandbox}/snapshots')
    async def delete_snapshot(self, sandbox, snapshot): return await self.request(f'/v1/sandboxes/{sandbox}/snapshots/{snapshot}', 'DELETE')
    async def endpoints(self, sandbox): return await self.request(f'/v1/sandboxes/{sandbox}/endpoints')
    async def events(self, sandbox): return await self.request(f'/v1/sandboxes/{sandbox}/events')
    async def put_file(self, sandbox, path, data_base64): return await self.request(f'/v1/sandboxes/{sandbox}/files/{path}', 'PUT', {"dataBase64": data_base64})
    async def get_file(self, sandbox, path): return await self.request(f'/v1/sandboxes/{sandbox}/files/{path}')
    async def template(self, name, spec): return await self.request('/v1/templates', 'POST', {"name": name, "spec": spec})
    async def templates(self): return await self.request('/v1/templates')
