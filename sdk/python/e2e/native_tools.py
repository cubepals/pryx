# SPDX-License-Identifier: FSL-1.1-ALv2
"""Use the real native tool routes from both Python context modes."""
import asyncio,inspect,os
from cubepals_pryx import Pryx
PNG=b'\x89PNG\r\n\x1a\n'
async def exercise(client,mode):
    async def take(value):return await value if inspect.isawaitable(value) else value
    box=await take(client.sandbox(os.environ['PRYX_TOOLS_SANDBOX']))
    kind=os.environ['PRYX_TOOLS_KIND']
    if kind=='desktop':
        assert (await take(box.desktop.screenshot())).startswith(PNG)
        await take(box.desktop.click(20,20));await take(box.desktop.type('python'));await take(box.desktop.key('Return'));await take(box.desktop.move(30,30));await take(box.desktop.scroll(0,1))
        assert (await take(box.desktop()))['url']
    elif kind=='browser':
        assert (await take(box.browser.screenshot('http://127.0.0.1:3000'))).startswith(PNG)
        assert (await take(box.browser()))['url']
    elif kind=='mount':
        mounts=await take(box.mounts());assert mounts[0]['state']=='mounted';assert 'credentials' not in mounts[0]
        result=await take(box.run(['sh','-c','cat /mnt/data/seed && printf "%s" "$1" > "/mnt/data/python-$1-$2"','sh',mode,box.id]))
        assert result.code==0 and result.stdout=='seed'
    elif kind=='ssh':
        access=await take(box.ssh());assert access.key.startswith('-----BEGIN OPENSSH PRIVATE KEY-----');assert access.host and access.port;assert 'PRIVATE KEY' not in repr(access)
        await take(box.ssh.revoke())
    else:raise AssertionError('Unknown tool case')
with Pryx(url=os.environ['PRYX_API_URL'],key=os.environ['PRYX_API_KEY']) as client:asyncio.run(exercise(client,'sync'))
async def main():
    async with Pryx(url=os.environ['PRYX_API_URL'],key=os.environ['PRYX_API_KEY']) as client:await exercise(client,'async')
asyncio.run(main())
