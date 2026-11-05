# SPDX-License-Identifier: FSL-1.1-ALv2
"""Public client exports. Parts (cubepals_pryx/): api.py is the thin client; generated/ owns wire models."""
from uuid import uuid4
from .api import Pryx as Api, PryxError, MakeRequest
from .async_api import AsyncApi
from .duration import make_options
from .handles import Sandbox, Snapshot, AsyncSandbox, AsyncSnapshot
from .generated.models import Sandbox as SandboxView
from .run import RunResult

class Pryx(Api):
    def __init__(self, base_url=None, api_key=None, *, url=None, key=None, **options):
        super().__init__(url if url is not None else base_url, key if key is not None else api_key, **options)
    @property
    def api(self): return self
    def sandbox(self, sandbox=None, *, key=None, **options):
        if sandbox is not None and options: raise ValueError('Use an id or make options')
        return Sandbox(self.api, self.api.get(sandbox) if sandbox is not None else self.api.make(make_options(options), (key if key is not None else str(uuid4()))))
    def sandboxes(self): return [Sandbox(self.api, SandboxView.from_dict(view)) for view in self.api.list()['sandboxes']]
    def run(self, image, command, **options):
        box = self.sandbox(image=image)
        try: return box.run(command, **options).wait()
        finally: box.delete()

class AsyncPryx(AsyncApi):
    def __init__(self, url, key, **options): super().__init__(url, key, **options)
    @property
    def api(self): return self
    async def __aenter__(self): return self
    async def __aexit__(self, *_): await self.close()
    async def sandbox(self, sandbox=None, *, key=None, **options):
        if sandbox is not None and options: raise ValueError('Use an id or make options')
        return AsyncSandbox(self.api, await self.api.get(sandbox) if sandbox is not None else await self.api.make(make_options(options), (key if key is not None else str(uuid4()))))
    async def sandboxes(self): return [AsyncSandbox(self.api, SandboxView.from_dict(view)) for view in (await self.api.list())['sandboxes']]
    async def run(self, image, command, **options):
        box = await self.sandbox(image=image)
        try: return await box.run(command, **options)
        finally: await box.delete()
