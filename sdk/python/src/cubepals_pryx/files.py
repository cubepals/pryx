# SPDX-License-Identifier: FSL-1.1-ALv2
"""File chunks use exec and one bounded tar member. No archive is extracted on the client."""
import base64, io, tarfile
from urllib.parse import quote, urlencode
from .api import PryxError
from .generated.defaults import SDK_DEFAULTS
CHUNK_BYTES = SDK_DEFAULTS['fileChunkBytes']
def path_for(sandbox, path): return f'/v1/sandboxes/{quote(sandbox, safe="")}/file-chunks/{quote(path, safe="/")}'
def decode_chunk(response):
    try:
        data = base64.b64decode(response['archiveBase64'], validate=True)
        with tarfile.open(fileobj=io.BytesIO(data), mode='r:') as archive:
            member = archive.next()
            if member is None or member.name != 'data' or not member.isfile() or not 0 <= member.size <= CHUNK_BYTES: raise ValueError('Invalid member')
            file = archive.extractfile(member)
            if file is None: raise ValueError('Missing data')
            result = file.read(CHUNK_BYTES + 1)
            if len(result) != member.size: raise ValueError('Truncated member')
            return result
    except (ValueError, KeyError, tarfile.TarError) as error:
        raise PryxError(0, {'error': {'code': 'invalid_archive', 'message': 'Invalid file chunk archive'}}) from error
def chunks(data):
    if isinstance(data, str): data = data.encode()
    if not isinstance(data, (bytes, bytearray, memoryview)): raise TypeError('File data is str or bytes')
    for offset in range(0, max(1,len(data)), CHUNK_BYTES):
        yield {'offset': offset, 'dataBase64': base64.b64encode(data[offset:offset+CHUNK_BYTES]).decode('ascii')}
def data_chunks(data):
    if isinstance(data, str): data = data.encode()
    if isinstance(data, (bytes, bytearray, memoryview)):
        for at in range(0, len(data), CHUNK_BYTES): yield bytes(data[at:at+CHUNK_BYTES])
    elif hasattr(data, 'read'):
        while chunk := data.read(CHUNK_BYTES): yield chunk
    else:
        for chunk in data: yield from data_chunks(chunk)

async def async_chunks(data):
    if hasattr(data, '__aiter__'):
        async for chunk in data:
            for part in data_chunks(chunk): yield part
    else:
        for chunk in data_chunks(data): yield chunk

class FileContext:
    def __init__(self, api, sandbox, agent=None, view=None): self.api, self.sandbox, self.agent, self.view = api, sandbox, agent, view
    def running(self):
        from .generated.types import Unset
        return self.agent is not None and self.view().state == 'running' and not isinstance(self.view().agent, Unset) and self.view().agent is not None
    def node(self):
        from .generated.types import Unset
        return self.view is not None and not isinstance(self.view().files, Unset) and self.view().files is True
    def path(self, kind, path): return f'/v1/sandboxes/{quote(self.sandbox, safe="")}/data/{kind}?{urlencode({"path": path})}'
    def need_agent(self):
        if self.agent is None: raise PryxError(0, {'error': {'code': 'unsupported', 'message': 'This sandbox has no agent'}})
        return self.agent

class Files(FileContext):
    def write(self, path, data):
        from .api import _checked
        if self.running():
            self.agent.raw(f'/_pryx/files/content?{urlencode({"path":path})}', 'PUT', content=data_chunks(data))
        elif self.node(): _checked(self.api._client.get_httpx_client().put(self.path('content', path), content=data_chunks(data)))
        else:
            offset, wrote = 0, False
            for part in data_chunks(data):
                self.api.request(path_for(self.sandbox, path), 'PUT', {'offset': offset, 'dataBase64': base64.b64encode(part).decode('ascii')})
                offset += len(part); wrote = True
            if not wrote: self.api.request(path_for(self.sandbox, path), 'PUT', {'offset': 0, 'dataBase64': ''})
    def read_stream(self, path):
        from .api import _checked
        if self.running() or self.node():
            url, headers = self.agent.target(f'/_pryx/files/content?{urlencode({"path":path})}') if self.running() else (self.path('content', path), {})
            with self.api._client.get_httpx_client().stream('GET', url, headers=headers) as response:
                if response.is_error: response.read(); _checked(response)
                yield from response.iter_bytes(CHUNK_BYTES)
        else:
            offset = 0
            while True:
                chunk = decode_chunk(self.api.request(f'{path_for(self.sandbox, path)}?offset={offset}'))
                if chunk: yield chunk
                offset += len(chunk)
                if len(chunk) < CHUNK_BYTES: break
    def read_bytes(self, path): return b''.join(self.read_stream(path))
    def read(self, path): return self.read_bytes(path).decode()
    def list(self, path='.'):
        return (self.agent.request(f'/_pryx/files/list?{urlencode({"path":path})}') if self.running() else self.api.request(self.path('list', path)))['files']
    def stat(self, path): return self.agent.request(f'/_pryx/files/stat?{urlencode({"path":path})}') if self.running() else self.api.request(self.path('stat', path))
    def mkdir(self, path): return self.need_agent().request('/_pryx/files/mkdir', 'POST', {'path':path})
    def move(self, source, target): return self.need_agent().request('/_pryx/files/move', 'POST', {'from':source, 'to':target})
    def delete(self, path): return self.need_agent().request(f'/_pryx/files/content?{urlencode({"path":path})}', 'DELETE')
    def upload(self, path, url): return self.need_agent().request('/_pryx/files/upload', 'POST', {'path':path, 'url':url})
    def download(self, path, url): return self.need_agent().request('/_pryx/files/download', 'POST', {'path':path, 'url':url})
    def url(self, path, method='GET'):
        agent = self.need_agent()
        grant = agent.request('/_pryx/files/token', 'POST', {'path':path, 'method':method})
        return {**grant, 'url':agent.token()['url'].rstrip('/')+grant['url']}
    def watch(self, path):
        import json
        with self.need_agent().socket(f'/_pryx/files/watch?{urlencode({"path":path})}') as socket:
            for frame in socket: yield json.loads(frame)

class AsyncFiles(FileContext):
    async def write(self, path, data):
        from .api import _checked
        if self.running(): await self.agent.raw(f'/_pryx/files/content?{urlencode({"path":path})}', 'PUT', content=async_chunks(data))
        elif self.node(): _checked(await self.api._http.put(self.path('content', path), content=async_chunks(data)))
        else:
            offset, wrote = 0, False
            async for part in async_chunks(data):
                await self.api.request(path_for(self.sandbox, path), 'PUT', {'offset': offset, 'dataBase64': base64.b64encode(part).decode('ascii')})
                offset += len(part); wrote = True
            if not wrote: await self.api.request(path_for(self.sandbox, path), 'PUT', {'offset':0, 'dataBase64':''})
    async def read_stream(self, path):
        from .api import _checked
        if self.running() or self.node():
            url, headers = await self.agent.target(f'/_pryx/files/content?{urlencode({"path":path})}') if self.running() else (self.path('content', path), {})
            async with self.api._http.stream('GET', url, headers=headers) as response:
                if response.is_error: await response.aread(); _checked(response)
                async for chunk in response.aiter_bytes(CHUNK_BYTES): yield chunk
        else:
            offset = 0
            while True:
                chunk = decode_chunk(await self.api.request(f'{path_for(self.sandbox, path)}?offset={offset}'))
                if chunk: yield chunk
                offset += len(chunk)
                if len(chunk) < CHUNK_BYTES: break
    async def read_bytes(self, path): return b''.join([chunk async for chunk in self.read_stream(path)])
    async def read(self, path): return (await self.read_bytes(path)).decode()
    async def list(self, path='.'):
        return (await self.agent.request(f'/_pryx/files/list?{urlencode({"path":path})}') if self.running() else await self.api.request(self.path('list', path)))['files']
    async def stat(self, path): return await self.agent.request(f'/_pryx/files/stat?{urlencode({"path":path})}') if self.running() else await self.api.request(self.path('stat', path))
    async def mkdir(self, path): return await self.need_agent().request('/_pryx/files/mkdir', 'POST', {'path':path})
    async def move(self, source, target): return await self.need_agent().request('/_pryx/files/move', 'POST', {'from':source, 'to':target})
    async def delete(self, path): return await self.need_agent().request(f'/_pryx/files/content?{urlencode({"path":path})}', 'DELETE')
    async def upload(self, path, url): return await self.need_agent().request('/_pryx/files/upload', 'POST', {'path':path, 'url':url})
    async def download(self, path, url): return await self.need_agent().request('/_pryx/files/download', 'POST', {'path':path, 'url':url})
    async def url(self, path, method='GET'):
        agent = self.need_agent()
        grant = await agent.request('/_pryx/files/token', 'POST', {'path':path, 'method':method})
        return {**grant, 'url':(await agent.token())['url'].rstrip('/')+grant['url']}
    async def watch(self, path):
        import json
        socket = await self.need_agent().socket(f'/_pryx/files/watch?{urlencode({"path":path})}')
        try:
            async for frame in socket: yield json.loads(frame)
        finally: await socket.close()
