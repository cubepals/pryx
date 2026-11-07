# SPDX-License-Identifier: FSL-1.1-ALv2
"""One lifecycle and eager-command surface, with private HTTP and capability transports."""
from urllib.parse import quote, urlencode
from uuid import uuid4
from .agent import Agent
from .async_agent import AsyncAgent
from .duration import make_options
from .files import Files, AsyncFiles
from .generated.models import Sandbox as SandboxView, Snapshot as SnapshotView
from .generated.types import Unset
from .generated.defaults import SDK_DEFAULTS
from .run import CommandStream, AsyncCommandStream

class Snapshot:
    def __init__(self, http, view, sandbox): self._http,self._view,self._sandbox=http,view,sandbox
    @property
    def id(self): return self._view.id
    def fork(self, *, key=None, **options):
        view=self._http.request(f'/v1/snapshots/{quote(self.id,safe="")}/sandboxes','POST',make_options(options),key or str(uuid4()))
        return Sandbox(self._http,SandboxView.from_dict(view))
    def delete(self): return self._http.request(f'/v1/sandboxes/{quote(self._sandbox,safe="")}/snapshots/{quote(self.id,safe="")}','DELETE')

class Sandbox:
    def __init__(self,http,view):
        self._http,self._view=http,view
        self._agent=Agent(http,self.id)
        self.files=Files(http,self.id,self._agent,lambda:self._view)
    @property
    def id(self):return self._view.id
    @property
    def state(self):return self._view.state
    @property
    def isolation(self):return None if isinstance(self._view.isolation,Unset) else self._view.isolation
    @property
    def units(self):return None if isinstance(self._view.units,Unset) else self._view.units
    def _path(self,suffix=''):return f'/v1/sandboxes/{quote(self.id,safe="")}{suffix}'
    def refresh(self):self._view=SandboxView.from_dict(self._http.request(self._path()));self._agent.invalidate();return self
    def spawn(self,command,**options):return self._agent.start(command,**options)
    def run(self,command,**options):return CommandStream(self.spawn(command,**{'timeout':SDK_DEFAULTS['runTimeoutSeconds'],**options}),False).result()
    def stream(self,command,**options):return CommandStream(self.spawn(command,**{'timeout':SDK_DEFAULTS['runTimeoutSeconds'],**options}))
    def write(self,path,data):return self.files.write(path,data)
    def read(self,path):return self.files.read(path)
    def read_bytes(self,path):return self.files.read_bytes(path)
    def read_stream(self,path):return self.files.read_stream(path)
    def terminal(self,**options):return self._agent.terminal(**options)
    def mcp_url(self):
        token=self._agent.token()
        return token['url'].rstrip('/')+'/_pryx/mcp?'+urlencode({'token':token['token']})
    def expose(self,port):return self._agent.expose(port)
    def unexpose(self,port):return self._agent.unexpose(port)
    def network(self,mode):self._view=SandboxView.from_dict(self._http.request(self._path('/network'),'POST',{'network':mode}));self._agent.invalidate();return self
    def snapshot(self):return Snapshot(self._http,SnapshotView.from_dict(self._http.request(self._path('/snapshots'),'POST')),self.id)
    def snapshots(self):return [Snapshot(self._http,SnapshotView.from_dict(v),self.id) for v in self._http.request(self._path('/snapshots'))['snapshots']]
    def _action(self,verb):self._view=SandboxView.from_dict(self._http.request(self._path('/'+verb),'POST')['sandbox']);self._agent.invalidate();return self
    def start(self):return self._action('start')
    def stop(self):return self._action('stop')
    def sleep(self):return self._action('sleep')
    def wake(self):return self._action('wake')
    def rest(self):return self._action('rest')
    def delete(self):self._agent.invalidate();return self._http.request(self._path(),'DELETE')
    def endpoints(self):return self._http.request(self._path('/endpoints'))['endpoints']
    def events(self,since=None):return self._http.request(self._path('/events')+('?' + urlencode({'since':since}) if since else ''))
    def logs(self,**options):return self._agent.logs(**options)

class AsyncSnapshot(Snapshot):
    async def fork(self,*,key=None,**options):
        view=await self._http.request(f'/v1/snapshots/{quote(self.id,safe="")}/sandboxes','POST',make_options(options),key or str(uuid4()))
        return AsyncSandbox(self._http,SandboxView.from_dict(view))
    async def delete(self):return await self._http.request(f'/v1/sandboxes/{quote(self._sandbox,safe="")}/snapshots/{quote(self.id,safe="")}','DELETE')

class AsyncSandbox(Sandbox):
    def __init__(self,http,view):
        self._http,self._view=http,view
        self._agent=AsyncAgent(http,self.id)
        self.files=AsyncFiles(http,self.id,self._agent,lambda:self._view)
    async def refresh(self):self._view=SandboxView.from_dict(await self._http.request(self._path()));self._agent.invalidate();return self
    async def spawn(self,command,**options):return await self._agent.start(command,**options)
    def run(self,command,**options):return AsyncCommandStream(self.spawn(command,**{'timeout':SDK_DEFAULTS['runTimeoutSeconds'],**options}),False)._task
    def stream(self,command,**options):return AsyncCommandStream(self.spawn(command,**{'timeout':SDK_DEFAULTS['runTimeoutSeconds'],**options}))
    async def write(self,path,data):return await self.files.write(path,data)
    async def read(self,path):return await self.files.read(path)
    async def read_bytes(self,path):return await self.files.read_bytes(path)
    def read_stream(self,path):return self.files.read_stream(path)
    async def terminal(self,**options):return await self._agent.terminal(**options)
    async def mcp_url(self):
        token=await self._agent.token()
        return token['url'].rstrip('/')+'/_pryx/mcp?'+urlencode({'token':token['token']})
    async def expose(self,port):return await self._agent.expose(port)
    async def unexpose(self,port):return await self._agent.unexpose(port)
    async def network(self,mode):self._view=SandboxView.from_dict(await self._http.request(self._path('/network'),'POST',{'network':mode}));self._agent.invalidate();return self
    async def snapshot(self):return AsyncSnapshot(self._http,SnapshotView.from_dict(await self._http.request(self._path('/snapshots'),'POST')),self.id)
    async def snapshots(self):return [AsyncSnapshot(self._http,SnapshotView.from_dict(v),self.id) for v in (await self._http.request(self._path('/snapshots')))['snapshots']]
    async def _action(self,verb):self._view=SandboxView.from_dict((await self._http.request(self._path('/'+verb),'POST'))['sandbox']);self._agent.invalidate();return self
    async def start(self):return await self._action('start')
    async def stop(self):return await self._action('stop')
    async def sleep(self):return await self._action('sleep')
    async def wake(self):return await self._action('wake')
    async def rest(self):return await self._action('rest')
    async def delete(self):self._agent.invalidate();return await self._http.request(self._path(),'DELETE')
    async def endpoints(self):return (await self._http.request(self._path('/endpoints')))['endpoints']
    async def events(self,since=None):return await self._http.request(self._path('/events')+('?' + urlencode({'since':since}) if since else ''))
