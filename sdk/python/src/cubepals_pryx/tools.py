# SPDX-License-Identifier: FSL-1.1-ALv2
"""Native service adapters share agent transport; SSH keys have no serializable/repr field."""
from urllib.parse import quote
from .generated.tools import TOOL_DEFAULTS
from ._transport import PryxError

class SshAccess:
    __slots__=('address','__key')
    def __init__(self,address,key): self.address,self.__key=address,key
    def __getattr__(self,name):
        fields={'host_key':'hostKey','expires_at':'expiresAt'}
        try:return self.address[fields.get(name,name)]
        except KeyError:raise AttributeError(name) from None
    @property
    def key(self):return self.__key
    def to_dict(self):return dict(self.address)
    def __repr__(self):return f'SshAccess({self.address!r})'

class Desktop:
    def __init__(self,agent):self.agent=agent
    def _check(self):
        health=self.agent.request('/_pryx/health')
        if health.get('toolsVersion',0)<TOOL_DEFAULTS['version']:raise PryxError(0,{'error':{'code':'unsupported','message':'This agent does not provide native tools'}})
    def _request(self,path,body=None):
        self._check()
        return self.agent.request('/_pryx/'+path,'POST',body)
    def __call__(self):return self._request('desktop')
    def screenshot(self):
        self._check()
        return self.agent.raw('/_pryx/desktop/screenshot','POST').content
    def click(self,x,y):return self._request('desktop/click',{'x':x,'y':y})
    def move(self,x,y):return self._request('desktop/move',{'x':x,'y':y})
    def type(self,text):return self._request('desktop/type',{'text':text})
    def key(self,combo):return self._request('desktop/key',{'combo':combo})
    def scroll(self,dx,dy):return self._request('desktop/scroll',{'dx':dx,'dy':dy})
class Browser(Desktop):
    def __call__(self):return self._request('browser')
    def screenshot(self,url):
        self._check()
        return self.agent.raw('/_pryx/browser/screenshot','POST',json_body={'url':url}).content
class Ssh(Desktop):
    def __init__(self,agent,http,id):super().__init__(agent);self.http,self.path=http,f'/v1/sandboxes/{quote(id,safe="")}/ssh'
    def __call__(self):
        created=self._request('ssh')
        try:return SshAccess(self.http.request(self.path,'POST',{'id':created['id']}),created['key'])
        except BaseException:
            try:self.agent.request('/_pryx/ssh/'+quote(created['id'],safe=''),'DELETE')
            except Exception:pass
            raise
    def revoke(self):return self.http.request(self.path,'DELETE')

class AsyncDesktop:
    def __init__(self,agent):self.agent=agent
    async def _check(self):
        health=await self.agent.request('/_pryx/health')
        if health.get('toolsVersion',0)<TOOL_DEFAULTS['version']:raise PryxError(0,{'error':{'code':'unsupported','message':'This agent does not provide native tools'}})
    async def _request(self,path,body=None):
        await self._check()
        return await self.agent.request('/_pryx/'+path,'POST',body)
    async def __call__(self):return await self._request('desktop')
    async def screenshot(self):
        await self._check()
        return (await self.agent.raw('/_pryx/desktop/screenshot','POST')).content
    async def click(self,x,y):return await self._request('desktop/click',{'x':x,'y':y})
    async def move(self,x,y):return await self._request('desktop/move',{'x':x,'y':y})
    async def type(self,text):return await self._request('desktop/type',{'text':text})
    async def key(self,combo):return await self._request('desktop/key',{'combo':combo})
    async def scroll(self,dx,dy):return await self._request('desktop/scroll',{'dx':dx,'dy':dy})
class AsyncBrowser(AsyncDesktop):
    async def __call__(self):return await self._request('browser')
    async def screenshot(self,url):
        await self._check()
        return (await self.agent.raw('/_pryx/browser/screenshot','POST',json_body={'url':url})).content
class AsyncSsh(AsyncDesktop):
    def __init__(self,agent,http,id):super().__init__(agent);self.http,self.path=http,f'/v1/sandboxes/{quote(id,safe="")}/ssh'
    async def __call__(self):
        created=await self._request('ssh')
        try:return SshAccess(await self.http.request(self.path,'POST',{'id':created['id']}),created['key'])
        except BaseException:
            try:await self.agent.request('/_pryx/ssh/'+quote(created['id'],safe=''),'DELETE')
            except Exception:pass
            raise
    async def revoke(self):return await self.http.request(self.path,'DELETE')
