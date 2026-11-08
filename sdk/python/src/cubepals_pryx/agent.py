# SPDX-License-Identifier: FSL-1.1-ALv2
"""Direct synchronous agent transport, processes and websocket terminals. Credentials stay in
headers or websocket subprotocols; lifecycle remains on the public API and mutations run once.
Parts (cubepals_pryx/): async_agent.py provides the asynchronous transport.
"""
import base64
import codecs
import json
import threading
import time
from uuid import uuid4
from urllib.parse import quote
from websockets.sync.client import connect
from ._transport import PryxError, _checked
from .generated.defaults import SDK_DEFAULTS

def output(events):
    decoders = {name: codecs.getincrementaldecoder('utf-8')(errors='replace') for name in ('stdout', 'stderr')}
    for event in events:
        kind = event['type']
        if kind == 'output':
            stream = event['stream']
            data = decoders[stream].decode(base64.b64decode(event['bytes'], validate=True))
            if data: yield {'type': stream, 'data': data, 'truncated': False}
        elif kind == 'exit':
            for stream, decoder in decoders.items():
                data = decoder.decode(b'', final=True)
                if data: yield {'type': stream, 'data': data, 'truncated': False}
            yield {'type': 'exit', **event['result']}
        else: yield {'type': 'stdout', 'data': '', 'truncated': True}

def start_body(command, options):
    from .duration import seconds
    body = {'command': command, **options}
    if 'timeout' in body:
        value = seconds(body['timeout'])
        if not float(value).is_integer(): raise ValueError('Process timeout is a whole number of seconds')
        body['timeout'] = int(value)
    return body

class Agent:
    def __init__(self, api, sandbox):
        self._api, self.id = api, sandbox
        self._credential = None
        self._lock = threading.RLock()
    @property
    def http(self): return self._api.http
    def invalidate(self):
        with self._lock: self._credential = None; self._version=None; self._kernel_version=None
    def token(self):
        with self._lock:
            if not self._credential or self._credential['expiresAt'] <= time.time() + SDK_DEFAULTS['tokenRenewBeforeSeconds']:
                self._credential = self._api.request(f'/v1/sandboxes/{quote(self.id, safe="")}/token', 'POST')
            return self._credential
    def target(self, path):
        token = self.token()
        return token['url'].rstrip('/') + path, {'authorization': f'Bearer {token["token"]}'}
    def raw(self, path, method='GET', *, content=None, json_body=None, key=None):
        url, headers = self.target(path)
        if key: headers['Idempotency-Key'] = key
        return _checked(self.http.request(method, url, headers=headers, content=content, json=json_body))
    def request(self, path, method='GET', body=None, key=None):
        response = self.raw(path, method, json_body=body, key=key)
        return response.json() if response.content else None
    def require_version(self):
        if getattr(self,'_version',None) is None:
            health=self.request('/_pryx/health')
            self._version=health.get('apiVersion',1)
        if self._version<SDK_DEFAULTS['agentVersion']:
            from ._transport import PryxError
            raise PryxError(0,{'error':{'code':'unsupported','message':'This operation requires agent API version 2'}})
    def request_v2(self,path,method='GET',body=None):
        self.require_version()
        return self.request(path,method,body)
    def require_kernels(self, version):
        if getattr(self, '_kernel_version', None) is None:
            self._kernel_version = self.request('/_pryx/health').get('kernelsVersion', 0)
        if self._kernel_version < version:
            raise PryxError(0, {'error': {'code': 'unsupported', 'message': 'This agent does not provide language kernels'}})
    def start(self, command, *, key=None, **options):
        stdin=options.get('stdin')
        if isinstance(stdin,(str,bytes,bytearray,memoryview)):options['stdin']=True
        process=Process(self,self.request('/_pryx/processes','POST',start_body(command,options),key if key is not None else str(uuid4())))
        if isinstance(stdin,(str,bytes,bytearray,memoryview)):
            data=stdin.encode() if isinstance(stdin,str) else bytes(stdin)
            try:
                for offset in range(0,len(data),SDK_DEFAULTS['stdinBytes']):
                    process.stdin.write(data[offset:offset+SDK_DEFAULTS['stdinBytes']])
                process.stdin.close()
            except BaseException as error:
                raise PryxError(0,{'error':{'code':'stdin_failed','message':'Input delivery failed; inspect the process before stopping it','details':{'processId':process.id}}}) from error
        return process
    def process(self, process): return Process(self, self.request(f'/_pryx/processes/{quote(process, safe="")}'))
    def processes(self): return [Process(self, view) for view in self.request('/_pryx/processes')['processes']]
    def expose(self, port): return self.request('/_pryx/ports', 'POST', {'port': port})['url']
    def unexpose(self, port): return self.request(f'/_pryx/ports/{port}', 'DELETE')
    def ports(self): return self.request('/_pryx/ports')['ports']
    def logs(self, **options):
        from .logs import Decoder
        from urllib.parse import urlencode
        decoder = Decoder(**options)
        url, headers = self.target("/_pryx/logs?"+urlencode({"follow":str(options.get("follow",False)).lower()}))
        with self.http.stream("GET",url,headers=headers) as response:
            if not response.is_success: response.read(); _checked(response)
            for line in response.iter_lines():
                if line: yield from decoder.frame(json.loads(line))
            yield from decoder.finish()
    def socket(self, path):
        token = self.token()
        url = token['url'].rstrip('/').replace('https:', 'wss:', 1).replace('http:', 'ws:', 1) + path
        return connect(url, subprotocols=['pryx', f'pryx-token.{token["token"]}'], max_queue=16, max_size=SDK_DEFAULTS['captureBytes'], proxy=None, legacy=True)
    def terminal(self, *, cols=80, rows=24):
        terminal = Terminal(self.socket('/_pryx/terminal'))
        terminal.resize(cols, rows)
        return terminal

class Stdin:
    def __init__(self, process): self._process = process
    def write(self, data): return self._process._agent.raw(self._process.path('/stdin'), 'POST', content=data.encode() if isinstance(data, str) else data)
    def close(self): return self._process._agent.request(self._process.path('/stdin'), 'DELETE')

class Process:
    def __init__(self, agent, view): self._agent, self.view, self.stdin = agent, view, Stdin(self)
    @property
    def id(self): return self.view['id']
    @property
    def pid(self): return self.view['pid']
    def path(self, suffix=''): return f'/_pryx/processes/{quote(self.id, safe="")}{suffix}'
    def wait_for_port(self,port,*,mode='tcp',path='/',status=200,timeout=SDK_DEFAULTS['runTimeoutSeconds']):
        from .duration import seconds
        return self._agent.request_v2(self.path('/ready'),'POST',{'kind':'port','port':port,'mode':mode,'path':path,'status':status,'timeout':seconds(timeout)})
    def wait_for_log(self,pattern,timeout=SDK_DEFAULTS['runTimeoutSeconds']):
        from .duration import seconds
        return self._agent.request_v2(self.path('/ready'),'POST',{'kind':'log','pattern':pattern,'timeout':seconds(timeout)})
    def wait_for_exit(self,timeout=SDK_DEFAULTS['runTimeoutSeconds']):
        from .duration import seconds
        from .generated.models import AgentResult
        return AgentResult.from_dict(self._agent.request_v2(self.path('/ready'),'POST',{'kind':'exit','timeout':seconds(timeout)}))
    def logs(self):return self.stream()
    def wait(self):
        from .generated.models import AgentResult
        return AgentResult.from_dict(self._agent.request(self.path('/wait')))
    def signal(self, signal): return self._agent.request(self.path('/signal'), 'POST', {'signal': signal})
    def kill(self): return self.signal('KILL')
    def resize(self, cols, rows): return self._agent.request(self.path('/resize'), 'POST', {'cols': cols, 'rows': rows})
    def events(self):
        url, headers = self._agent.target(self.path('/output'))
        with self._agent.http.stream('GET', url, headers=headers) as response:
            if not response.is_success: response.read(); _checked(response)
            for line in response.iter_lines():
                if line: yield json.loads(line)
    def run_events(self): yield from output(self.events())
    def stream(self):
        for event in self.run_events():
            if event['type'] in ('stdout', 'stderr'): yield {'stream': event['type'], 'data': event['data']}
    def lines(self):
        from .run import lines
        yield from lines(self.stream())

class Terminal:
    def __init__(self, socket): self.socket, self.stdin = socket, self
    def write(self, data): self.socket.send(json.dumps({'type': 'stdin', 'bytes': base64.b64encode(data.encode() if isinstance(data, str) else data).decode('ascii')}))
    def resize(self, cols, rows): self.socket.send(json.dumps({'type': 'resize', 'cols': cols, 'rows': rows}))
    def signal(self, signal): self.socket.send(json.dumps({'type': 'signal', 'signal': signal}))
    def close(self): self.socket.close()
    def events(self):
        for frame in self.socket: yield json.loads(frame)
    def stream(self):
        for event in output(self.events()):
            if event['type'] in ('stdout', 'stderr'): yield {'stream': event['type'], 'data': event['data']}
