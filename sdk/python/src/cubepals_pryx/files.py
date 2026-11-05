# SPDX-License-Identifier: FSL-1.1-ALv2
"""File chunks use exec and one bounded tar member. No archive is extracted on the client."""
import base64, io, tarfile
from urllib.parse import quote
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
class Files:
    def __init__(self, api, sandbox): self.api, self.sandbox = api, sandbox
    def write(self, path, data):
        for chunk in chunks(data): self.api.request(path_for(self.sandbox, path), 'PUT', chunk)
    def read_bytes(self, path):
        parts, offset = [], 0
        while True:
            chunk = decode_chunk(self.api.request(f'{path_for(self.sandbox, path)}?offset={offset}'))
            parts.append(chunk); offset += len(chunk)
            if len(chunk) < CHUNK_BYTES: return b''.join(parts)
    def read(self, path): return self.read_bytes(path).decode()
class AsyncFiles:
    def __init__(self, api, sandbox): self.api, self.sandbox = api, sandbox
    async def write(self, path, data):
        for chunk in chunks(data): await self.api.request(path_for(self.sandbox, path), 'PUT', chunk)
    async def read_bytes(self, path):
        parts, offset = [], 0
        while True:
            chunk = decode_chunk(await self.api.request(f'{path_for(self.sandbox, path)}?offset={offset}'))
            parts.append(chunk); offset += len(chunk)
            if len(chunk) < CHUNK_BYTES: return b''.join(parts)
    async def read(self, path): return (await self.read_bytes(path)).decode()
