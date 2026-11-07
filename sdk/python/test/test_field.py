# SPDX-License-Identifier: FSL-1.1-ALv2
"""Both context modes exercise field conveniences over the same strict fake HTTP peer."""
import base64,inspect,json,tempfile,unittest
from http import HTTPStatus
from pathlib import Path
import httpx
from cubepals_pryx import Pryx,PryxError
from test_handles import VIEW
from cubepals_pryx.generated.defaults import SDK_DEFAULTS
RESULT=dict(code=0,timedOut=False,killed=False,durationMs=1,stdoutTruncated=False,stderrTruncated=False)
async def take(value):return await value if inspect.isawaitable(value) else value
async def items(value):
    if hasattr(value,'__aiter__'):return [item async for item in value]
    return list(value)
class Peer:
    def __init__(self):self.calls=[];self.state='sleeping';self.command=[];self.files={};self.archives={};self.failure=None;self.version=2;self.isolation=None
    def view(self):return dict(VIEW,state=self.state,name='named',labels={'role':'agent'},workdir='/data',**({'isolation':self.isolation} if self.isolation else {}))
    def handle(self,request):
        path=request.url.path;method=request.method;body=json.loads(request.content) if request.content and request.headers.get('content-type')=='application/json' else {}
        self.calls.append((path,method,body,request.content))
        if path.startswith('/_pryx/'):assert request.headers['authorization']=='Bearer scoped'
        def response(value,status=HTTPStatus.OK):return httpx.Response(status,json=value)
        if path.endswith('/token'):return response(dict(url='http://agent.test',token='scoped',expiresAt=9999999999,epoch=1))
        if path=='/_pryx/health':return response(dict(status='ok',apiVersion=self.version))
        if path=='/v1/project':return response({'defaults':{'isolation':'wasm','repoImage':'unused'}})
        if path=='/v1/sandboxes' and method=='GET':return response({'sandboxes':[self.view()]})
        if path.endswith('/wake'):self.state='running';return response({'sandbox':self.view()})
        if path.endswith('/stop'):self.state='stopped';return response({'sandbox':self.view()})
        if path=='/_pryx/processes':
            if method=='GET':return response({'processes':[dict(id='proc',pid=1,running=True,result=None)]})
            self.command=body['command'];return response(dict(id='proc',pid=1,running=True,result=None))
        if path.endswith('/ready'):return response({'error':{'code':self.failure,'message':self.failure}},HTTPStatus.CONFLICT) if self.failure else response(RESULT if body['kind']=='exit' else dict(ready=True,matched=True))
        if path.endswith('/kill-all'):return response({'killed':['proc']})
        if path=='/_pryx/shells':return response({'id':'shell'})
        if path.endswith('/shells/shell/run'):return response(dict(RESULT,stdout='state',stderr='err',code=7))
        if path.endswith('/wait'):return response(RESULT)
        if path.endswith('/output'):
            stdout='PRYX_GIT_PROXY_READY 34567\n' if any('PRYX_GIT_PROXY_READY' in arg for arg in self.command) else 'https://example.com/repo.git\n' if 'get-url' in self.command else 'abc\n' if 'rev-parse' in self.command else 'abc\0Name\0email\0date\0subject\0\n' if 'log' in self.command else ''
            events=[dict(type='output',stream='stdout',bytes=base64.b64encode(stdout.encode()).decode()),dict(type='exit',result=RESULT)]
            return httpx.Response(HTTPStatus.OK,text=''.join(json.dumps(event)+'\n' for event in events))
        if path.endswith('/find'):return response({'paths':['known.txt']})
        if path.endswith('/grep'):return response({'matches':[dict(path='known.txt',line=2,text='known')]})
        if path.endswith('/replace'):return response({'replacements':1})
        if path.endswith('/stat'):
            name=request.url.params['path']
            return response(dict(kind='file',size=len(self.files[name]))) if name in self.files else response({'error':{'code':'not_found'}},HTTPStatus.NOT_FOUND)
        if path.endswith('/content'):
            name=request.url.params['path']
            if method=='PUT':self.files[name]=request.content;return httpx.Response(HTTPStatus.NO_CONTENT)
            return httpx.Response(HTTPStatus.OK,content=self.files[name]) if name in self.files else response({'error':{'code':'not_found'}},HTTPStatus.NOT_FOUND)
        if path.endswith('/archive'):
            name=request.url.params['path']
            if method=='PUT':self.archives[name]=request.content;return httpx.Response(HTTPStatus.NO_CONTENT)
            return httpx.Response(HTTPStatus.OK,content=self.archives[name])
        if path.endswith('/snapshots'):return response({'id':'00000000-0000-4000-8000-000000000001'})
        if '/templates/' in path and path.endswith('/versions'):return response(dict(id='template',name='named',version=1,snapshot=body['snapshot']))
        if path=='/v1/templates':return response({'templates':[]})
        if path=='/v1/webhooks':return response(dict(id='hook',secret='shown-once',**body)) if method=='POST' else response({'webhooks':[]})
        if path.endswith('/deadletters'):return response({'deliveries':[]})
        if path.endswith('/events'):return httpx.Response(HTTPStatus.OK,text=json.dumps(dict(id=1,kind='sandbox.created',at='now',data={}))+'\n')
        if path.endswith('/metrics'):return response(dict(at='now',metrics={'memoryBytes':12}))
        if method=='DELETE':return response({'existed':True})
        return response(self.view())
class Field(unittest.IsolatedAsyncioTestCase):
    async def both(self,exercise):
        peer=Peer()
        with Pryx(url='http://api.test',key='project-secret',transport=httpx.MockTransport(peer.handle)) as client:await exercise(client,peer)
        peer=Peer()
        async with Pryx(url='http://api.test',key='project-secret',transport=httpx.MockTransport(peer.handle)) as client:await exercise(client,peer)
    async def test_processes(self):
        async def exercise(client,peer):
            box=await take(client.sandbox('box'));process=await take(box.spawn('cat',stdin=b'\0\xff'))
            await take(process.wait_for_port(3000,mode='http',path='/ready',timeout='2s'));await take(process.wait_for_log('ready','2s'));self.assertEqual((await take(process.wait_for_exit())).code,0)
            self.assertEqual((await take(box.processes()))[0].id,'proc');self.assertEqual(await take(box.kill_all()),{'killed':['proc']})
            self.assertEqual(len([c for c in peer.calls if c[0]=='/_pryx/processes' and c[1]=='POST']),1)
            self.assertEqual([c[1] for c in peer.calls if c[0].endswith('/stdin')],['POST','DELETE'])
            for code in ['process_exited','wait_timeout']:
                peer.failure=code
                with self.assertRaises(PryxError) as error:await take(process.wait_for_port(1))
                self.assertEqual(error.exception.code,code)
        await self.both(exercise)
    async def test_shell_and_version_gate(self):
        async def exercise(client,peer):
            box=await take(client.sandbox('box'));sh=await take(box.shell(env={'X':'1'}));result=await take(sh.run('pwd'))
            self.assertEqual((result.stdout,result.stderr,result.code),('state','err',7));await take(sh.close())
            box=await take(client.sandbox('box'));peer.version=1
            with self.assertRaises(PryxError) as error:await take(box.shell())
            self.assertEqual(error.exception.code,'unsupported')
        await self.both(exercise)
    async def test_file_search_replace_modes_and_disk_transfers(self):
        async def exercise(client,peer):
            box=await take(client.sandbox('box'));self.assertFalse(await take(box.files.exists('missing')))
            self.assertEqual(await take(box.files.find('**/*.txt')),['known.txt']);self.assertEqual((await take(box.files.grep('known',paths=['known.txt'])))[0]['line'],2)
            self.assertEqual(await take(box.files.replace('known','new',paths=['known.txt'])),{'replacements':1});await take(box.files.chmod('known.txt',0o600))
            with tempfile.TemporaryDirectory() as root:
                local=Path(root)/'source';local.write_bytes(b'binary\0content');progress=[]
                await take(box.files.upload(local,'known.txt',on_progress=lambda n,total:progress.append(n)));await take(box.files.download('known.txt',Path(root)/'out'))
                self.assertEqual((Path(root)/'out').read_bytes(),local.read_bytes());self.assertEqual(progress[-1],14)
        await self.both(exercise)
    async def test_archives_and_content_sync(self):
        async def exercise(client,peer):
            box=await take(client.sandbox('box'))
            with tempfile.TemporaryDirectory() as root:
                source=Path(root)/'source';source.mkdir();(source/'known').write_text('content');(source/'empty').mkdir()
                await take(box.files.upload_dir(source,'tree'));await take(box.files.download_dir('tree',Path(root)/'out'));self.assertEqual((Path(root)/'out/known').read_text(),'content')
                first=await take(box.sync(source,'sync'));self.assertEqual(first['uploaded'],['known']);second=await take(box.sync(source,'sync'));self.assertEqual(second['unchanged'],['known']);self.assertEqual(second['uploaded'],[])
        await self.both(exercise)
    async def test_git_and_creation_credentials(self):
        async def exercise(client,peer):
            box=await take(client.sandbox(repo='https://github.com/cubepals/pryx',isolation='wasm',token='git-secret'))
            self.assertTrue((await take(box.git.status()))['value']['clean']);await take(box.git.diff(staged=True));await take(box.git.branch('topic'));await take(box.git.checkout('topic',create=True));await take(box.git.add('known.txt'))
            self.assertEqual((await take(box.git.commit('message',author={'name':'Name','email':'email'})))['value']['hash'],'abc');await take(box.git.pull());self.assertEqual((await take(box.git.log(limit=1)))['value'][0]['subject'],'subject')
            create=next(c for c in peer.calls if c[0]=='/v1/sandboxes' and c[1]=='POST');self.assertNotIn('git-secret',json.dumps(create[2]));self.assertIn('git',create[2]['packages'])
            clone=next(c[2] for c in peer.calls if c[0]=='/_pryx/processes' and 'clone' in c[2].get('command',[]));self.assertNotIn('git-secret',json.dumps(clone['command']));self.assertEqual(clone['env']['PRYX_GIT_TOKEN'],'git-secret');self.assertEqual(clone['env']['PRYX_GIT_HOST'],'github.com')
            with self.assertRaises(ValueError):await take(box.git.clone('https://secret@github.com/repo'))
        await self.both(exercise)
    async def test_name_filters_create_options_and_wake(self):
        async def exercise(client,peer):
            inspected=await take(client.sandbox(name='named',wake=False));self.assertEqual(inspected.state,'sleeping');box=await take(client.sandbox('box'));self.assertEqual(box.state,'running')
            await take(client.sandboxes(name='named',state='running',labels={'role':'agent'}));await take(client.sandbox(isolation='container',image='owned',name='new',env={'X':'1'},labels={'role':'agent'},metadata={'a':[1]},workdir='/data',user='1000',autoDelete=True,idle='2m',egress={'allow':['example.com'],'deny':[]}))
            create=[c[2] for c in peer.calls if c[0]=='/v1/sandboxes' and c[1]=='POST'][-1];self.assertEqual(create['metadata'],{'a':[1]});self.assertEqual(create['idle'],120);self.assertEqual(create['egress']['allow'],['example.com'])
        await self.both(exercise)
    async def test_lifetimes_metrics_and_event_iterators(self):
        async def exercise(client,peer):
            box=await take(client.sandbox('box'));await take(box.keep_alive('2m'));await take(box.set_ttl('3m'));await take(box.set_idle('1m'));await take(box.wait_until('running'));self.assertEqual((await take(box.metrics()))['metrics']['memoryBytes'],12);self.assertEqual((await items(box.events()))[0]['kind'],'sandbox.created')
            self.assertEqual([c[2] for c in peer.calls if c[0].endswith('/lifetime')],[dict(keepAlive=120),dict(ttl=180),dict(idle=60)])
        await self.both(exercise)
    async def test_code_template_build_and_rebuild(self):
        async def exercise(client,peer):
            template=await take(client.template(name='named',isolation='wasm',run=['true'],ready='true',start='sleep 60'));self.assertEqual(template.version,1);self.assertEqual((await take(template.rebuild())).name,'named');self.assertEqual(await take(client.templates()),[])
            self.assertTrue(any(c[0].endswith('/versions') and c[2]['start']=='sleep 60' for c in peer.calls))
        await self.both(exercise)
    async def test_webhook_registration_does_not_list_its_secret(self):
        async def exercise(client,peer):
            hook=await take(client.webhook(url='https://example.com/hook',kinds=['created','deleted']));self.assertEqual(hook.secret,'shown-once');self.assertNotIn('secret',hook.view);self.assertEqual(await take(hook.deadletters()),[]);await take(hook.delete());self.assertEqual(await take(client.webhooks()),[])
        await self.both(exercise)


    async def test_virtual_git_uses_bridge_for_push_and_guest_metadata(self):
        async def exercise(client,peer):
            peer.isolation='wasm';box=await take(client.sandbox('box'))
            await take(box.git.clone('https://example.com/repo.git',token='fixture-only',timeout='2m'));await take(box.git.push(token='fixture-only',timeout='2m'))
            starts=[c[2] for c in peer.calls if c[0]=='/_pryx/processes' and c[1]=='POST']
            self.assertEqual(len([c for c in starts if any('PRYX_GIT_PROXY_READY' in arg for arg in c['command'])]),2)
            self.assertTrue(all(c['timeout']==2*SDK_DEFAULTS['units']['m'] for c in starts))
            clone=next(c for c in starts if 'clone' in c['command']);self.assertTrue(any(arg.startswith('--separate-git-dir=/tmp/.pryx-git-') for arg in clone['command']))
            push=next(c for c in starts if 'push' in c['command']);self.assertTrue(any('insteadOf=https://example.com/' in arg for arg in push['command']));self.assertNotIn('fixture-only',str(push['command']))
        await self.both(exercise)

class GitTraceSecurity(unittest.TestCase):
    def test_trace_settings_cannot_persist_a_token_in_both_modes(self):
        import asyncio,os,socket,subprocess
        from cubepals_pryx.git import Git,AsyncGit
        from cubepals_pryx.run import RunResult
        for asynchronous in (False,True):
            with tempfile.TemporaryDirectory() as root,socket.socket() as listener:
                listener.bind(('127.0.0.1',0));port=listener.getsockname()[1]
                # Bound without listening: the owned endpoint refuses before any TLS/credential exchange.
                trace=Path(root)/'trace.json'
                def run(command,*,env,**_options):
                    child=subprocess.run(command,cwd=root,env={**os.environ,**env,'NO_PROXY':'127.0.0.1','no_proxy':'127.0.0.1','GIT_TRACE2_EVENT':str(trace),'GIT_TRACE2_ENV_VARS':'PRYX_GIT_TOKEN'},capture_output=True,text=True,check=False,timeout=10)
                    return RunResult(stdout=child.stdout,stderr=child.stderr,code=child.returncode)
                async def async_run(command,*,env,**_options):return await asyncio.to_thread(run,command,env=env)
                git=AsyncGit(async_run) if asynchronous else Git(run)
                with self.assertRaises(PryxError):
                    result=git.clone('https://127.0.0.1:'+str(port)+'/fixture.git',token='one-call-fixture-token')
                    if asynchronous:asyncio.run(result)
                subprocess.run(['git','init',str(Path(root)/'repo')],capture_output=True,check=True)
                subprocess.run(['git','-C',str(Path(root)/'repo'),'remote','add','origin','https://127.0.0.1:'+str(port)+'/fixture.git'],capture_output=True,check=True)
                with self.assertRaises(PryxError):
                    result=git.push(token='one-call-fixture-token')
                    if asynchronous:asyncio.run(result)
                self.assertFalse(trace.exists())
