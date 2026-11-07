# SPDX-License-Identifier: FSL-1.1-ALv2
"""Git parsing and fixed argv plans keep credentials in one-call environment helpers only."""
from urllib.parse import urlparse
from uuid import uuid4
from .git_proxy import GitProxy,AsyncGitProxy
from ._transport import PryxError
from .generated.defaults import SDK_DEFAULTS

def args(command,token=None,path=None,bridge=None):
    return ['env',*sum((['-u',key] for key in ['GIT_CURL_VERBOSE','GIT_TRACE_CURL','GIT_TRACE','GIT_TRACE_PACKET','GIT_TRACE_PACK_ACCESS','GIT_TRACE_SETUP','GIT_TRACE_PERFORMANCE','GIT_TRACE2','GIT_TRACE2_EVENT','GIT_TRACE2_PERF','GIT_TRACE2_ENV_VARS','GIT_TRACE2_CONFIG_PARAMS','GIT_TRACE2_BRIEF','GIT_TRACE2_EVENT_BRIEF','GIT_CONFIG_PARAMETERS','GIT_SSL_NO_VERIFY']),[]),'git',*((bridge or {}).get('args',[])),'-c','credential.helper=','-c','http.followRedirects=false','-c','core.hooksPath=/dev/null','-c','core.fsmonitor=false','-c','maintenance.auto=false','-c','gc.auto=0',*(['-C',path] if path else []),*(['-c','credential.helper=!f() { [ "$1" = get ] || return 0; protocol=; host=; while IFS="=" read -r key value; do case "$key" in protocol) protocol=$value;; host) host=$value;; esac; done; [ "$protocol" = "$PRYX_GIT_PROTOCOL" ] && [ "$host" = "$PRYX_GIT_HOST" ] || return 1; printf "%s\\n" "username=x-access-token" "password=$PRYX_GIT_TOKEN"; }; f'] if token else []),*command]
def env(token=None,extra=None,host="",protocol="https"):
    return {'GIT_TERMINAL_PROMPT':'0','GIT_CONFIG_NOSYSTEM':'1','GIT_CONFIG_GLOBAL':'/dev/null','GIT_CONFIG_COUNT':'0',**(extra or {}),'PRYX_GIT_TOKEN':token or '', 'PRYX_GIT_HOST':host,'PRYX_GIT_PROTOCOL':protocol}
def checked(raw,command):
    if raw.stdout_truncated:raise PryxError(0,{'error':{'code':'git_output_truncated','message':'Git stdout exceeded the capture budget; parsed output is incomplete'}})
    if raw.code!=0:raise PryxError(0,{'error':{'code':'git_failed','message':f'git {command[0]} failed: {raw.stderr}','details':{'raw':vars(raw)}}})
    return raw
def status(raw):
    fields=iter(raw.stdout.split('\0'));entries=[]
    for field in fields:
        if not field:continue
        entries.append({'status':field[:2],'path':field[3:]})
        if any(code in field[:2] for code in 'RC'):next(fields,None)
    return {'value':{'clean':not entries,'entries':entries},'raw':raw}
def history(raw):
    fields=raw.stdout.split('\0')
    return {'value':[{'hash':fields[i].strip(),'author':{'name':fields[i+1],'email':fields[i+2]},'at':fields[i+3],'subject':fields[i+4]} for i in range(0,len(fields)-4,5)],'raw':raw}
def clone_args(url,path,branch,depth,token,isolation=None):
    remote=urlparse(url)
    if remote.scheme in ('http','https') and (remote.username or remote.password or remote.query or remote.fragment):raise ValueError('Git URLs cannot contain credentials, queries or fragments')
    if token and (not url.startswith('https://') or any(c in token for c in '\r\n\0')):raise ValueError('Git tokens require HTTPS and a single line')
    if depth is not None and (not isinstance(depth,int) or isinstance(depth,bool) or depth<=0):raise ValueError('depth must be a positive integer')
    return ['clone',*(['--separate-git-dir=/tmp/.pryx-git-'+str(uuid4())] if isolation=='wasm' else []),*(['--branch',branch] if branch else []),*(['--depth',str(depth)] if depth else []),'--',url,path]
class Git:
    def __init__(self,run,spawn=None,isolation=None):self._run,self._path,self._spawn,self._isolation=run,'repo',spawn,isolation
    def _call(self,command,*,token=None,extra=None,root=False,credential_url=None,timeout=None):
        duration=timeout if timeout is not None else SDK_DEFAULTS['gitTimeoutSeconds' if command[0] in ('clone','push','pull') else 'runTimeoutSeconds']
        host=''
        if token:
            remote=urlparse(credential_url or self._call(['remote','get-url','origin'],timeout=timeout).stdout.strip())
            if remote.scheme!='https' or remote.username or remote.password or remote.query or remote.fragment or any(c in token for c in '\r\n\0'):raise ValueError('Git token requires a clean HTTPS remote and a single line')
            host=remote.netloc
        bridge=None
        if self._isolation=='wasm' and self._spawn and command[0] in ['clone','push','pull']:
            remote=credential_url or (self._call(['remote','get-url','origin'],timeout=timeout)).stdout.strip()
            if remote.startswith(('http:','https:')):bridge=GitProxy.start(self._spawn,remote,duration)
        config=bridge.config if bridge else {}
        if token and bridge:host=config['host']
        try:
            raw=self._run(args(command,token,None if root else self._path,config),env=env(token,extra,host,config.get('protocol','https')),timeout=duration)
            if token:raw.stdout=raw.stdout.replace(token,'[redacted]');raw.stderr=raw.stderr.replace(token,'[redacted]')
            return checked(raw,command)
        finally:
            if bridge:bridge.close()
    def clone(self,url,*,path='repo',branch=None,depth=None,token=None,timeout=None):
        raw=self._call(clone_args(url,path,branch,depth,token,self._isolation),token=token,root=True,credential_url=url,timeout=timeout);self._path=path;return {'value':{'path':path},'raw':raw}
    def status(self,*,timeout=None):return status(self._call(['status','--porcelain=v1','-z'],timeout=timeout))
    def diff(self,*,staged=False,timeout=None):
        raw=self._call(['diff',*(['--cached'] if staged else []),'--no-ext-diff','--no-textconv'],timeout=timeout);return {'value':{'patch':raw.stdout},'raw':raw}
    def branch(self,name=None,*,timeout=None):
        raw=self._call(['branch','--',name] if name else ['branch','--format=%(refname:short)'],timeout=timeout);return {'value':raw.stdout.splitlines(),'raw':raw}
    def checkout(self,ref,*,create=False,timeout=None):return {'value':{'ref':ref},'raw':self._call(['checkout',*(['-b'] if create else []),ref,'--'],timeout=timeout)}
    def add(self,paths,*,timeout=None):return {'value':{'added':True},'raw':self._call(['add','--',*([paths] if isinstance(paths,str) else paths)],timeout=timeout)}
    def commit(self,message,*,author=None,timeout=None):
        extra={f'GIT_{role}_{key.upper()}':value for role in ['AUTHOR','COMMITTER'] for key,value in (author or {}).items()}
        raw=self._call(['commit','--no-gpg-sign','-m',message],extra=extra,timeout=timeout)
        return {'value':{'hash':self._call(['rev-parse','HEAD'],timeout=timeout).stdout.strip()},'raw':raw}
    def push(self,*,token=None,timeout=None):return {'value':{'pushed':True},'raw':self._call(['push'],token=token,timeout=timeout)}
    def pull(self,*,timeout=None):return {'value':{'pulled':True},'raw':self._call(['pull','--ff-only'],timeout=timeout)}
    def log(self,*,limit=10,timeout=None):
        if not isinstance(limit,int) or isinstance(limit,bool) or limit<=0:raise ValueError('limit must be a positive integer')
        return history(self._call(['log',f'-{limit}','--format=%H%x00%an%x00%ae%x00%aI%x00%s%x00'],timeout=timeout))
class AsyncGit(Git):
    async def _call(self,command,*,token=None,extra=None,root=False,credential_url=None,timeout=None):
        duration=timeout if timeout is not None else SDK_DEFAULTS['gitTimeoutSeconds' if command[0] in ('clone','push','pull') else 'runTimeoutSeconds']
        host=''
        if token:
            remote=urlparse(credential_url or (await self._call(['remote','get-url','origin'],timeout=timeout)).stdout.strip())
            if remote.scheme!='https' or remote.username or remote.password or remote.query or remote.fragment or any(c in token for c in '\r\n\0'):raise ValueError('Git token requires a clean HTTPS remote and a single line')
            host=remote.netloc
        bridge=None
        if self._isolation=='wasm' and self._spawn and command[0] in ['clone','push','pull']:
            remote=credential_url or (await self._call(['remote','get-url','origin'],timeout=timeout)).stdout.strip()
            if remote.startswith(('http:','https:')):bridge=await AsyncGitProxy.start(self._spawn,remote,duration)
        config=bridge.config if bridge else {}
        if token and bridge:host=config['host']
        try:
            raw=await self._run(args(command,token,None if root else self._path,config),env=env(token,extra,host,config.get('protocol','https')),timeout=duration)
            if token:raw.stdout=raw.stdout.replace(token,'[redacted]');raw.stderr=raw.stderr.replace(token,'[redacted]')
            return checked(raw,command)
        finally:
            if bridge:await bridge.close()
    async def clone(self,url,*,path='repo',branch=None,depth=None,token=None,timeout=None):
        raw=await self._call(clone_args(url,path,branch,depth,token,self._isolation),token=token,root=True,credential_url=url,timeout=timeout);self._path=path;return {'value':{'path':path},'raw':raw}
    async def status(self,*,timeout=None):return status(await self._call(['status','--porcelain=v1','-z'],timeout=timeout))
    async def diff(self,*,staged=False,timeout=None):
        raw=await self._call(['diff',*(['--cached'] if staged else []),'--no-ext-diff','--no-textconv'],timeout=timeout);return {'value':{'patch':raw.stdout},'raw':raw}
    async def branch(self,name=None,*,timeout=None):
        raw=await self._call(['branch','--',name] if name else ['branch','--format=%(refname:short)'],timeout=timeout);return {'value':raw.stdout.splitlines(),'raw':raw}
    async def checkout(self,ref,*,create=False,timeout=None):return {'value':{'ref':ref},'raw':await self._call(['checkout',*(['-b'] if create else []),ref,'--'],timeout=timeout)}
    async def add(self,paths,*,timeout=None):return {'value':{'added':True},'raw':await self._call(['add','--',*([paths] if isinstance(paths,str) else paths)],timeout=timeout)}
    async def commit(self,message,*,author=None,timeout=None):
        extra={f'GIT_{role}_{key.upper()}':value for role in ['AUTHOR','COMMITTER'] for key,value in (author or {}).items()}
        raw=await self._call(['commit','--no-gpg-sign','-m',message],extra=extra,timeout=timeout)
        return {'value':{'hash':(await self._call(['rev-parse','HEAD'],timeout=timeout)).stdout.strip()},'raw':raw}
    async def push(self,*,token=None,timeout=None):return {'value':{'pushed':True},'raw':await self._call(['push'],token=token,timeout=timeout)}
    async def pull(self,*,timeout=None):return {'value':{'pulled':True},'raw':await self._call(['pull','--ff-only'],timeout=timeout)}
    async def log(self,*,limit=10,timeout=None):
        if not isinstance(limit,int) or isinstance(limit,bool) or limit<=0:raise ValueError('limit must be a positive integer')
        return history(await self._call(['log',f'-{limit}','--format=%H%x00%an%x00%ae%x00%aI%x00%s%x00'],timeout=timeout))
