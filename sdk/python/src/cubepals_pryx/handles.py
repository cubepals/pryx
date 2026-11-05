# SPDX-License-Identifier: FSL-1.1-ALv2
"""Synchronous and asynchronous sandbox handles over the stable thin API, with explicit refresh."""
from uuid import uuid4
from .duration import make_options, run_options
from .files import Files, AsyncFiles
from .generated.models import Sandbox as SandboxView
from .generated.types import Unset
from .run import Run, AsyncRun
class Snapshot:
    def __init__(self, api, view, sandbox): self.api, self.view, self.sandbox = api, view, sandbox
    @property
    def id(self): return self.view.id
    def fork(self, *, key=None, **options): return Sandbox(self.api, self.api.fork(self.id, make_options(options), (key if key is not None else str(uuid4()))))
    def delete(self): return self.api.delete_snapshot(self.sandbox, self.id)
class Sandbox:
    def __init__(self, api, view):
        self.api, self.view = api, view
        self.files = Files(api, self.id)
    @property
    def id(self): return self.view.id
    @property
    def state(self): return self.view.state
    @property
    def units(self): return None if isinstance(self.view.units, Unset) else self.view.units
    def refresh(self): self.view = self.api.get(self.id); return self
    def run(self, command, *, key=None, **options): return Run(self.api.exec(self.id, **run_options(command, options), key=(key if key is not None else str(uuid4()))))
    def write(self, path, data): return self.files.write(path, data)
    def read(self, path): return self.files.read(path)
    def read_bytes(self, path): return self.files.read_bytes(path)
    def snapshot(self): return Snapshot(self.api, self.api.snapshot(self.id), self.id)
    def _action(self, verb): self.view = SandboxView.from_dict(self.api.action(self.id, verb)['sandbox']); return self
    def sleep(self): return self._action('sleep')
    def wake(self): return self._action('wake')
    def stop(self): return self._action('stop')
    def start(self): return self._action('start')
    def rest(self): return self._action('rest')
    def delete(self): return self.api.delete(self.id)
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
        self.files = AsyncFiles(api, self.id)
    @property
    def id(self): return self.view.id
    @property
    def state(self): return self.view.state
    @property
    def units(self): return None if isinstance(self.view.units, Unset) else self.view.units
    async def refresh(self): self.view = await self.api.get(self.id); return self
    def run(self, command, *, key=None, **options): return AsyncRun(self.api.exec(self.id, **run_options(command, options), key=(key if key is not None else str(uuid4()))))
    async def write(self, path, data): return await self.files.write(path, data)
    async def read(self, path): return await self.files.read(path)
    async def read_bytes(self, path): return await self.files.read_bytes(path)
    async def snapshot(self): return AsyncSnapshot(self.api, await self.api.snapshot(self.id), self.id)
    async def _action(self, verb): self.view = SandboxView.from_dict((await self.api.action(self.id, verb))['sandbox']); return self
    async def sleep(self): return await self._action('sleep')
    async def wake(self): return await self._action('wake')
    async def stop(self): return await self._action('stop')
    async def start(self): return await self._action('start')
    async def rest(self): return await self._action('rest')
    async def delete(self): return await self.api.delete(self.id)
    async def endpoints(self): return (await self.api.endpoints(self.id))['endpoints']
    def logs(self, **options): return self.api.logs(self.id, **options)
