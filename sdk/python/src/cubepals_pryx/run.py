# SPDX-License-Identifier: FSL-1.1-ALv2
"""Single-consumer command values. Iteration is live; collecting never sends a second command."""
from dataclasses import dataclass
from .api import PryxError

@dataclass
class RunResult:
    stdout: str = ""
    stderr: str = ""
    code: int | None = None
    timed_out: bool = False
    killed: bool = False
    duration_ms: int = 0
    stdout_truncated: bool = False
    stderr_truncated: bool = False

def _event(result, event):
    kind = event.get("type")
    if kind in ("stdout", "stderr"):
        setattr(result, kind, getattr(result, kind) + event["data"])
        setattr(result, kind + "_truncated", getattr(result, kind + "_truncated") or event["truncated"])
        return {"stream": kind, "data": event["data"]}
    if kind == "exit":
        result.code = event["code"]
        result.timed_out = event["timedOut"]
        result.killed = event["killed"]
        result.duration_ms = event["durationMs"]
    elif kind == "error":
        raise PryxError(0, {"error": event.get("error", {})})
    return None

def _interrupted():
    return PryxError(0, {"error": {"code": "exec_interrupted", "message": "Exec ended without an exit result; its outcome is unknown"}})

class Run:
    def __init__(self, events):
        self._events = events
        self._result = RunResult()
        self._started = False
        self._done = False
        self._error = None
    def stream(self):
        if self._started: raise RuntimeError("A run has one stream consumer; use wait() for its result")
        self._started = True
        exited = False
        try:
            for event in self._events:
                chunk = _event(self._result, event)
                if event.get("type") == "exit": exited = True
                if chunk: yield chunk
            if not exited: raise _interrupted()
            self._done = True
        except BaseException as error:
            self._error = error if isinstance(error, Exception) else _interrupted()
            raise
        finally:
            if not self._done and self._error is None: self._error = _interrupted()
            self._events.close()
    def lines(self):
        rest = {"stdout": "", "stderr": ""}
        for chunk in self.stream():
            kind = chunk["stream"]
            rest[kind] += chunk["data"]
            while "\n" in rest[kind]:
                line, rest[kind] = rest[kind].split("\n", 1)
                yield line.removesuffix("\r")
        yield from (line for line in rest.values() if line)
    def wait(self):
        if self._error: raise self._error
        if not self._started:
            for _ in self.stream(): pass
        if not self._done: raise RuntimeError("The command stream is still being consumed")
        return self._result
    def __getattr__(self, name):
        if name in RunResult.__dataclass_fields__: return getattr(self.wait(), name)
        raise AttributeError(name)

class AsyncRun:
    def __init__(self, events):
        self._events = events
        self._result = RunResult()
        self._started = False
        self._done = False
        self._error = None
    async def stream(self):
        if self._started: raise RuntimeError("A run has one stream consumer; await it for its result")
        self._started = True
        exited = False
        try:
            async for event in self._events:
                chunk = _event(self._result, event)
                if event.get("type") == "exit": exited = True
                if chunk: yield chunk
            if not exited: raise _interrupted()
            self._done = True
        except BaseException as error:
            self._error = error if isinstance(error, Exception) else _interrupted()
            raise
        finally:
            if not self._done and self._error is None: self._error = _interrupted()
            await self._events.aclose()
    async def lines(self):
        rest = {"stdout": "", "stderr": ""}
        async for chunk in self.stream():
            kind = chunk["stream"]
            rest[kind] += chunk["data"]
            while "\n" in rest[kind]:
                line, rest[kind] = rest[kind].split("\n", 1)
                yield line.removesuffix("\r")
        for line in rest.values():
            if line: yield line
    async def wait(self):
        if self._error: raise self._error
        if not self._started:
            async for _ in self.stream(): pass
        if not self._done: raise RuntimeError("The command stream is still being consumed")
        return self._result
    def __await__(self): return self.wait().__await__()
