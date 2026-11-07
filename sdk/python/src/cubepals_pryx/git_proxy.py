# SPDX-License-Identifier: FSL-1.1-ALv2
"""A transient guest HTTP bridge adapts virtual Git sockets to the VM's policy-controlled fetch."""
from uuid import uuid4
from urllib.parse import urlparse
from .generated.git_proxy import GIT_PROXY_SOURCE
from .generated.defaults import SDK_DEFAULTS
from ._transport import PryxError

def options(url):
    parsed=urlparse(url);origin=parsed.scheme+'://'+parsed.netloc;nonce=uuid4().hex
    return origin,nonce,dict(timeout=SDK_DEFAULTS['runTimeoutSeconds'],env={'PRYX_PROXY_ORIGIN':origin,'PRYX_PROXY_NONCE':nonce,'PRYX_PROXY_MAX_BODY':str(SDK_DEFAULTS['guestFrameBytes'])})
def configuration(origin,nonce,port):
    host='127.0.0.1:'+str(port);base='http://'+host+'/'+nonce+'/'
    return dict(host=host,protocol='http',args=['-c','url.'+base+'.insteadOf='+origin+'/','-c','credential.useHttpPath=true'])
def port(text):
    import re
    match=re.search(r'PRYX_GIT_PROXY_READY (\d+)\n',text)
    return int(match[1]) if match else None
def failed():return PryxError(0,{'error':{'code':'git_proxy_failed','message':'Git HTTP bridge failed before listening'}})
class GitProxy:
    def __init__(self,process,config):self.process,self.config=process,config
    @classmethod
    def start(cls,spawn,url):
        origin,nonce,config=options(url);process=spawn(['node','-e',GIT_PROXY_SOURCE],**config);text='';stream=process.stream()
        try:
            for chunk in stream:
                if chunk['stream']=='stderr':raise failed()
                text+=chunk['data']
                if len(text)>SDK_DEFAULTS['captureBytes']:raise failed()
                value=port(text)
                if value:return cls(process,configuration(origin,nonce,value))
            raise failed()
        except BaseException:process.kill();raise
        finally:stream.close()
    def close(self):
        try:self.process.kill()
        except PryxError:
            if self.process.wait().code is None:raise
        self.process.wait()
class AsyncGitProxy(GitProxy):
    @classmethod
    async def start(cls,spawn,url):
        origin,nonce,config=options(url);process=await spawn(['node','-e',GIT_PROXY_SOURCE],**config);text='';stream=process.stream()
        try:
            async for chunk in stream:
                if chunk['stream']=='stderr':raise failed()
                text+=chunk['data']
                if len(text)>SDK_DEFAULTS['captureBytes']:raise failed()
                value=port(text)
                if value:return cls(process,configuration(origin,nonce,value))
            raise failed()
        except BaseException:await process.kill();raise
        finally:await stream.aclose()
    async def close(self):
        try:await self.process.kill()
        except PryxError:
            if (await self.process.wait()).code is None:raise
        await self.process.wait()
