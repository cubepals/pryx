# SPDX-License-Identifier: FSL-1.1-ALv2
"""Token reuse, project-key isolation, streamed data fallback and single-send mutations in both
Python transports. Real process and websocket behavior runs in the CI container acceptance suite.
"""
import base64
import json
import time
import unittest
from http import HTTPStatus
import httpx
from cubepals_pryx import Pryx, AsyncPryx, PryxError
from cubepals_pryx.generated.defaults import SDK_DEFAULTS

class Peer:
    def __init__(self): self.calls, self.data, self.state = [], b'', 'running'
    @property
    def view(self): return dict(id='one',project='default',state=self.state,node=None,epoch=1,endpoints=[],usage=None,issues=[],createdAt='',updatedAt='',expiresAt=None,agent={'url':'http://agent.test'},files=True)
    def handle(self, request):
        self.calls.append((request.url.host, request.url.path, request.headers.get('authorization'), request.headers.get('idempotency-key')))
        path = request.url.path
        if path.endswith('/token'): return httpx.Response(HTTPStatus.OK,json={'url':'http://agent.test','token':'sandbox-only','expiresAt':time.time()+SDK_DEFAULTS['units']['m'],'epoch':1,'publicKey':'public'})
        if path.endswith('/stop'): self.state='stopped'; return httpx.Response(HTTPStatus.OK,json={'sandbox':self.view})
        if path.endswith('/start'): self.state='running'; return httpx.Response(HTTPStatus.OK,json={'sandbox':self.view})
        if path.endswith('/content'):
            if request.method=='PUT': self.data=request.content; return httpx.Response(HTTPStatus.NO_CONTENT)
            return httpx.Response(HTTPStatus.OK,content=self.data)
        result=dict(code=0,timedOut=False,killed=False,durationMs=1,stdoutTruncated=False,stderrTruncated=False)
        if path.endswith('/processes'):
            self.command=json.loads(request.content)
            return httpx.Response(HTTPStatus.CREATED,json={'id':'process','pid':123,'running':True,'result':None})
        if path.endswith('/output'):
            events=[{'type':'output','stream':'stdout','bytes':base64.b64encode('héllo\n'.encode()).decode()},{'type':'exit','result':result}]
            return httpx.Response(HTTPStatus.OK,text=''.join(json.dumps(event)+'\n' for event in events))
        if path.endswith('/wait'): return httpx.Response(HTTPStatus.OK,json=result)
        if path.endswith('/ports'): return httpx.Response(HTTPStatus.CONFLICT,json={'error':{'code':'no_spare_port','message':'No spare port'}})
        return httpx.Response(HTTPStatus.OK,json=self.view)

class DirectSync(unittest.TestCase):
    def test_capabilities_streams_and_node_fallback(self):
        peer=Peer()
        with Pryx(url='http://control.test',key='project-secret',transport=httpx.MockTransport(peer.handle)) as sdk:
            box=sdk.sandbox('one')
            self.assertEqual(box.run('printf hello',cwd='/data',env={'CI':'1'}).stdout,'héllo\n')
            self.assertEqual(peer.command['command'],'printf hello')
            self.assertEqual(peer.command['cwd'],'/data')
            self.assertEqual(box.start('true').wait().code,0)
            box.write('large',iter([b'a'*50_000,b'b'*50_000]))
            self.assertEqual(b''.join(box.read_stream('large')),b'a'*50_000+b'b'*50_000)
            with self.assertRaises(PryxError) as error: box.expose(3000)
            self.assertEqual(error.exception.code,'no_spare_port')
            self.assertEqual(len([c for c in peer.calls if c[1].endswith('/ports')]),1)
            self.assertEqual(len([c for c in peer.calls if c[1].endswith('/token')]),1)
            self.assertTrue(all(c[2]=='Bearer sandbox-only' for c in peer.calls if c[0]=='agent.test'))
            self.assertTrue(all(c[3] for c in peer.calls if c[1].endswith('/processes')))
            box.stop();box.write('stopped',b'node-data')
            self.assertEqual(box.read_bytes('stopped'),b'node-data')
            self.assertEqual(peer.calls[-1][0],'control.test')
            box.start();box.run('true').wait()
            self.assertEqual(len([c for c in peer.calls if c[1].endswith('/token')]),2)

class DirectAsync(unittest.IsolatedAsyncioTestCase):
    async def test_capabilities_streams_and_node_fallback(self):
        peer=Peer()
        async with AsyncPryx('http://control.test','project-secret',transport=httpx.MockTransport(peer.handle)) as sdk:
            box=await sdk.sandbox('one')
            self.assertEqual((await box.run('printf hello',cwd='/data')).stdout,'héllo\n')
            self.assertEqual((await (await box.start('true')).wait()).code,0)
            async def chunks(): yield b'a'*50_000; yield b'b'*50_000
            await box.write('large',chunks())
            self.assertEqual(b''.join([chunk async for chunk in box.read_stream('large')]),b'a'*50_000+b'b'*50_000)
            with self.assertRaises(PryxError) as error: await box.expose(3000)
            self.assertEqual(error.exception.code,'no_spare_port')
            self.assertEqual(len([c for c in peer.calls if c[1].endswith('/ports')]),1)
            self.assertEqual(len([c for c in peer.calls if c[1].endswith('/token')]),1)
            self.assertTrue(all(c[2]=='Bearer sandbox-only' for c in peer.calls if c[0]=='agent.test'))
            await box.stop();await box.write('stopped',b'node-data')
            self.assertEqual(await box.read_bytes('stopped'),b'node-data')
            self.assertEqual(peer.calls[-1][0],'control.test')
            await box.start();await box.run('true')
            self.assertEqual(len([c for c in peer.calls if c[1].endswith('/token')]),2)
