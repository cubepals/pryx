# SPDX-License-Identifier: FSL-1.1-ALv2
"""Both context modes preserve all MIME forms and never retry interrupted snippets."""
import asyncio, base64, json, time, unittest
from http import HTTPStatus
import httpx
from cubepals_pryx import Pryx, PryxError
from cubepals_pryx.generated.kernels import KERNEL_DEFAULTS

class Peer:
    def __init__(self, events): self.events, self.calls, self.bodies = events, [], []
    def handle(self, request):
        path=request.url.path; self.calls.append((path,request.headers.get('authorization')))
        if path.endswith('/execute'): self.bodies.append(json.loads(request.content))
        if path.endswith('/token'): value={'url':'http://agent.test','token':'scoped','expiresAt':time.time()+300,'epoch':1}
        elif path.endswith('/health'): value={'kernelsVersion':KERNEL_DEFAULTS['version']}
        elif path.endswith('/execute'): return httpx.Response(HTTPStatus.OK,text=''.join(json.dumps(e)+'\n' for e in self.events))
        elif path=='/_pryx/kernels': value={'id':'kernel','pid':12,'processId':'process','context':'','language':'python','state':'idle'}
        else: value={'id':'one','project':'default','state':'running','node':None,'epoch':1,'agent':{'url':'http://agent.test'},'files':True,'endpoints':[],'usage':None,'issues':[],'createdAt':'','updatedAt':'','expiresAt':None}
        return httpx.Response(HTTPStatus.OK,json=value)

class Kernels(unittest.TestCase):
    def test_both_context_modes_and_unknown_outcome(self):
        async def exercise(client, peer):
            async def take(value):
                return await value if hasattr(value,'__await__') else value
            box=await take(client.sandbox('one'));py=await take(box.python())
            result=await take(py.run('4'))
            self.assertEqual(result.json,4);self.assertEqual(result.text,'4');self.assertEqual(result.png,b'\x89PNG\r\n\x1a\n');self.assertEqual(result.stdout,'hello');self.assertEqual(result.html,'<b>four</b>')
            peer.events=[{'type':'error','name':'ValueError','value':'bad','traceback':['frame']},{'type':'done','executionCount':2,'status':'error'}]
            self.assertEqual((await take(py.run('bad',timeout='5s'))).error['traceback'],['frame'])
            # The agent picks the timeout within the token's ceiling unless the caller names one.
            self.assertEqual(peer.bodies[:2],[{'code':'4'},{'code':'bad','timeout':5}])
            peer.events=[]
            with self.assertRaises(PryxError):await take(py.run('mutate()'))
            self.assertEqual(len([call for call in peer.calls if call[0].endswith('/execute')]),3)
            self.assertTrue(all(auth=='Bearer scoped' for path,auth in peer.calls if path.startswith('/_pryx/')))
        events=[{'type':'stream','stream':'stdout','text':'hello'},{'type':'display','data':{'image/png':base64.b64encode(b'\x89PNG\r\n\x1a\n').decode(),'text/html':'<b>four</b>'},'metadata':{},'displayId':None,'update':False},{'type':'result','data':{'text/plain':'4','application/json':4},'metadata':{},'executionCount':1},{'type':'done','executionCount':1,'status':'ok'}]
        peer=Peer(events)
        with Pryx(url='http://control.test',key='project',transport=httpx.MockTransport(peer.handle)) as client:asyncio.run(exercise(client,peer))
        async def main():
            peer=Peer(events)
            async with Pryx(url='http://control.test',key='project',transport=httpx.MockTransport(peer.handle)) as client:await exercise(client,peer)
        asyncio.run(main())
