# SPDX-License-Identifier: FSL-1.1-ALv2
"""Sync and async public handles against fake HTTP, including single-use runs and chunked files."""
import base64, io, json, tarfile, unittest
from http import HTTPStatus
import httpx
from cubepals_pryx import Pryx, AsyncPryx, PryxError
from cubepals_pryx.duration import seconds
from cubepals_pryx.generated.defaults import SDK_DEFAULTS

VIEW = dict(id='sbx_one', project='one', state='running', node=None, epoch=None, endpoints=[], usage=None, issues=[], createdAt='', updatedAt='', expiresAt=None)
EVENTS = [dict(type='stdout',data='hi\n',truncated=False),dict(type='stderr',data='warning',truncated=False),dict(type='exit',code=0,timedOut=False,killed=False,durationMs=1)]
class Transport:
    def __init__(self): self.calls, self.data = [], b''
    def handle(self, request):
        body = json.loads(request.content) if request.content else {}
        self.calls.append((request.method, request.url.path, body, request.headers.get('idempotency-key')))
        if request.url.path.endswith('/exec'): return httpx.Response(HTTPStatus.OK, text=''.join(json.dumps(event)+'\n' for event in EVENTS))
        if '/file-chunks/' in request.url.path:
            if request.method == 'PUT':
                offset = body['offset']
                chunk = base64.b64decode(body['dataBase64'])
                self.data = (self.data[:offset] if offset else b'') + chunk
                return httpx.Response(HTTPStatus.OK,json={'written':True})
            offset = int(request.url.params['offset'])
            data = self.data[offset:offset+SDK_DEFAULTS['fileChunkBytes']]
            stream = io.BytesIO()
            with tarfile.open(fileobj=stream, mode='w', format=tarfile.USTAR_FORMAT) as archive:
                entry = tarfile.TarInfo('data'); entry.size = len(data)
                archive.addfile(entry, io.BytesIO(data))
            return httpx.Response(HTTPStatus.OK,json={'archiveBase64':base64.b64encode(stream.getvalue()).decode()})
        if request.url.path.endswith('/snapshots'): return httpx.Response(HTTPStatus.CREATED,json={'id':'snap_one'})
        if request.url.path.endswith('/sleep'): return httpx.Response(HTTPStatus.OK,json={'sandbox':dict(VIEW,state='sleeping')})
        if request.url.path.endswith('/wake'): return httpx.Response(HTTPStatus.OK,json={'sandbox':VIEW})
        if request.method == 'DELETE': return httpx.Response(HTTPStatus.OK,json={'existed':True})
        return httpx.Response(HTTPStatus.CREATED if request.method == 'POST' else HTTPStatus.OK,json=VIEW)

class Handles(unittest.TestCase):
    def test_sync(self):
        transport = Transport()
        with Pryx(url='http://api.test',key='secret',transport=httpx.MockTransport(transport.handle)) as client:
            box = client.sandbox(image='node:22',idle='10m',ttl='2h')
            self.assertEqual(transport.calls[0][2]['idle'],10*SDK_DEFAULTS['units']['m'])
            self.assertEqual(transport.calls[0][2]['ttl'],2*SDK_DEFAULTS['units']['h'])
            self.assertTrue(transport.calls[0][3])
            self.assertIs(client.api,client)
            run = box.run('echo hi',timeout='30s',env={'CI':'1'})
            self.assertEqual(run.stdout,'hi\n'); self.assertEqual(run.code,0)
            self.assertEqual(run.wait().stderr,'warning')
            self.assertEqual(len([call for call in transport.calls if call[1].endswith('/exec')]),1)
            self.assertEqual(transport.calls[-1][2]['command'],['sh','-c','echo hi'])
            box.sleep(); self.assertEqual(box.state,'sleeping')
            box.wake(); self.assertEqual(box.state,'running')
            snap = box.snapshot(); twin = snap.fork(idle='5m')
            self.assertEqual(twin.id,box.id); self.assertTrue(transport.calls[-1][3])
            data='é'*70_000
            box.write('src/app.py',data); self.assertEqual(box.read('src/app.py'),data)
            box.write('src/app.py',b''); self.assertEqual(box.read_bytes('src/app.py'),b'')
            self.assertEqual(list(box.run('echo hi').lines()),['hi','warning'])
    def test_unknown_and_durations(self):
        calls=[]
        def handle(request):
            calls.append(request)
            return httpx.Response(HTTPStatus.OK,text='' if request.url.path.endswith('/exec') else json.dumps(VIEW))
        with Pryx(url='http://api.test',key='secret',transport=httpx.MockTransport(handle)) as client:
            with self.assertRaises(PryxError) as error: client.sandbox('sbx_one').run('true').wait()
            self.assertEqual(error.exception.code,'exec_interrupted')
            self.assertEqual(len(calls),2)
        for invalid in ('bad','0s','1ms',True,float('inf')):
            with self.assertRaises(ValueError): seconds(invalid)

class AsyncHandles(unittest.IsolatedAsyncioTestCase):
    async def test_async(self):
        transport = Transport()
        async with AsyncPryx('http://api.test','secret',transport=httpx.MockTransport(transport.handle)) as client:
            box = await client.sandbox(image='alpine',idle='5m')
            run = box.run(['echo','hi'])
            self.assertEqual([line async for line in run.lines()],['hi','warning'])
            self.assertEqual((await run).stdout,'hi\n')
            self.assertEqual(len([call for call in transport.calls if call[1].endswith('/exec')]),1)
            await box.write('file',b'x'*100_000)
            self.assertEqual(await box.read_bytes('file'),b'x'*100_000)
            twin = await (await box.snapshot()).fork(idle='5m')
            self.assertEqual(twin.id,box.id)
            await box.sleep(); self.assertEqual(box.state,'sleeping')
            await box.wake(); self.assertEqual(box.state,'running')
            await box.delete()
