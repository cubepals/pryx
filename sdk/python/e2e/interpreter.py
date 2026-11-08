# SPDX-License-Identifier: FSL-1.1-ALv2
"""The same warmed interpreter is exercised by synchronous and asynchronous SDK contexts."""
import asyncio, inspect, os
from cubepals_pryx import Pryx

async def take(value): return await value if inspect.isawaitable(value) else value
async def exercise(client, name):
    box=await take(client.sandbox(os.environ['PRYX_KERNEL_SANDBOX']))
    python=os.environ['PRYX_KERNEL_LANGUAGE']=='python'
    kernel=await take(box.python(context=name) if python else box.javascript(context=name))
    try:
        await take(kernel.run('x=40' if python else 'var x=40'))
        assert (await take(kernel.run('x+2'))).text=='42'
        error=await take(kernel.run("raise ValueError('known')" if python else "throw new Error('known')"))
        assert error.error and error.error['traceback']
        if python:
            result=await take(kernel.run('plt.plot([1,2,3]); plt.show()'))
            assert result.png.startswith(b'\x89PNG\r\n\x1a\n') and '<svg' in result.svg
            result=await take(kernel.run('pd.DataFrame({"a":[1,2]})'))
            assert result.json==[{'a':1},{'a':2}] and '<table' in result.html
        else:
            result=await take(kernel.run("$$.mime({'image/png':require('./chart.cjs')().toString('base64')})"))
            assert result.png.startswith(b'\x89PNG\r\n\x1a\n')
        async def event_loop():
            stream=kernel.stream("print('interrupt-ready',flush=True)\nwhile True: pass" if python else "console.log('interrupt-ready'); while(true){}")
            interrupted=False
            async def consume(event):
                nonlocal interrupted
                if event['type']=='stream' and 'interrupt-ready' in event['text'] and not interrupted:
                    interrupted=True;await take(kernel.interrupt())
            if hasattr(stream,'__aiter__'):
                async for event in stream:await consume(event)
            else:
                for event in stream:await consume(event)
            assert interrupted
        await event_loop()
        assert (await take(kernel.run('x+2'))).text=='42'
        assert (await take(kernel.run('1+1'))).text=='2'
        await take(kernel.restart());assert (await take(kernel.run('x'))).error
    finally:await take(kernel.close())

with Pryx(url=os.environ['PRYX_API_URL'],key=os.environ['PRYX_API_KEY']) as client:asyncio.run(exercise(client,'python-sync'))
async def main():
    async with Pryx(url=os.environ['PRYX_API_URL'],key=os.environ['PRYX_API_KEY']) as client:await exercise(client,'python-async')
asyncio.run(main())
