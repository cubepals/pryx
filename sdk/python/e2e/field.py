# SPDX-License-Identifier: FSL-1.1-ALv2
"""Both Python context modes traverse the existing real field sandbox through public APIs."""
import asyncio,inspect,os
from pathlib import Path
from cubepals_pryx import Pryx
from agent import web_command
from urllib.parse import urlparse
import httpx
async def take(value):return await value if inspect.isawaitable(value) else value
async def exercise(client,suffix):
    box=await take(client.sandbox(os.environ['PRYX_FIELD_SANDBOX']))
    assert (await take(box.run('cat',stdin='python-input'))).stdout=='python-input'
    await take(box.write('index.html','python-web'))
    web=await take(box.spawn(web_command()));await take(web.wait_for_port(3000,mode='http'))
    url=await take(box.expose(3000))
    async with httpx.AsyncClient() as http:response=await http.get(os.environ['PRYX_TEST_EDGE_URL'],headers={'host':urlparse(url).hostname})
    assert response.status_code==200 and response.text=='python-web'
    await take(web.kill());await take(web.wait_for_exit())
    sh=await take(box.shell())
    try:
        await take(sh.run('mkdir -p python-state; cd python-state; export PY_STATE=kept'))
        result=await take(sh.run('printf "$PY_STATE"; pwd; printf error >&2; false'))
        assert result.stdout.startswith('kept') and result.stdout.endswith('/python-state\n') and result.stderr=='error' and result.code==1,result
    finally:await take(sh.close())
    process=await take(box.spawn('printf python-log; sleep 600'));await take(process.wait_for_log('python-log'));await take(process.kill());await take(process.wait_for_exit())
    await take(box.write('python-known','first\nneedle\n'));assert await take(box.files.exists('python-known'));assert (await take(box.files.grep('needle',paths=['python-known'])))[0]['line']==2
    assert 'python-known' in await take(box.files.find('python-*'));await take(box.files.replace('needle','changed',paths=['python-known']));await take(box.files.chmod('python-known',0o600))
    root=Path(os.environ['PRYX_FIELD_LOCAL']);local=root/'source';await take(box.files.upload(local/'local.txt','python-upload'));await take(box.files.download('python-upload',root/f'python-{suffix}'));assert (root/f'python-{suffix}').read_text()=='local content'
    await take(box.files.upload_dir(local,'python-tree'));await take(box.files.download_dir('python-tree',root/f'download-{suffix}'))
    # Keep generated download trees outside the next sync source to avoid self-copy recursion.
    await take(box.sync(local,'python-sync'))
    path='python-repo-'+suffix;branch='python-'+suffix
    await take(box.git.clone('https://github.com/octocat/Hello-World.git',path=path,depth=1));assert (await take(box.git.status()))['value']['clean']
    await take(box.git.branch(branch));await take(box.git.checkout(branch));await take(box.write(path+'/python.txt','committed'));await take(box.git.add('python.txt'));assert (await take(box.git.diff(staged=True)))['value']['patch']
    await take(box.git.commit('Python field commit',author={'name':'Field Test','email':'field@example.test'}));assert (await take(box.git.log(limit=1)))['value'][0]['subject']=='Python field commit'
    for args in [['remote','set-url','origin',os.environ['PRYX_FIELD_GIT_REMOTE']],['config','push.default','current'],['config','remote.origin.fetch','+refs/heads/*:refs/remotes/origin/*'],['config','branch.'+branch+'.remote','origin'],['config','branch.'+branch+'.merge','refs/heads/'+branch]]:assert (await take(box.run(['git','-C',path,*args]))).code==0
    await take(box.git.push());await take(box.git.pull())
    await take(box.keep_alive('10m'));await take(box.set_ttl('10m'));await take(box.set_idle('10m'));await take(box.wait_until('running'));assert (await take(box.metrics()))['metrics']
    events=box.events()
    if hasattr(events,'__aiter__'):assert [event async for event in events]
    else:assert list(events)
    assert (await take(client.sandboxes(name='field-owned',labels={'suite':'field'})))[0].id==box.id
    template=await take(client.template(name='python-code-'+suffix,isolation=box.isolation,**({'from_':'alpine:pryx-tls'} if box.isolation=='container' else {}),copy=[(str(local),'context')],run=['printf prepared > prepared'],ready='test -s prepared',start='printf python-started > python-start; sleep 600'))
    rebuilt=await take(template.rebuild());assert rebuilt.version==template.version+1
    child=await take(client.sandbox(template=template.name))
    try:
        for attempt in range(100):
            try:
                if await take(child.read('python-start'))=='python-started':break
            except Exception as error:
                if getattr(error,'status',None)!=404:raise
            await asyncio.sleep(.1)
        assert await take(child.read('context/local.txt'))=='local content'
        assert await take(child.read('python-start'))=='python-started'
    finally:await take(child.delete())
    hook=await take(client.webhook(url=os.environ['PRYX_FIELD_RECEIVER'],kinds=['created','deleted']))
    assert hook.secret and await take(client.webhooks());assert not await take(hook.deadletters());await take(hook.delete())
    await take(box.kill_all())
with Pryx(url=os.environ['PRYX_API_URL'],key=os.environ['PRYX_API_KEY']) as client:asyncio.run(exercise(client,'sync'))
async def main():
    async with Pryx(url=os.environ['PRYX_API_URL'],key=os.environ['PRYX_API_KEY']) as client:await exercise(client,'async')
asyncio.run(main())
