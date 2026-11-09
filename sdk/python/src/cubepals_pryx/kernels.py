# SPDX-License-Identifier: FSL-1.1-ALv2
"""Persistent kernels retain every MIME representation and stream through the scoped agent.
Language errors remain result data; transport interruption is never treated as successful code.
"""
import asyncio
import base64
import json
from dataclasses import dataclass, field
from urllib.parse import quote
from ._transport import PryxError, _checked
from .duration import seconds
from .generated.defaults import SDK_DEFAULTS
from .generated.kernels import KERNEL_DEFAULTS

def invalid(message): return PryxError(0, {'error': {'code': 'agent_protocol', 'message': message}})

def execution(code, timeout):
    """An omitted timeout is the agent's to choose: its default, bounded by the token's ceiling."""
    return {'code': code} if timeout is None else {'code': code, 'timeout': seconds(timeout)}

def checked(event):
    if not isinstance(event, dict): raise invalid('Invalid kernel event')
    kind = event.get('type')
    def count(value): return value is None or (isinstance(value, int) and not isinstance(value, bool) and value >= 0)
    if kind == 'stream': valid = event.get('stream') in ('stdout', 'stderr') and isinstance(event.get('text'), str)
    elif kind in ('result', 'display'):
        valid = isinstance(event.get('data'), dict) and isinstance(event.get('metadata'), dict) and (kind == 'display' or count(event.get('executionCount')))
    elif kind == 'error': valid = isinstance(event.get('name'), str) and isinstance(event.get('value'), str) and isinstance(event.get('traceback'), list) and all(isinstance(line, str) for line in event['traceback'])
    elif kind == 'done': valid = isinstance(event.get('status'), str) and 'executionCount' in event and count(event['executionCount'])
    else: valid = False
    if not valid: raise invalid('Invalid kernel event')
    return event

class Lines:
    def __init__(self): self.pending = b''
    def feed(self, chunk, final=False):
        self.pending += chunk
        out = []
        while b'\n' in self.pending:
            line, self.pending = self.pending.split(b'\n', 1)
            if len(line) > SDK_DEFAULTS['captureBytes'] * 2: raise invalid('Kernel event exceeded its line budget')
            if line.strip(): out.append(self.parse(line))
        if len(self.pending) > SDK_DEFAULTS['captureBytes'] * 2: raise invalid('Kernel event exceeded its line budget')
        if final and self.pending.strip(): out.append(self.parse(self.pending)); self.pending = b''
        return out
    @staticmethod
    def parse(line):
        try: return checked(json.loads(line))
        except (ValueError, UnicodeError) as error: raise invalid('Invalid kernel JSON event') from error

@dataclass
class KernelResult:
    stdout: str = ''
    stderr: str = ''
    error: dict | None = None
    execution_count: int | None = None
    status: str = ''
    results: list = field(default_factory=list)
    events: list = field(default_factory=list)
    _bytes: int = field(default=0, repr=False)
    def append(self, event):
        self._bytes += len(json.dumps(event, ensure_ascii=False).encode())
        if self._bytes > SDK_DEFAULTS['captureBytes'] * 2: raise invalid('Kernel result exceeded capture budget')
        self.events.append(event)
        kind = event['type']
        if kind == 'stream':
            if event['stream'] == 'stdout': self.stdout += event['text']
            else: self.stderr += event['text']
        elif kind in ('result', 'display'): self.results.append(event)
        elif kind == 'error': self.error = {key: event[key] for key in ('name', 'value', 'traceback')}
        elif kind == 'done': self.execution_count, self.status = event['executionCount'], event['status']
    def mime(self, name):
        for item in reversed(self.results):
            if name in item['data']: return item['data'][name]
        return None
    @property
    def text(self): return self.mime('text/plain')
    @property
    def html(self): return self.mime('text/html')
    @property
    def json(self): return self.mime('application/json')
    @property
    def svg(self): return self.mime('image/svg+xml')
    @property
    def png(self):
        value = self.mime('image/png')
        if value is None: return None
        try: return base64.b64decode(value, validate=True)
        except (ValueError, TypeError) as error: raise invalid('Invalid kernel PNG') from error
    def finish(self):
        if not self.status: raise PryxError(0, {'error': {'code': 'kernel_interrupted', 'message': 'Kernel stream ended without completion; inspect it before retrying'}})
        return self

class Kernel:
    def __init__(self, agent, view): self._agent, self._view = agent, view
    @classmethod
    def open(cls, agent, language, context=''):
        agent.require_kernels(KERNEL_DEFAULTS['version'])
        return cls(agent, agent.request('/_pryx/kernels', 'POST', {'language': language, 'context': context}))
    @property
    def id(self): return self._view['id']
    @property
    def pid(self): return self._view['pid']
    @property
    def process_id(self): return self._view['processId']
    @property
    def context(self): return self._view['context']
    def _path(self, suffix=''): return '/_pryx/kernels/' + quote(self.id, safe='') + suffix
    def stream(self, code, *, timeout=None):
        url, headers = self._agent.target(self._path('/execute'))
        parser, done = Lines(), False
        with self._agent.http.stream('POST', url, headers=headers, json=execution(code, timeout)) as response:
            if not response.is_success: response.read(); _checked(response)
            for chunk in response.iter_bytes():
                for event in parser.feed(chunk):
                    if done: raise invalid('Kernel sent data after done')
                    done = event['type'] == 'done'
                    yield event
            for event in parser.feed(b'', final=True):
                if done: raise invalid('Kernel sent data after done')
                done = event['type'] == 'done'
                yield event
        if not done: raise PryxError(0, {'error': {'code': 'kernel_interrupted', 'message': 'Kernel stream ended without completion; inspect it before retrying'}})
    def run(self, code, **options):
        result = KernelResult()
        for event in self.stream(code, **options): result.append(event)
        return result.finish()
    def complete(self, code): return self._agent.request(self._path('/complete'), 'POST', {'code': code})
    def interrupt(self): return self._agent.request(self._path('/interrupt'), 'POST')
    def restart(self): self._view = self._agent.request(self._path('/restart'), 'POST'); return self
    def close(self): return self._agent.request(self._path(), 'DELETE')

class AsyncKernel(Kernel):
    @classmethod
    async def open(cls, agent, language, context=''):
        await agent.require_kernels(KERNEL_DEFAULTS['version'])
        return cls(agent, await agent.request('/_pryx/kernels', 'POST', {'language': language, 'context': context}))
    async def stream(self, code, *, timeout=None):
        url, headers = await self._agent.target(self._path('/execute'))
        parser, done = Lines(), False
        async with self._agent.http.stream('POST', url, headers=headers, json=execution(code, timeout)) as response:
            if not response.is_success: await response.aread(); _checked(response)
            async for chunk in response.aiter_bytes():
                for event in parser.feed(chunk):
                    if done: raise invalid('Kernel sent data after done')
                    done = event['type'] == 'done'
                    yield event
            for event in parser.feed(b'', final=True):
                if done: raise invalid('Kernel sent data after done')
                done = event['type'] == 'done'
                yield event
        if not done: raise PryxError(0, {'error': {'code': 'kernel_interrupted', 'message': 'Kernel stream ended without completion; inspect it before retrying'}})
    async def _collect(self, code, options):
        result = KernelResult()
        async for event in self.stream(code, **options): result.append(event)
        return result.finish()
    def run(self, code, **options): return asyncio.create_task(self._collect(code, options))
    async def complete(self, code): return await self._agent.request(self._path('/complete'), 'POST', {'code': code})
    async def interrupt(self): return await self._agent.request(self._path('/interrupt'), 'POST')
    async def restart(self): self._view = await self._agent.request(self._path('/restart'), 'POST'); return self
    async def close(self): return await self._agent.request(self._path(), 'DELETE')
