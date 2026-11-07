# SPDX-License-Identifier: FSL-1.1-ALv2
"""Persistent shell handles delegate serialization and separate-stream framing to the agent."""
import re
from urllib.parse import quote
from .duration import seconds
from .generated.defaults import SDK_DEFAULTS
from .run import RunResult

def result(value):
    return RunResult(**{re.sub(r'([A-Z])',lambda m:'_'+m[1].lower(),key):item for key,item in value.items()})

class Shell:
    def __init__(self,agent,id):self._agent,self.id=agent,id
    def run(self,command,*,timeout=SDK_DEFAULTS['runTimeoutSeconds']):
        return result(self._agent.request_v2('/_pryx/shells/'+quote(self.id,safe='')+'/run','POST',{'command':command,'timeout':seconds(timeout)}))
    def close(self):return self._agent.request_v2('/_pryx/shells/'+quote(self.id,safe=''),'DELETE')

class AsyncShell(Shell):
    async def run(self,command,*,timeout=SDK_DEFAULTS['runTimeoutSeconds']):
        return result(await self._agent.request_v2('/_pryx/shells/'+quote(self.id,safe='')+'/run','POST',{'command':command,'timeout':seconds(timeout)}))
    async def close(self):return await self._agent.request_v2('/_pryx/shells/'+quote(self.id,safe=''),'DELETE')
