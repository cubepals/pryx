# SPDX-License-Identifier: FSL-1.1-ALv2
"""Binary files over the agent or stopped sandbox data; no shell, archive or encoded chunks.
Parts (cubepals_pryx/): _transport.py owns HTTP pools; agent.py owns scoped capabilities.
"""
import json
from urllib.parse import quote,urlencode
from ._transport import _checked
from .generated.defaults import SDK_DEFAULTS

def data_chunks(data):
    if isinstance(data,str):data=data.encode()
    if isinstance(data,(bytes,bytearray,memoryview)):
        for at in range(0,len(data),SDK_DEFAULTS['fileChunkBytes']):yield bytes(data[at:at+SDK_DEFAULTS['fileChunkBytes']])
    elif hasattr(data,'read'):
        while chunk:=data.read(SDK_DEFAULTS['fileChunkBytes']):yield chunk
    else:
        for chunk in data:yield from data_chunks(chunk)

async def async_chunks(data):
    if hasattr(data,'__aiter__'):
        async for chunk in data:
            for part in data_chunks(chunk):yield part
    else:
        for chunk in data_chunks(data):yield chunk

class Files:
    def __init__(self,http,sandbox,agent,view):self._http,self._sandbox,self._agent,self._view=http,sandbox,agent,view
    def _running(self):return self._view().state=='running'
    def _path(self,kind,path):return f'/v1/sandboxes/{quote(self._sandbox,safe="")}/data/{kind}?{urlencode({"path":path})}'
    def write(self,path,data):
        if self._running():self._agent.raw('/_pryx/files/content?'+urlencode({'path':path}),'PUT',content=data_chunks(data))
        else:self._http.transfer(self._path('content',path),'PUT',data_chunks(data))
    def read_stream(self,path):
        url,headers=self._agent.target('/_pryx/files/content?'+urlencode({'path':path})) if self._running() else (self._path('content',path),{'authorization':f'Bearer {self._http._key}'})
        with self._http.http.stream('GET',url,headers=headers) as response:
            if not response.is_success:response.read();_checked(response)
            yield from response.iter_bytes(SDK_DEFAULTS['fileChunkBytes'])
    def read_bytes(self,path):return b''.join(self.read_stream(path))
    def read(self,path):return self.read_bytes(path).decode()
    def list(self,path='.'):
        return (self._agent.request('/_pryx/files/list?'+urlencode({'path':path})) if self._running() else self._http.request(self._path('list',path)))['files']
    def stat(self,path):return self._agent.request('/_pryx/files/stat?'+urlencode({'path':path})) if self._running() else self._http.request(self._path('stat',path))
    def mkdir(self,path):return self._agent.request('/_pryx/files/mkdir','POST',{'path':path})
    def move(self,source,target):return self._agent.request('/_pryx/files/move','POST',{'from':source,'to':target})
    def delete(self,path):return self._agent.request('/_pryx/files/content?'+urlencode({'path':path}),'DELETE')
    def upload(self,path,url):return self._agent.request('/_pryx/files/upload','POST',{'path':path,'url':url})
    def download(self,path,url):return self._agent.request('/_pryx/files/download','POST',{'path':path,'url':url})
    def url(self,path,method='GET'):
        grant=self._agent.request('/_pryx/files/token','POST',{'path':path,'method':method})
        return {**grant,'url':self._agent.token()['url'].rstrip('/')+grant['url']}
    def watch(self,path):
        with self._agent.socket('/_pryx/files/watch?'+urlencode({'path':path})) as socket:
            for frame in socket:yield json.loads(frame)

class AsyncFiles(Files):
    async def write(self,path,data):
        if self._running():await self._agent.raw('/_pryx/files/content?'+urlencode({'path':path}),'PUT',content=async_chunks(data))
        else:await self._http.transfer(self._path('content',path),'PUT',async_chunks(data))
    async def read_stream(self,path):
        url,headers=await self._agent.target('/_pryx/files/content?'+urlencode({'path':path})) if self._running() else (self._path('content',path),{'authorization':f'Bearer {self._http._key}'})
        async with self._http.http.stream('GET',url,headers=headers) as response:
            if not response.is_success:await response.aread();_checked(response)
            async for chunk in response.aiter_bytes(SDK_DEFAULTS['fileChunkBytes']):yield chunk
    async def read_bytes(self,path):return b''.join([chunk async for chunk in self.read_stream(path)])
    async def read(self,path):return (await self.read_bytes(path)).decode()
    async def list(self,path='.'):
        return (await self._agent.request('/_pryx/files/list?'+urlencode({'path':path})) if self._running() else await self._http.request(self._path('list',path)))['files']
    async def stat(self,path):return await self._agent.request('/_pryx/files/stat?'+urlencode({'path':path})) if self._running() else await self._http.request(self._path('stat',path))
    async def mkdir(self,path):return await self._agent.request('/_pryx/files/mkdir','POST',{'path':path})
    async def move(self,source,target):return await self._agent.request('/_pryx/files/move','POST',{'from':source,'to':target})
    async def delete(self,path):return await self._agent.request('/_pryx/files/content?'+urlencode({'path':path}),'DELETE')
    async def upload(self,path,url):return await self._agent.request('/_pryx/files/upload','POST',{'path':path,'url':url})
    async def download(self,path,url):return await self._agent.request('/_pryx/files/download','POST',{'path':path,'url':url})
    async def url(self,path,method='GET'):
        grant=await self._agent.request('/_pryx/files/token','POST',{'path':path,'method':method})
        return {**grant,'url':(await self._agent.token())['url'].rstrip('/')+grant['url']}
    async def watch(self,path):
        socket=await self._agent.socket('/_pryx/files/watch?'+urlencode({'path':path}))
        try:
            async for frame in socket:yield json.loads(frame)
        finally:await socket.close()
