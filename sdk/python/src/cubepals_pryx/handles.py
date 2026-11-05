# SPDX-License-Identifier: FSL-1.1-ALv2
"""Synchronous and asynchronous sandbox handles over the stable thin API, with explicit refresh."""
from uuid import uuid4
from .duration import make_options, run_options
from .files import Files, AsyncFiles
from .generated.models import Sandbox as SandboxView
from .generated.types import Unset
from .run import Run, AsyncRun
from .agent import Agent
from .async_agent import AsyncAgent
from .generated.defaults import SDK_DEFAULTS

def has_agent(view): return not isinstance(view.agent, Unset) and view.agent is not None
class Snapshot:
    def __init__(self, api, view, sandbox): self.api, self.view, self.sandbox = api, view, sandbox
    @property
    def id(self): return self.view.id
    def fork(self, *, key=None, **options): return Sandbox(self.api, self.api.fork(self.id, make_options(options), (key if key is not None else str(uuid4()))))
    def delete(self): return self.api.delete_snapshot(self.sandbox, self.id)
class Sandbox:
    def __init__(self, api, view):
        self.api, self.view = api, view
        self.agent = Agent(api, self.id)
        self.files = Files(api, self.id, self.agent, lambda: self.view)
    @property
    def id(self): return self.view.id
    @property
    def state(self): return self.view.state
    @property
    def units(self): return None if isinstance(self.view.units, Unset) else self.view.units
    def refresh(self): self.view = self.api.get(self.id); self.agent.invalidate(); return self
    def run(self, command, *, key=None, **options):
        if not has_agent(self.view): return Run(self.api.exec(self.id, **run_options(command, options), key=(key if key is not None else str(uuid4()))))
        def events():
            process = self.agent.start(command, key=key, **{'timeout':SDK_DEFAULTS['runTimeoutSeconds'], **options})
            yield from process.run_events()
        return Run(events())
    def write(self, path, data): return self.files.write(path, data)
    def read(self, path): return self.files.read(path)
    def read_bytes(self, path): return self.files.read_bytes(path)
    def read_stream(self, path): return self.files.read_stream(path)
    def terminal(self, **options): return self.agent.terminal(**options)
    def expose(self, port): return self.agent.expose(port)
    def unexpose(self, port): return self.agent.unexpose(port)
    def network(self, mode):
        self.api.request(f'/v1/sandboxes/{self.id}/network', 'POST', {'network':mode})
        return self.refresh()
    def snapshot(self): return Snapshot(self.api, self.api.snapshot(self.id), self.id)
    def _action(self, verb): self.view = SandboxView.from_dict(self.api.action(self.id, verb)['sandbox']); self.agent.invalidate(); return self
    def sleep(self): return self._action('sleep')
    def wake(self): return self._action('wake')
    def stop(self): return self._action('stop')
    def start(self, command=None, **options): return self._action('start') if command is None else self.agent.start(command, **options)
    def rest(self): return self._action('rest')
    def delete(self): self.agent.invalidate(); return self.api.delete(self.id)
    def endpoints(self): return self.api.endpoints(self.id)['endpoints']
    def logs(self, **options): return self.api.logs(self.id, **options)
class AsyncSnapshot:
    def __init__(self, api, view, sandbox): self.api, self.view, self.sandbox = api, view, sandbox
    @property
    def id(self): return self.view.id
    async def fork(self, *, key=None, **options): return AsyncSandbox(self.api, await self.api.fork(self.id, make_options(options), (key if key is not None else str(uuid4()))))
    async def delete(self): return await self.api.delete_snapshot(self.sandbox, self.id)
class AsyncSandbox:
    def __init__(self, api, view):
        self.api, self.view = api, view
        self.agent = AsyncAgent(api, self.id)
        self.files = AsyncFiles(api, self.id, self.agent, lambda: self.view)
    @property
    def id(self): return self.view.id
    @property
    def state(self): return self.view.state
    @property
    def units(self): return None if isinstance(self.view.units, Unset) else self.view.units
    async def refresh(self): self.view = await self.api.get(self.id); self.agent.invalidate(); return self
    def run(self, command, *, key=None, **options):
        if not has_agent(self.view): return AsyncRun(self.api.exec(self.id, **run_options(command, options), key=(key if key is not None else str(uuid4()))))
        async def events():
            process = await self.agent.start(command, key=key, **{'timeout':SDK_DEFAULTS['runTimeoutSeconds'], **options})
            async for event in process.run_events(): yield event
        return AsyncRun(events())
    async def write(self, path, data): return await self.files.write(path, data)
    async def read(self, path): return await self.files.read(path)
    async def read_bytes(self, path): return await self.files.read_bytes(path)
    def read_stream(self, path): return self.files.read_stream(path)
    async def terminal(self, **options): return await self.agent.terminal(**options)
    async def expose(self, port): return await self.agent.expose(port)
    async def unexpose(self, port): return await self.agent.unexpose(port)
    async def network(self, mode):
        await self.api.request(f'/v1/sandboxes/{self.id}/network', 'POST', {'network':mode})
        return await self.refresh()
    async def snapshot(self): return AsyncSnapshot(self.api, await self.api.snapshot(self.id), self.id)
    async def _action(self, verb): self.view = SandboxView.from_dict((await self.api.action(self.id, verb))['sandbox']); self.agent.invalidate(); return self
    async def sleep(self): return await self._action('sleep')
    async def wake(self): return await self._action('wake')
    async def stop(self): return await self._action('stop')
    async def start(self, command=None, **options): return await self._action('start') if command is None else await self.agent.start(command, **options)
    async def rest(self): return await self._action('rest')
    async def delete(self): self.agent.invalidate(); return await self.api.delete(self.id)
    async def endpoints(self): return (await self.api.endpoints(self.id))['endpoints']
    def logs(self, **options): return self.api.logs(self.id, **options)
