# SPDX-License-Identifier: FSL-1.1-ALv2
"""Sync and async public handles against fake HTTP, including single-use runs and chunked files."""
import base64, json, unittest
from test_agent import Peer
from http import HTTPStatus
import httpx
from cubepals_pryx import Pryx, PryxError
from cubepals_pryx.duration import seconds
from cubepals_pryx.generated.defaults import SDK_DEFAULTS

VIEW = dict(id='sbx_one', project='one', state='running', node=None, epoch=None, endpoints=[], usage=None, issues=[], createdAt='', updatedAt='', expiresAt=None)
EVENTS = [dict(type='stdout',data='hi\n',truncated=False),dict(type='stderr',data='warning',truncated=False),dict(type='exit',code=0,timedOut=False,killed=False,durationMs=1)]
class Transport(Peer):
    def __init__(self):super().__init__();self.calls=[]
    def handle(self,request):
        path=request.url.path
        body=json.loads(request.content) if request.content and request.headers.get('content-type')=='application/json' else {}
        self.calls.append((request.method,path,body,request.headers.get('idempotency-key')))
        if path.endswith('/token'):return httpx.Response(HTTPStatus.OK,json={'url':'http://agent.test','token':'scoped','expiresAt':9999999999,'epoch':1,'publicKey':'public'})
        if path=='/_pryx/processes':return httpx.Response(HTTPStatus.CREATED,json={'id':'process','pid':123,'running':True,'result':None})
        if path.endswith('/output'):
            events=[{'type':'output','stream':e['type'],'bytes':base64.b64encode(e['data'].encode()).decode()} for e in EVENTS[:-1]]+[{'type':'exit','result':{k:v for k,v in EVENTS[-1].items() if k!='type'}}]
            return httpx.Response(HTTPStatus.OK,text=''.join(json.dumps(e)+'\n' for e in events))
        if path.endswith('/content'):
            if request.method=='PUT':self.data=request.content;return httpx.Response(HTTPStatus.NO_CONTENT)
            return httpx.Response(HTTPStatus.OK,content=self.data)
        if path.endswith('/snapshots'):return httpx.Response(HTTPStatus.CREATED,json={'id':'snap_one'})
        if path.endswith('/sleep'):return httpx.Response(HTTPStatus.OK,json={'sandbox':dict(VIEW,state='sleeping')})
        if path.endswith('/wake'):return httpx.Response(HTTPStatus.OK,json={'sandbox':VIEW})
        if request.method=='DELETE':return httpx.Response(HTTPStatus.OK,json={'existed':True})
        return httpx.Response(HTTPStatus.OK,json=VIEW)

class Handles(unittest.TestCase):
    def test_sync(self):
        transport = Transport()
        with Pryx(url='http://api.test',key='secret',transport=httpx.MockTransport(transport.handle)) as client:
            box = client.sandbox(image='node:22',idle='10m',ttl='2h')
            self.assertEqual(transport.calls[0][2]['idle'],10*SDK_DEFAULTS['units']['m'])
            self.assertEqual(transport.calls[0][2]['ttl'],2*SDK_DEFAULTS['units']['h'])
            self.assertTrue(transport.calls[0][3])
            self.assertFalse(hasattr(client,'api'))
            run = box.run('echo hi',timeout='30s',env={'CI':'1'})
            self.assertEqual(run.stdout,'hi\n'); self.assertEqual(run.code,0)
            self.assertEqual(run.stderr,'warning')
            self.assertEqual(len([call for call in transport.calls if call[1]=='/_pryx/processes']),1)
            self.assertEqual([c for c in transport.calls if c[1]=='/_pryx/processes'][0][2]['command'],'echo hi')
            box.sleep(); self.assertEqual(box.state,'sleeping')
            box.wake(); self.assertEqual(box.state,'running')
            snap = box.snapshot(); twin = snap.fork(idle='5m')
            self.assertEqual(twin.id,box.id); self.assertTrue(transport.calls[-1][3])
            data='é'*70_000
            box.write('src/app.py',data); self.assertEqual(box.read('src/app.py'),data)
            box.write('src/app.py',b''); self.assertEqual(box.read_bytes('src/app.py'),b'')
            self.assertEqual(list(box.stream('echo hi').lines()),['hi','warning'])
    def test_unknown_and_durations(self):
        transport=Transport()
        original=transport.handle
        def handle(request):
            if request.url.path.endswith('/output'):return httpx.Response(HTTPStatus.OK,text='')
            return original(request)
        with Pryx(url='http://api.test',key='secret',transport=httpx.MockTransport(handle)) as client:
            with self.assertRaises(PryxError) as error:client.sandbox('sbx_one').run('true')
            self.assertEqual(error.exception.code,'exec_interrupted')
            self.assertEqual(len([c for c in transport.calls if c[1]=='/_pryx/processes']),1)
        for invalid in ('bad','0s','1ms',True,float('inf')):
            with self.assertRaises(ValueError): seconds(invalid)

class AsyncHandles(unittest.IsolatedAsyncioTestCase):
    async def test_async(self):
        transport = Transport()
        async with Pryx(url='http://api.test',key='secret',transport=httpx.MockTransport(transport.handle)) as client:
            box = await client.sandbox(image='alpine',idle='5m')
            run = box.stream(['echo','hi'])
            self.assertEqual([line async for line in run.lines()],['hi','warning'])
            self.assertEqual((await run.result()).stdout,'hi\n')
            self.assertEqual(len([call for call in transport.calls if call[1]=='/_pryx/processes']),1)
            await box.write('file',b'x'*100_000)
            self.assertEqual(await box.read_bytes('file'),b'x'*100_000)
            twin = await (await box.snapshot()).fork(idle='5m')
            self.assertEqual(twin.id,box.id)
            await box.sleep(); self.assertEqual(box.state,'sleeping')
            await box.wake(); self.assertEqual(box.state,'running')
            await box.delete()
