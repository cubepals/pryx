# SPDX-License-Identifier: FSL-1.1-ALv2
"""One SDK facade; sync and async context managers share the same names.
Parts (cubepals_pryx/): _transport.py owns private HTTP; handles.py owns identities; agent.py and
async_agent.py own capabilities; files.py owns binary data; run.py owns eager observation;
duration.py parses durations; logs.py decodes framed history; generated/ owns option and result models.
"""
import asyncio as _asyncio
from uuid import uuid4 as _uuid4
from ._transport import Transport as _Transport,AsyncTransport as _AsyncTransport,PryxError
from .handles import Sandbox,Snapshot,AsyncSandbox as _AsyncSandbox
from .agent import Process,Terminal
from .files import Files
from .duration import make_options as _make_options
from .generated.models import Sandbox as _SandboxView,MakeRequest
from .run import RunResult
from urllib.parse import quote as _quote

__all__=['Pryx','Sandbox','Snapshot','Process','Terminal','Files','PryxError','RunResult','MakeRequest']

class Pryx:
    def __init__(self,*,url,key,transport=None,timeout=None):
        self._url,self._key,self._options=url,key,{'transport':transport,'timeout':timeout}
        self._http=None
    def _transport(self):
        if self._http is None:self._http=_Transport(self._url,self._key,**self._options)
        return self._http
    def close(self):
        if self._http is not None:self._http.close()
    def __enter__(self):return self
    def __exit__(self,*_):self.close()
    async def __aenter__(self):
        self._async=_AsyncPryx(self._url,self._key,**self._options)
        return self._async
    async def __aexit__(self,*_):await self._async.close()
    def sandbox(self,sandbox=None,*,key=None,**options):
        if sandbox is not None and options:raise ValueError('Use an id or create options')
        http=self._transport()
        view=http.request('/v1/sandboxes/'+_quote(sandbox,safe='')) if sandbox is not None else http.request('/v1/sandboxes','POST',_make_options(options),key or str(_uuid4()))
        return Sandbox(http,_SandboxView.from_dict(view))
    def sandboxes(self):return [Sandbox(self._transport(),_SandboxView.from_dict(view)) for view in self._transport().request('/v1/sandboxes')['sandboxes']]
    def project(self):return self._transport().request('/v1/project')
    def run(self,image,command,**options):
        box=self.sandbox(image=image)
        try:return box.run(command,**options)
        finally:box.delete()

class _AsyncPryx:
    def __init__(self,url,key,**options):self._http=_AsyncTransport(url,key,**options)
    async def close(self):await self._http.close()
    async def sandbox(self,sandbox=None,*,key=None,**options):
        if sandbox is not None and options:raise ValueError('Use an id or create options')
        view=await self._http.request('/v1/sandboxes/'+_quote(sandbox,safe='')) if sandbox is not None else await self._http.request('/v1/sandboxes','POST',_make_options(options),key or str(_uuid4()))
        return _AsyncSandbox(self._http,_SandboxView.from_dict(view))
    async def sandboxes(self):return [_AsyncSandbox(self._http,_SandboxView.from_dict(view)) for view in (await self._http.request('/v1/sandboxes'))['sandboxes']]
    async def project(self):return await self._http.request('/v1/project')
    def run(self,image,command,**options):
        async def execute():
            box=await self.sandbox(image=image)
            try:return await box.run(command,**options)
            finally:await box.delete()
        return _asyncio.Task(execute(),loop=_asyncio.get_running_loop(),eager_start=True)
