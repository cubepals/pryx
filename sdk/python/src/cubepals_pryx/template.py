# SPDX-License-Identifier: FSL-1.1-ALv2
"""Code-template builds keep local copy paths on the caller and persist completed snapshots."""
from pathlib import Path
from urllib.parse import quote
from ._transport import PryxError
from .generated.defaults import SDK_DEFAULTS
def _failed(result):
    """A failed build step, saying how it ended: its stderr alone can hold only warnings."""
    ended=f'timed out after {result.duration_ms} ms' if result.timed_out else 'ended by a signal' if result.code is None else f'exit {result.code}'
    return PryxError(0,{'error':{'code':'template_failed','message':f'Template command failed ({ended}): {result.stderr}'}})
class Template:
    def __init__(self,view,rebuild):self.view,self._rebuild=view,rebuild
    @property
    def id(self):return self.view['id']
    @property
    def name(self):return self.view['name']
    @property
    def version(self):return self.view['version']
    def rebuild(self):return self._rebuild()
def build(client,options):
    box=client.sandbox(**{key:options[source] for key,source in [('image','from'),('isolation','isolation'),('env','env'),('workdir','workdir'),('spare_ports','sparePorts')] if source in options});registered=False
    try:
        for local,remote in options.get('copy',[]):
            if Path(local).is_dir():box.files.upload_dir(local,remote)
            else:box.files.upload(local,remote)
        for command in [*options.get('run',[]),*([options['ready']] if options.get('ready') else [])]:
            result=box.run(command,timeout=SDK_DEFAULTS['templateStepTimeoutSeconds'])
            if result.code!=0:raise _failed(result)
        snapshot=box.snapshot();box.stop()
        view=client._transport().request('/v1/templates/'+quote(options['name'],safe='')+'/versions','POST',{'snapshot':snapshot.id,**({'start':options['start']} if options.get('start') else {}),'plan':{key:value for key,value in options.items() if key!='copy'}})
        registered=True;return Template(view,lambda:build(client,options))
    finally:
        if not registered:box.delete()
async def async_build(client,options):
    box=await client.sandbox(**{key:options[source] for key,source in [('image','from'),('isolation','isolation'),('env','env'),('workdir','workdir'),('spare_ports','sparePorts')] if source in options});registered=False
    try:
        for local,remote in options.get('copy',[]):
            if Path(local).is_dir():await box.files.upload_dir(local,remote)
            else:await box.files.upload(local,remote)
        for command in [*options.get('run',[]),*([options['ready']] if options.get('ready') else [])]:
            result=await box.run(command,timeout=SDK_DEFAULTS['templateStepTimeoutSeconds'])
            if result.code!=0:raise _failed(result)
        snapshot=await box.snapshot();await box.stop()
        view=await client._http.request('/v1/templates/'+quote(options['name'],safe='')+'/versions','POST',{'snapshot':snapshot.id,**({'start':options['start']} if options.get('start') else {}),'plan':{key:value for key,value in options.items() if key!='copy'}})
        registered=True;return Template(view,lambda:async_build(client,options))
    finally:
        if not registered:await box.delete()

class Webhook:
    def __init__(self,http,view):self._http,self.view,self.secret=http,{key:value for key,value in view.items() if key!='secret'},view['secret']
    @property
    def id(self):return self.view['id']
    def delete(self):return self._http.request('/v1/webhooks/'+quote(self.id,safe=''),'DELETE')
    def deadletters(self):return self._http.request('/v1/webhooks/'+quote(self.id,safe='')+'/deadletters')['deliveries']
class AsyncWebhook(Webhook):
    async def delete(self):return await self._http.request('/v1/webhooks/'+quote(self.id,safe=''),'DELETE')
    async def deadletters(self):return (await self._http.request('/v1/webhooks/'+quote(self.id,safe='')+'/deadletters'))['deliveries']
