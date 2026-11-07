# SPDX-License-Identifier: FSL-1.1-ALv2
"""File convenience methods share the agent's bounded search and safe archive implementation.
Parts (cubepals_pryx/): local_files.py handles caller disk paths; files.py owns binary I/O.
"""
import asyncio,hashlib,stat,tempfile
from urllib.parse import urlencode
from . import local_files as disk
from ._transport import PryxError,_checked
from .generated.defaults import SDK_DEFAULTS
class FileConveniences:
    def exists(self,path):
        try:self.stat(path);return True
        except PryxError as error:
            if error.status==404:return False
            raise
    def find(self,glob):return self._agent.request_v2('/_pryx/files/find','POST',{'pattern':glob})['paths']
    def grep(self,pattern,*,paths):return self._agent.request_v2('/_pryx/files/grep','POST',{'pattern':pattern,'paths':paths})['matches']
    def replace(self,pattern,text,*,paths):return self._agent.request_v2('/_pryx/files/replace','POST',{'pattern':pattern,'text':text,'paths':paths})
    def chmod(self,path,mode):return self._agent.request_v2('/_pryx/files/chmod','POST',{'path':path,'mode':mode})
    def upload(self,local_path,remote,*,on_progress=None):
        with disk.open_file(local_path) as file:
            total=file.seek(0,2);file.seek(0);return self.write(remote,disk.chunks(file,on_progress,total))
    def download(self,remote,local_path,*,on_progress=None):
        total=self.stat(remote)['size'];size=0
        with disk.open_file(local_path,True) as file:
            for part in self.read_stream(remote):
                file.write(part);size+=len(part)
                if on_progress:on_progress(size,total)
    def upload_dir(self,local_dir,remote,*,on_progress=None):
        self._agent.require_version()
        with tempfile.TemporaryFile() as file:
            disk.pack(local_dir,file)
            return self._agent.raw('/_pryx/files/archive?'+urlencode({'path':remote}),'PUT',content=disk.chunks(file,on_progress))
    def download_dir(self,remote,local_dir,*,on_progress=None):
        self._agent.require_version();url,headers=self._agent.target('/_pryx/files/archive?'+urlencode({'path':remote}));size=0
        with tempfile.TemporaryFile() as file,self._agent.http.stream('GET',url,headers=headers) as response:
            if not response.is_success:response.read();_checked(response)
            for part in response.iter_bytes(SDK_DEFAULTS['fileChunkBytes']):
                file.write(part);size+=len(part)
                if size>SDK_DEFAULTS['archiveBytes']:raise ValueError('Archive exceeds byte budget')
                if on_progress:on_progress(size,None)
            file.seek(0);disk.extract(file,local_dir)
    def sync(self,local_dir,remote_dir):
        self.mkdir(remote_dir);uploaded=[];unchanged=[]
        for path,name,info in disk.entries(local_dir):
            remote=remote_dir+'/'+name
            if stat.S_ISDIR(info.st_mode):self.mkdir(remote);continue
            with disk.open_file(path) as file:local=disk.digest(disk.chunks(file))
            try:same=local==disk.digest(self.read_stream(remote))
            except PryxError as error:
                if error.status!=404:raise
                same=False
            if same:unchanged.append(name)
            else:self.upload(path,remote);uploaded.append(name)
        return {'uploaded':uploaded,'unchanged':unchanged}
class AsyncFileConveniences:
    async def exists(self,path):
        try:await self.stat(path);return True
        except PryxError as error:
            if error.status==404:return False
            raise
    async def find(self,glob):return (await self._agent.request_v2('/_pryx/files/find','POST',{'pattern':glob}))['paths']
    async def grep(self,pattern,*,paths):return (await self._agent.request_v2('/_pryx/files/grep','POST',{'pattern':pattern,'paths':paths}))['matches']
    async def replace(self,pattern,text,*,paths):return await self._agent.request_v2('/_pryx/files/replace','POST',{'pattern':pattern,'text':text,'paths':paths})
    async def chmod(self,path,mode):return await self._agent.request_v2('/_pryx/files/chmod','POST',{'path':path,'mode':mode})
    async def upload(self,local_path,remote,*,on_progress=None):
        file=await asyncio.to_thread(disk.open_file,local_path);total=file.seek(0,2);file.seek(0);size=0
        async def chunks():
            nonlocal size
            while part:=await asyncio.to_thread(file.read,SDK_DEFAULTS['fileChunkBytes']):
                size+=len(part)
                if on_progress:on_progress(size,total)
                yield part
        try:return await self.write(remote,chunks())
        finally:file.close()
    async def download(self,remote,local_path,*,on_progress=None):
        total=(await self.stat(remote))['size'];size=0;file=await asyncio.to_thread(disk.open_file,local_path,True)
        try:
            async for part in self.read_stream(remote):
                await asyncio.to_thread(file.write,part);size+=len(part)
                if on_progress:on_progress(size,total)
        finally:file.close()
    async def upload_dir(self,local_dir,remote,*,on_progress=None):
        await self._agent.require_version()
        with tempfile.TemporaryFile() as file:
            await asyncio.to_thread(disk.pack,local_dir,file);size=0
            async def chunks():
                nonlocal size
                while part:=await asyncio.to_thread(file.read,SDK_DEFAULTS['fileChunkBytes']):
                    size+=len(part)
                    if on_progress:on_progress(size,None)
                    yield part
            return await self._agent.raw('/_pryx/files/archive?'+urlencode({'path':remote}),'PUT',content=chunks())
    async def download_dir(self,remote,local_dir,*,on_progress=None):
        await self._agent.require_version();url,headers=await self._agent.target('/_pryx/files/archive?'+urlencode({'path':remote}));size=0
        with tempfile.TemporaryFile() as file:
            async with self._agent.http.stream('GET',url,headers=headers) as response:
                if not response.is_success:await response.aread();_checked(response)
                async for part in response.aiter_bytes(SDK_DEFAULTS['fileChunkBytes']):
                    await asyncio.to_thread(file.write,part);size+=len(part)
                    if size>SDK_DEFAULTS['archiveBytes']:raise ValueError('Archive exceeds byte budget')
                    if on_progress:on_progress(size,None)
            file.seek(0);await asyncio.to_thread(disk.extract,file,local_dir)
    async def sync(self,local_dir,remote_dir):
        await self.mkdir(remote_dir);uploaded=[];unchanged=[]
        for path,name,info in disk.entries(local_dir):
            remote=remote_dir+'/'+name
            if stat.S_ISDIR(info.st_mode):await self.mkdir(remote);continue
            with disk.open_file(path) as file:local=await asyncio.to_thread(disk.digest,disk.chunks(file))
            value=hashlib.sha256()
            try:
                async for part in self.read_stream(remote):value.update(part)
                same=local==value.hexdigest()
            except PryxError as error:
                if error.status!=404:raise
                same=False
            if same:unchanged.append(name)
            else:await self.upload(path,remote);uploaded.append(name)
        return {'uploaded':uploaded,'unchanged':unchanged}
