# SPDX-License-Identifier: FSL-1.1-ALv2
"""Asynchronous direct agent requests use the existing verified HTTP pool with a sandbox
capability overriding the project authorization header. No mutation is retried.
"""
import asyncio
import base64
import codecs
import json
import time
from uuid import uuid4
from urllib.parse import quote
from websockets.asyncio.client import connect
from .agent import start_body
from ._transport import _checked
from .generated.defaults import SDK_DEFAULTS

class AsyncAgent:
    def __init__(self, api, sandbox):
        self._api, self.id, self._credential = api, sandbox, None
        self._lock, self._generation = asyncio.Lock(), 0
    @property
    def http(self): return self._api.http
    def invalidate(self): self._generation += 1; self._credential = None
    async def token(self):
        async with self._lock:
            if self._credential and self._credential['expiresAt'] > time.time() + SDK_DEFAULTS['tokenRenewBeforeSeconds']: return self._credential
            generation = self._generation
            credential = await self._api.request(f'/v1/sandboxes/{quote(self.id, safe="")}/token', 'POST')
            if generation == self._generation: self._credential = credential
            return credential
    async def target(self, path):
        token = await self.token()
        return token['url'].rstrip('/') + path, {'authorization': f'Bearer {token["token"]}'}
    async def raw(self, path, method='GET', *, content=None, json_body=None, key=None):
        url, headers = await self.target(path)
        if key: headers['Idempotency-Key'] = key
        return _checked(await self.http.request(method, url, headers=headers, content=content, json=json_body))
    async def request(self, path, method='GET', body=None, key=None):
        response = await self.raw(path, method, json_body=body, key=key)
        return response.json() if response.content else None
    async def start(self, command, *, key=None, **options):
        return AsyncProcess(self, await self.request('/_pryx/processes', 'POST', start_body(command, options), key if key is not None else str(uuid4())))
    async def process(self, process): return AsyncProcess(self, await self.request(f'/_pryx/processes/{quote(process, safe="")}'))
    async def processes(self): return [AsyncProcess(self, view) for view in (await self.request('/_pryx/processes'))['processes']]
    async def expose(self, port): return (await self.request('/_pryx/ports', 'POST', {'port': port}))['url']
    async def unexpose(self, port): return await self.request(f'/_pryx/ports/{port}', 'DELETE')
    async def ports(self): return (await self.request('/_pryx/ports'))['ports']
    async def logs(self, **options):
        from .logs import Decoder
        from urllib.parse import urlencode
        decoder=Decoder(**options)
        url,headers=await self.target("/_pryx/logs?"+urlencode({"follow":str(options.get("follow",False)).lower()}))
        async with self.http.stream("GET",url,headers=headers) as response:
            if not response.is_success: await response.aread(); _checked(response)
            async for line in response.aiter_lines():
                if line:
                    for record in decoder.frame(json.loads(line)): yield record
            for record in decoder.finish(): yield record
    async def socket(self, path):
        token = await self.token()
        url = token['url'].rstrip('/').replace('https:', 'wss:', 1).replace('http:', 'ws:', 1) + path
        return await connect(url, subprotocols=['pryx', f'pryx-token.{token["token"]}'], max_queue=16, max_size=SDK_DEFAULTS['captureBytes'], proxy=None)
    async def terminal(self, *, cols=80, rows=24):
        terminal = AsyncTerminal(await self.socket('/_pryx/terminal'))
        await terminal.resize(cols, rows)
        return terminal

class AsyncStdin:
    def __init__(self, process): self._process = process
    async def write(self, data): return await self._process._agent.raw(self._process.path('/stdin'), 'POST', content=data.encode() if isinstance(data, str) else data)
    async def close(self): return await self._process._agent.request(self._process.path('/stdin'), 'DELETE')

class AsyncProcess:
    def __init__(self, agent, view): self._agent, self.view, self.stdin = agent, view, AsyncStdin(self)
    @property
    def id(self): return self.view['id']
    @property
    def pid(self): return self.view['pid']
    def path(self, suffix=''): return f'/_pryx/processes/{quote(self.id, safe="")}{suffix}'
    async def wait(self):
        from .generated.models import AgentResult
        return AgentResult.from_dict(await self._agent.request(self.path('/wait')))
    async def signal(self, signal): return await self._agent.request(self.path('/signal'), 'POST', {'signal': signal})
    async def kill(self): return await self.signal('KILL')
    async def resize(self, cols, rows): return await self._agent.request(self.path('/resize'), 'POST', {'cols': cols, 'rows': rows})
    async def events(self):
        url, headers = await self._agent.target(self.path('/output'))
        async with self._agent.http.stream('GET', url, headers=headers) as response:
            if not response.is_success: await response.aread(); _checked(response)
            async for line in response.aiter_lines():
                if line: yield json.loads(line)
    async def run_events(self):
        decoders = {name: codecs.getincrementaldecoder('utf-8')(errors='replace') for name in ('stdout', 'stderr')}
        async for event in self.events():
            if event['type'] == 'output':
                stream = event['stream']
                data = decoders[stream].decode(base64.b64decode(event['bytes'], validate=True))
                if data: yield {'type': stream, 'data': data, 'truncated': False}
            elif event['type'] == 'exit':
                for stream, decoder in decoders.items():
                    data = decoder.decode(b'', final=True)
                    if data: yield {'type': stream, 'data': data, 'truncated': False}
                yield {'type': 'exit', **event['result']}
            else: yield {'type': 'stdout', 'data': '', 'truncated': True}
    async def stream(self):
        async for event in self.run_events():
            if event['type'] in ('stdout', 'stderr'): yield {'stream': event['type'], 'data': event['data']}
    async def lines(self):
        from .run import async_lines
        async for line in async_lines(self.stream()): yield line

class AsyncTerminal:
    def __init__(self, socket): self.socket, self.stdin = socket, self
    async def write(self, data): await self.socket.send(json.dumps({'type': 'stdin', 'bytes': base64.b64encode(data.encode() if isinstance(data, str) else data).decode('ascii')}))
    async def resize(self, cols, rows): await self.socket.send(json.dumps({'type': 'resize', 'cols': cols, 'rows': rows}))
    async def signal(self, signal): await self.socket.send(json.dumps({'type': 'signal', 'signal': signal}))
    async def close(self): await self.socket.close()
    async def events(self):
        async for frame in self.socket: yield json.loads(frame)
    async def stream(self):
        # Share the incremental process decoder, with this terminal as its event source.
        async for event in AsyncProcess.run_events(self):
            if event['type'] in ('stdout', 'stderr'): yield {'stream': event['type'], 'data': event['data']}
