# SPDX-License-Identifier: FSL-1.1-ALv2
"""Sync and async native handles preserve PNG bytes and keep private keys away from control."""
import asyncio,json,time,unittest
from http import HTTPStatus
import httpx
from cubepals_pryx import Pryx
from cubepals_pryx.generated.tools import TOOL_DEFAULTS
PNG=b'\x89PNG\r\n\x1a\n'
class Peer:
    def __init__(self):self.calls=[]
    def handle(self,request):
        path=request.url.path;self.calls.append((path,request.headers.get('authorization'),request.content))
        if path.endswith('/token'):value={'url':'http://agent.test','token':'scoped','expiresAt':time.time()+300,'epoch':1}
        elif path.endswith('/health'):value={'toolsVersion':TOOL_DEFAULTS['version']}
        elif path.endswith('/screenshot'):return httpx.Response(HTTPStatus.OK,content=PNG,headers={'content-type':'image/png'})
        elif path=='/_pryx/ssh':value={'id':'a','key':'PRIVATE-ONCE','hostKey':'ssh-ed25519 public','port':1234,'slot':2,'user':'pryx','expiresAt':123,'running':True}
        elif path=='/v1/sandboxes/one/ssh':value={'revoked':True} if request.method=='DELETE' else {'id':'a','host':'one.test','port':22000,'user':'pryx','hostKey':'ssh-ed25519 public','expiresAt':123}
        elif path=='/_pryx/mounts':value={'mounts':[{'bucket':'s3://test','path':'/mnt/data','state':'mounted','backend':'rclone','readOnly':False}]}
        elif path.startswith('/_pryx/'):value={'url':'http://preview.test','expiresAt':123}
        else:value={'id':'one','project':'default','state':'running','node':None,'epoch':1,'agent':{'url':'http://agent.test'},'files':True,'endpoints':[],'usage':None,'issues':[],'createdAt':'','updatedAt':'','expiresAt':None}
        return httpx.Response(HTTPStatus.OK,json=value)
class Tools(unittest.TestCase):
    def test_sync_and_async_native_handles(self):
        async def exercise(client,peer):
            async def take(value):return await value if hasattr(value,'__await__') else value
            box=await take(client.sandbox('one'))
            self.assertEqual(await take(box.desktop.screenshot()),PNG)
            await take(box.desktop.click(20,30));await take(box.desktop.type('text'));await take(box.desktop.key('Return'));await take(box.desktop.move(4,5));await take(box.desktop.scroll(0,1))
            self.assertEqual(await take(box.browser.screenshot('https://page.test')),PNG)
            access=await take(box.ssh());self.assertEqual(access.key,'PRIVATE-ONCE');self.assertNotIn('PRIVATE-ONCE',repr(access));self.assertNotIn('PRIVATE-ONCE',json.dumps(access.to_dict()))
            self.assertEqual([json.loads(body) for path,_,body in peer.calls if path=='/v1/sandboxes/one/ssh'],[{'id':'a'}])
            self.assertEqual((await take(box.mounts()))[0]['bucket'],'s3://test')
            await take(box.ssh.revoke())
            self.assertTrue(all(auth=='Bearer scoped' for path,auth,_ in peer.calls if path.startswith('/_pryx/')))
        peer=Peer()
        with Pryx(url='http://control.test',key='project',transport=httpx.MockTransport(peer.handle)) as client:asyncio.run(exercise(client,peer))
        async def main():
            peer=Peer()
            async with Pryx(url='http://control.test',key='project',transport=httpx.MockTransport(peer.handle)) as client:await exercise(client,peer)
        asyncio.run(main())
