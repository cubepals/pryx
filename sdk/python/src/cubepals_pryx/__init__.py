# SPDX-License-Identifier: FSL-1.1-ALv2
"""One SDK facade; sync and async context managers share the same names.
Parts (cubepals_pryx/): _transport.py owns private HTTP; handles.py owns identities; agent.py and
async_agent.py own capabilities; files.py owns binary data; run.py owns eager observation;
git.py and git_proxy.py own Git; shell.py owns sessions; template.py owns builds/webhooks;
file_conveniences.py and local_files.py own caller disk operations; duration.py parses durations; logs.py decodes framed history; generated/ owns option and result models.
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
from urllib.parse import quote as _quote,urlencode as _urlencode
import json as _json

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
    def sandbox(self,sandbox=None,*,key=None,wake=True,**options):
        if sandbox is not None and options:raise ValueError('Use an id or create options')
        http=self._transport();existing=sandbox is not None
        if options.get('repo') and not any(options.get(key) for key in ['image','template','snapshot']) and options.get('isolation') in (None,'container'):
            defaults=self.project().get('defaults',{})
            if options.get('isolation',defaults.get('isolation'))=='container':options['image']=defaults.get('repoImage')
            if options.get('isolation',defaults.get('isolation'))=='wasm':options['packages']=list(dict.fromkeys([*options.get('packages',[]),'git']))
        if options.get('repo') and options.get('isolation')=='wasm':options['packages']=list(dict.fromkeys([*options.get('packages',[]),'git']))
        if sandbox is None and set(options)=={'name'}:
            boxes=self.sandboxes(name=options['name'])
            if not boxes:raise PryxError(404,{'error':{'code':'not_found','message':'No sandbox with that name'}})
            box=boxes[0];existing=True
        else:
            repo=options.pop('repo',None);branch=options.pop('branch',None);token=options.pop('token',None)
            view=http.request('/v1/sandboxes/'+_quote(sandbox,safe='')) if existing else http.request('/v1/sandboxes','POST',_make_options(options),key or str(_uuid4()))
            box=Sandbox(http,_SandboxView.from_dict(view))
            if repo:
                try:box.git.clone(repo,branch=branch,token=token)
                except BaseException as error:
                    if isinstance(error,PryxError):error.details={'sandboxId':box.id,'cause':error.details}
                    error.add_note('Sandbox retained for inspection: '+box.id);raise
        if existing and wake and box.state in ('sleeping','resting'):box.wake()
        return box
    def sandboxes(self,*,labels=None,state=None,name=None):
        query=_urlencode({key:value for key,value in {'labels':_json.dumps(labels) if labels is not None else None,'state':state,'name':name}.items() if value is not None})
        return [Sandbox(self._transport(),_SandboxView.from_dict(view)) for view in (self._transport().request('/v1/sandboxes'+('?' +query if query else '')))['sandboxes']]
    def template(self,**options):
        from .template import build
        if 'from_' in options:options['from']=options.pop('from_')
        return build(self,options)
    def templates(self):return (self._transport().request('/v1/templates'))['templates']
    def webhook(self,*,url,kinds):
        from .template import Webhook
        return Webhook(self._transport(),self._transport().request('/v1/webhooks','POST',{'url':url,'kinds':kinds}))
    def webhooks(self):return self._transport().request('/v1/webhooks')['webhooks']
    def project(self):return self._transport().request('/v1/project')
    def run(self,image,command,**options):
        box=self.sandbox(image=image)
        try:return box.run(command,**options)
        finally:box.delete()

class _AsyncPryx:
    def __init__(self,url,key,**options):self._http=_AsyncTransport(url,key,**options)
    async def close(self):await self._http.close()
    async def sandbox(self,sandbox=None,*,key=None,wake=True,**options):
        if sandbox is not None and options:raise ValueError('Use an id or create options')
        http=self._http;existing=sandbox is not None
        if options.get('repo') and not any(options.get(key) for key in ['image','template','snapshot']) and options.get('isolation') in (None,'container'):
            defaults=(await self.project()).get('defaults',{})
            if options.get('isolation',defaults.get('isolation'))=='container':options['image']=defaults.get('repoImage')
            if options.get('isolation',defaults.get('isolation'))=='wasm':options['packages']=list(dict.fromkeys([*options.get('packages',[]),'git']))
        if options.get('repo') and options.get('isolation')=='wasm':options['packages']=list(dict.fromkeys([*options.get('packages',[]),'git']))
        if sandbox is None and set(options)=={'name'}:
            boxes=await self.sandboxes(name=options['name'])
            if not boxes:raise PryxError(404,{'error':{'code':'not_found','message':'No sandbox with that name'}})
            box=boxes[0];existing=True
        else:
            repo=options.pop('repo',None);branch=options.pop('branch',None);token=options.pop('token',None)
            view=await http.request('/v1/sandboxes/'+_quote(sandbox,safe='')) if existing else await http.request('/v1/sandboxes','POST',_make_options(options),key or str(_uuid4()))
            box=_AsyncSandbox(http,_SandboxView.from_dict(view))
            if repo:
                try:await box.git.clone(repo,branch=branch,token=token)
                except BaseException as error:
                    if isinstance(error,PryxError):error.details={'sandboxId':box.id,'cause':error.details}
                    error.add_note('Sandbox retained for inspection: '+box.id);raise
        if existing and wake and box.state in ('sleeping','resting'):await box.wake()
        return box
    async def sandboxes(self,*,labels=None,state=None,name=None):
        query=_urlencode({key:value for key,value in {'labels':_json.dumps(labels) if labels is not None else None,'state':state,'name':name}.items() if value is not None})
        return [_AsyncSandbox(self._http,_SandboxView.from_dict(view)) for view in (await self._http.request('/v1/sandboxes'+('?' +query if query else '')))['sandboxes']]
    async def template(self,**options):
        from .template import async_build
        if 'from_' in options:options['from']=options.pop('from_')
        return await async_build(self,options)
    async def templates(self):return (await self._http.request('/v1/templates'))['templates']
    async def webhook(self,*,url,kinds):
        from .template import AsyncWebhook
        return AsyncWebhook(self._http,await self._http.request('/v1/webhooks','POST',{'url':url,'kinds':kinds}))
    async def webhooks(self):return (await self._http.request('/v1/webhooks'))['webhooks']
    async def project(self):return await self._http.request('/v1/project')
    def run(self,image,command,**options):
        async def execute():
            box=await self.sandbox(image=image)
            try:return await box.run(command,**options)
            finally:await box.delete()
        return _asyncio.Task(execute(),loop=_asyncio.get_running_loop(),eager_start=True)
