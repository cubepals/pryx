# SPDX-License-Identifier: FSL-1.1-ALv2
"""Bounded UTF-8 agent log decoding for both verified HTTP pools."""
import base64
import codecs
from datetime import datetime, timezone
from .generated.defaults import SDK_DEFAULTS

class Decoder:
    def __init__(self, *, tail=None, since=None, follow=False):
        if tail is not None and (not isinstance(tail,int) or tail<0): raise ValueError('tail is a non-negative integer')
        self.tail,self.follow=tail,follow
        self.since=float('-inf') if since is None else datetime.fromisoformat(since.replace('Z','+00:00')).timestamp()*1000
        self.rest={s:'' for s in ('stdout','stderr')}
        self.at={s:0 for s in self.rest}
        self.decoders={s:codecs.getincrementaldecoder('utf-8')(errors='replace') for s in self.rest}
    def record(self,stream,line):
        return {'ts':datetime.fromtimestamp(self.at[stream]/SDK_DEFAULTS['secondMs'],timezone.utc).isoformat().replace('+00:00','Z'),'stream':stream,'line':line}
    def chunk(self,chunk):
        stream=chunk['stream']
        if not self.rest[stream]:self.at[stream]=chunk['at']
        self.rest[stream]+=self.decoders[stream].decode(base64.b64decode(chunk['bytes'],validate=True))
        out=[]
        while '\n' in self.rest[stream] or len(self.rest[stream])>=SDK_DEFAULTS['captureBytes']:
            end=self.rest[stream].find('\n')
            size=min(end,SDK_DEFAULTS['captureBytes']) if end>=0 else SDK_DEFAULTS['captureBytes']
            if self.at[stream]>=self.since:out.append(self.record(stream,self.rest[stream][:size].removesuffix('\r')))
            self.rest[stream]=self.rest[stream][size+(end==size):]
            self.at[stream]=chunk['at']
        return out
    def flush(self):
        out=[]
        for stream in self.rest:
            self.rest[stream]+=self.decoders[stream].decode(b'',final=True)
            if self.rest[stream] and self.at[stream]>=self.since:out.append(self.record(stream,self.rest[stream]))
            self.rest[stream]=''
        return out
    def frame(self,frame):
        if frame['type']=='replay':
            out=[line for chunk in frame['chunks'] for line in self.chunk(chunk)]
            if not self.follow:out+=self.flush()
            return [] if self.tail==0 else out if self.tail is None else out[-self.tail:]
        if frame['type']=='chunk':return self.chunk(frame['chunk'])
        return [{'event':'gap','reason':'Agent log history or subscriber output was truncated'}]
    def finish(self):return self.flush() if self.follow else []
