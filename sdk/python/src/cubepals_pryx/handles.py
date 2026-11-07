# SPDX-License-Identifier: FSL-1.1-ALv2
"""One lifecycle and eager-command surface, with private HTTP and capability transports."""
from urllib.parse import quote, urlencode
from uuid import uuid4
from .git import Git,AsyncGit
from .shell import Shell,AsyncShell
from .duration import seconds
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
        self.git=Git(self.run,self.spawn,self.isolation)
        self.files=Files(http,self.id,self._agent,lambda:self._view)
    @property
    def id(self):return self._view.id
    @property
    def state(self):return self._view.state
    @property
    def isolation(self):return None if isinstance(self._view.isolation,Unset) else self._view.isolation
    @property
    def units(self):return None if isinstance(self._view.units,Unset) else self._view.units
    def _command_options(self,options):
        cwd=getattr(self._view,'workdir',None)
        return ({'cwd':cwd,**options} if cwd is not None and not isinstance(cwd,Unset) else options)
    def _path(self,suffix=''):return f'/v1/sandboxes/{quote(self.id,safe="")}{suffix}'
    def refresh(self):self._view=SandboxView.from_dict(self._http.request(self._path()));self._agent.invalidate();return self
    def spawn(self,command,**options):return self._agent.start(command,**self._command_options(options))
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
    def processes(self):return self._agent.processes()
    def kill_all(self):return self._agent.request_v2('/_pryx/processes/kill-all','POST')
    def shell(self,**options):return Shell(self._agent,self._agent.request_v2('/_pryx/shells','POST',self._command_options(options))['id'])
    def sync(self,local_dir,remote_dir):return self.files.sync(local_dir,remote_dir)
    def _lifetime(self,**body):self._view=SandboxView.from_dict(self._http.request(self._path('/lifetime'),'POST',body));return self
    def keep_alive(self,duration):return self._lifetime(keepAlive=seconds(duration))
    def set_ttl(self,duration):return self._lifetime(ttl=seconds(duration))
    def set_idle(self,duration):return self._lifetime(idle=seconds(duration))
    def metrics(self,*,since=None):return self._http.request(self._path('/metrics')+('?' + urlencode({'since':since}) if since else ''))
    def wait_until(self,state,timeout=SDK_DEFAULTS['runTimeoutSeconds']):
        import time
        from ._transport import PryxError
        deadline=time.monotonic()+seconds(timeout)
        while time.monotonic()<deadline:
            self.refresh()
            if self.state==state:return self
            if self.state=='failed':raise PryxError(0,{'error':{'code':'sandbox_failed','message':'Sandbox entered failed state'}})
            time.sleep(0.1)
        raise PryxError(0,{'error':{'code':'wait_timeout','message':f'Sandbox did not reach {state}'}})
    def events(self,since=None,*,follow=False):
        query=urlencode({'stream':'true','follow':str(follow).lower(),**({'since':since} if since else {})})
        yield from self._http.lines(self._path('/events')+'?'+query)
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
        self.git=AsyncGit(self.run,self.spawn,self.isolation)
        self.files=AsyncFiles(http,self.id,self._agent,lambda:self._view)
    async def refresh(self):self._view=SandboxView.from_dict(await self._http.request(self._path()));self._agent.invalidate();return self
    async def spawn(self,command,**options):return await self._agent.start(command,**self._command_options(options))
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
    async def processes(self):return await self._agent.processes()
    async def kill_all(self):return await self._agent.request_v2('/_pryx/processes/kill-all','POST')
    async def shell(self,**options):return AsyncShell(self._agent,(await self._agent.request_v2('/_pryx/shells','POST',self._command_options(options)))['id'])
    async def sync(self,local_dir,remote_dir):return await self.files.sync(local_dir,remote_dir)
    async def _lifetime(self,**body):self._view=SandboxView.from_dict(await self._http.request(self._path('/lifetime'),'POST',body));return self
    async def keep_alive(self,duration):return await self._lifetime(keepAlive=seconds(duration))
    async def set_ttl(self,duration):return await self._lifetime(ttl=seconds(duration))
    async def set_idle(self,duration):return await self._lifetime(idle=seconds(duration))
    async def metrics(self,*,since=None):return await self._http.request(self._path('/metrics')+('?' + urlencode({'since':since}) if since else ''))
    async def wait_until(self,state,timeout=SDK_DEFAULTS['runTimeoutSeconds']):
        import asyncio
        from ._transport import PryxError
        deadline=asyncio.get_running_loop().time()+seconds(timeout)
        while asyncio.get_running_loop().time()<deadline:
            await self.refresh()
            if self.state==state:return self
            if self.state=='failed':raise PryxError(0,{'error':{'code':'sandbox_failed','message':'Sandbox entered failed state'}})
            await asyncio.sleep(0.1)
        raise PryxError(0,{'error':{'code':'wait_timeout','message':f'Sandbox did not reach {state}'}})
    async def events(self,since=None,*,follow=False):
        query=urlencode({'stream':'true','follow':str(follow).lower(),**({'since':since} if since else {})})
        async for event in self._http.lines(self._path('/events')+'?'+query):yield event
