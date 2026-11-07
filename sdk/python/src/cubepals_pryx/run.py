# SPDX-License-Identifier: FSL-1.1-ALv2
"""Native results and eager bounded observations; consuming output never starts another command."""
import asyncio
import threading
from collections import deque
from concurrent.futures import Future
from dataclasses import dataclass
from ._transport import PryxError
from .generated.defaults import SDK_DEFAULTS

@dataclass
class RunResult:
    stdout: str = ''
    stderr: str = ''
    code: int | None = None
    timed_out: bool = False
    killed: bool = False
    duration_ms: int = 0
    stdout_truncated: bool = False
    stderr_truncated: bool = False

def _event(result, event):
    kind = event.get('type')
    if kind in ('stdout', 'stderr'):
        old = getattr(result, kind)
        room = max(0, SDK_DEFAULTS['captureBytes'] - len(old))
        setattr(result, kind, old + event['data'][:room])
        setattr(result, kind+'_truncated', getattr(result,kind+'_truncated') or event['truncated'] or len(event['data']) > room)
        return {'stream':kind, 'data':event['data']}
    if kind == 'exit':
        for target, source in [('code','code'),('timed_out','timedOut'),('killed','killed'),('duration_ms','durationMs')]: setattr(result,target,event[source])
        for target, source in [('stdout_truncated','stdoutTruncated'),('stderr_truncated','stderrTruncated')]: setattr(result,target,getattr(result,target) or event.get(source,False))
    elif kind == 'error': raise PryxError(0, {'error':event.get('error',{})})
    return None

def _interrupted(): return PryxError(0, {'error':{'code':'exec_interrupted','message':'Command ended without an exit result; its outcome is unknown'}})

def lines(chunks):
    rest = {'stdout':'','stderr':''}
    for chunk in chunks:
        stream = chunk['stream']
        if len(rest[stream])+len(chunk['data']) > SDK_DEFAULTS['captureBytes']: raise PryxError(0,{'error':{'code':'line_too_long','message':'Output line exceeds its bounded buffer'}})
        rest[stream] += chunk['data']
        while '\n' in rest[stream]:
            line,rest[stream] = rest[stream].split('\n',1)
            yield line.removesuffix('\r')
    yield from (line for line in rest.values() if line)

async def async_lines(chunks):
    rest = {'stdout':'','stderr':''}
    async for chunk in chunks:
        stream = chunk['stream']
        if len(rest[stream])+len(chunk['data']) > SDK_DEFAULTS['captureBytes']: raise PryxError(0,{'error':{'code':'line_too_long','message':'Output line exceeds its bounded buffer'}})
        rest[stream] += chunk['data']
        while '\n' in rest[stream]:
            line,rest[stream] = rest[stream].split('\n',1)
            yield line.removesuffix('\r')
    for line in rest.values():
        if line: yield line

class CommandStream:
    def __init__(self, process, observe=True):
        self._pending, self._held, self._done, self._consuming = deque(), 0, False, False
        self._condition, self._result, self._failure = threading.Condition(), Future(), None
        def consume():
            result, exited = RunResult(), False
            try:
                for event in process.run_events():
                    chunk = _event(result,event)
                    exited |= event.get('type')=='exit'
                    if chunk and observe:
                        with self._condition:
                            if self._failure is None:
                                if self._held+len(chunk['data']) > SDK_DEFAULTS['captureBytes'] or len(self._pending)>=1024:
                                    self._failure = PryxError(0,{'error':{'code':'stream_overflow','message':'Output consumer fell behind; result remains available'}})
                                    self._pending.clear();self._held=0
                                else: self._pending.append(chunk);self._held+=len(chunk['data'])
                            self._condition.notify_all()
                if not exited: raise _interrupted()
                self._result.set_result(result)
            except BaseException as error: self._result.set_exception(error)
            finally:
                with self._condition: self._done=True; self._condition.notify_all()
        threading.Thread(target=consume, daemon=True).start()
    def __iter__(self):
        if self._consuming: raise RuntimeError('A command stream has one consumer')
        self._consuming=True
        try:
            while True:
                with self._condition:
                    while not self._pending and not self._done and not self._failure: self._condition.wait()
                    if self._failure: raise self._failure
                    if not self._pending: break
                    chunk=self._pending.popleft();self._held-=len(chunk['data'])
                yield chunk
            self._result.result()
        finally:
            with self._condition:
                self._pending.clear();self._held=0
                self._failure = self._failure or RuntimeError('Command observation closed')
    def lines(self): return lines(self)
    def result(self): return self._result.result()

class AsyncCommandStream:
    def __init__(self, process, observe=True):
        self._pending,self._held,self._done,self._consuming = deque(),0,False,False
        self._changed,self._failure = asyncio.Event(),None
        async def consume():
            result,exited=RunResult(),False
            try:
                async for event in (await process).run_events():
                    chunk=_event(result,event);exited |= event.get('type')=='exit'
                    if chunk and observe and self._failure is None:
                        if self._held+len(chunk['data']) > SDK_DEFAULTS['captureBytes'] or len(self._pending)>=1024:
                            self._failure=PryxError(0,{'error':{'code':'stream_overflow','message':'Output consumer fell behind; result remains available'}})
                            self._pending.clear();self._held=0
                        else:self._pending.append(chunk);self._held+=len(chunk['data'])
                        self._changed.set()
                if not exited:raise _interrupted()
                return result
            finally:self._done=True;self._changed.set()
        self._task=asyncio.Task(consume(),loop=asyncio.get_running_loop(),eager_start=True)
        self._task.add_done_callback(lambda task: None if task.cancelled() else task.exception())
    async def __aiter__(self):
        if self._consuming:raise RuntimeError('A command stream has one consumer')
        self._consuming=True
        try:
            while not self._done or self._pending:
                if self._failure:raise self._failure
                if self._pending:
                    chunk=self._pending.popleft();self._held-=len(chunk['data']);yield chunk
                else:
                    self._changed.clear()
                    await self._changed.wait()
            if self._failure:raise self._failure
            await self._task
        finally:self._pending.clear();self._held=0;self._failure=self._failure or RuntimeError('Command observation closed')
    def lines(self):return async_lines(self)
    async def result(self):return await self._task
